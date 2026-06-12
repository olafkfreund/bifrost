//! The `SourceAdapter` trait and an in-memory mock implementation.
//!
//! A source adapter discovers and reads a CI source platform (Azure DevOps
//! first). It is read-only and platform-agnostic; the orchestrator depends on
//! the trait, never on a concrete platform client.

use async_trait::async_trait;
use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskKind,
    TaskUsage, VariableGroup, VariableRef,
};

/// Errors a source adapter can surface. Concrete adapters map their transport
/// and auth failures onto these variants.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("transport error: {0}")]
    Transport(String),
}

/// Per-request HTTP timeout for adapter REST calls (#106). Long enough for a slow
/// ADO/GitLab/Jenkins page, short enough that a hung connection becomes a retry.
pub(crate) const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Classify an [`AdapterError`] for retry (#106): transport failures (timeouts,
/// resets, 5xx/429) are transient; auth and not-found are permanent. Shared by
/// every HTTP adapter (ADO, Jenkins, GitLab).
pub(crate) fn classify_adapter_error(e: &AdapterError) -> bifrost_llm::ErrorClass {
    use bifrost_llm::ErrorClass;
    match e {
        AdapterError::Transport(_) => ErrorClass::Retryable,
        AdapterError::Auth(_) | AdapterError::NotFound(_) => ErrorClass::Permanent,
    }
}

/// Read-only access to a CI source platform.
///
/// ADO is the first implementation; keep this platform-agnostic so Jenkins,
/// GitLab, etc. can follow. Secret *values* are never returned — only the
/// names/types needed to drive migration risk.
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    /// List the projects / team-projects in the org.
    async fn discover(&self) -> Result<Vec<Project>, AdapterError>;

    /// Enumerate every pipeline, tagged classic vs YAML.
    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError>;

    /// Fetch a single pipeline's definition (YAML pipelines carry source;
    /// classic/designer pipelines do not).
    async fn fetch_definition(&self, pipeline_id: &str)
        -> Result<PipelineDefinition, AdapterError>;

    /// Enumerate service connections (names + types only) — drives OIDC risk.
    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError>;

    /// Enumerate variable groups and variable *names* (secret values excluded).
    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError>;

    /// Aggregate task/extension usage across the org.
    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError>;
}

/// An in-memory [`SourceAdapter`] with a small canned org. Used by tests and to
/// exercise the orchestrator/API without a live Azure DevOps connection.
#[derive(Debug, Clone)]
pub struct MockSourceAdapter {
    pub projects: Vec<Project>,
    pub pipelines: Vec<SourcePipeline>,
    pub service_connections: Vec<ServiceConnection>,
    pub variable_groups: Vec<VariableGroup>,
    pub tasks: Vec<TaskUsage>,
}

impl Default for MockSourceAdapter {
    fn default() -> Self {
        let project = |id: &str, name: &str| Project {
            id: id.into(),
            name: name.into(),
        };
        let pipe = |id: &str, name: &str, project: &str, c: Classification| SourcePipeline {
            id: id.into(),
            name: name.into(),
            project: project.into(),
            classification: c,
            repository: Some(format!("{project}/{name}")),
        };
        Self {
            projects: vec![project("p1", "Storefront"), project("p2", "Payments")],
            pipelines: vec![
                pipe(
                    "web-portal-ci",
                    "web-portal",
                    "Storefront",
                    Classification::Yaml,
                ),
                pipe(
                    "payments-deploy",
                    "payments-deploy",
                    "Payments",
                    Classification::Classic,
                ),
            ],
            service_connections: vec![ServiceConnection {
                id: "sc1".into(),
                name: "azure-prod".into(),
                kind: "azurerm".into(),
                project: "Payments".into(),
            }],
            variable_groups: vec![VariableGroup {
                id: "vg1".into(),
                name: "shared".into(),
                project: "Storefront".into(),
                variables: vec![
                    VariableRef {
                        name: "API_URL".into(),
                        is_secret: false,
                    },
                    VariableRef {
                        name: "API_TOKEN".into(),
                        is_secret: true,
                    },
                ],
            }],
            tasks: vec![
                TaskUsage {
                    task: "PublishBuildArtifacts@1".into(),
                    kind: TaskKind::BuiltIn,
                    count: 5,
                },
                TaskUsage {
                    task: "SonarQubePrepare@5".into(),
                    kind: TaskKind::Marketplace,
                    count: 2,
                },
            ],
        }
    }
}

impl MockSourceAdapter {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SourceAdapter for MockSourceAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        Ok(self.projects.clone())
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        Ok(self.pipelines.clone())
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        let p = self
            .pipelines
            .iter()
            .find(|p| p.id == pipeline_id)
            .ok_or_else(|| AdapterError::NotFound(pipeline_id.to_string()))?;
        let yaml = match p.classification {
            Classification::Yaml => Some(format!("# {}\nsteps: []\n", p.name)),
            Classification::Classic => None,
        };
        Ok(PipelineDefinition {
            id: p.id.clone(),
            classification: p.classification,
            yaml,
        })
    }

    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError> {
        Ok(self.service_connections.clone())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        Ok(self.variable_groups.clone())
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        Ok(self.tasks.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_adapter_enumerates_and_classifies() {
        let a = MockSourceAdapter::new();
        assert_eq!(a.discover().await.unwrap().len(), 2);

        let pipelines = a.enumerate_pipelines().await.unwrap();
        assert_eq!(pipelines.len(), 2);
        assert_eq!(
            pipelines
                .iter()
                .filter(|p| p.classification == Classification::Classic)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn classic_pipelines_have_no_yaml_source() {
        let a = MockSourceAdapter::new();
        let yaml = a.fetch_definition("web-portal-ci").await.unwrap();
        assert!(yaml.yaml.is_some());
        let classic = a.fetch_definition("payments-deploy").await.unwrap();
        assert!(classic.yaml.is_none());
    }

    #[tokio::test]
    async fn unknown_pipeline_is_not_found() {
        let a = MockSourceAdapter::new();
        let err = a.fetch_definition("does-not-exist").await.unwrap_err();
        assert!(matches!(err, AdapterError::NotFound(_)));
    }

    #[tokio::test]
    async fn secret_variables_are_flagged_but_present_by_name_only() {
        let a = MockSourceAdapter::new();
        let groups = a.fetch_variable_groups().await.unwrap();
        let secret = groups[0].variables.iter().find(|v| v.is_secret).unwrap();
        assert_eq!(secret.name, "API_TOKEN");
    }
}
