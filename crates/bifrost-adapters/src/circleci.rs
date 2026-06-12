//! CircleCI source adapter (#194).
//!
//! The fifth [`SourceAdapter`] implementation, discovering a CircleCI estate over
//! the CircleCI API (v1.1 for the followed-project list, v2 for env vars). The
//! official Importer owns the CircleCI → GitHub Actions conversion; this adapter
//! owns discovery/ingestion.
//!
//! Mapping to the domain model: a VCS **org** (`username`) is the pipeline
//! grouping → [`Project`]; a followed **project** (repo) is one pipeline →
//! [`SourcePipeline`]. CircleCI is pipeline-as-code (`.circleci/config.yml`) with
//! no designer concept, so every project is [`Classification::Yaml`]. Project
//! environment variables map to a variable group, recorded by name only — CircleCI
//! masks values, and we never read them (hard rule).

use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use bifrost_llm::{retry, RetryPolicy};

use crate::source::{classify_adapter_error, AdapterError, SourceAdapter, HTTP_TIMEOUT};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Distinct VCS orgs (`username`) across the followed-project list.
pub fn parse_projects(projects: &Value) -> Vec<Project> {
    let mut orgs: Vec<String> = projects
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    p.get("username")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    orgs.sort();
    orgs.dedup();
    orgs.into_iter()
        .map(|o| Project {
            id: o.clone(),
            name: o,
        })
        .collect()
}

/// Parse the CircleCI v1.1 projects list into [`SourcePipeline`]s — one per repo.
/// `username/reponame` is the stable id; the org is its grouping.
pub fn parse_pipelines(projects: &Value) -> Vec<SourcePipeline> {
    projects
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let org = p.get("username").and_then(Value::as_str)?;
                    let repo = p.get("reponame").and_then(Value::as_str)?;
                    Some(SourcePipeline {
                        id: format!("{org}/{repo}"),
                        name: repo.to_string(),
                        project: org.to_string(),
                        // CircleCI has no designer pipelines — always YAML.
                        classification: Classification::Yaml,
                        repository: p.get("vcs_url").and_then(Value::as_str).map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a CircleCI v2 env-var list into a [`VariableGroup`]. CircleCI masks
/// values (the API returns only the last few characters), so every variable is
/// secret-flagged and recorded by name only — we read `name`, never `value`.
pub fn parse_envvars(envvars: &Value, project: &str) -> VariableGroup {
    let variables = envvars
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str)?;
                    Some(VariableRef {
                        name: name.to_string(),
                        is_secret: true,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    VariableGroup {
        id: format!("{project}/env"),
        name: "Environment variables".to_string(),
        project: project.to_string(),
        variables,
    }
}

/// CircleCI's project slug for the v2 API: `{vcs}/{org}/{repo}` where `vcs` is
/// `gh` (GitHub) or `bb` (Bitbucket), inferred from the project's `vcs_url`.
fn project_slug(org: &str, repo: &str, vcs_url: &str) -> String {
    let vcs = if vcs_url.contains("bitbucket") {
        "bb"
    } else {
        "gh"
    };
    format!("{vcs}/{org}/{repo}")
}

// ---- live adapter ----------------------------------------------------------

/// Read-only CircleCI adapter. Auth is a personal API token (`Circle-Token`).
pub struct CircleCiAdapter {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl CircleCiAdapter {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://circleci.com".to_string(),
            token: token.into(),
        }
    }

    /// Build from `CIRCLECI_TOKEN`.
    pub fn from_env() -> Result<Self, AdapterError> {
        let token = std::env::var("CIRCLECI_TOKEN")
            .map_err(|_| AdapterError::Auth("CIRCLECI_TOKEN not set".into()))?;
        Ok(Self::new(token))
    }

    /// GET a path (relative to the host) with bounded retries + backoff (#106).
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_CIRCLECI"),
            classify_adapter_error,
            || self.attempt(path),
        )
        .await
    }

    async fn attempt(&self, path: &str) -> Result<Value, AdapterError> {
        let url = format!("{}/{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("Circle-Token", &self.token)
            .header("Accept", "application/json")
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|e| AdapterError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            200 => resp
                .json()
                .await
                .map_err(|e| AdapterError::Transport(e.to_string())),
            401 | 403 => Err(AdapterError::Auth(format!(
                "CircleCI returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("CircleCI returned {s}"))),
        }
    }
}

#[async_trait::async_trait]
impl SourceAdapter for CircleCiAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let projects = self.get("api/v1.1/projects").await?;
        Ok(parse_projects(&projects))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let projects = self.get("api/v1.1/projects").await?;
        Ok(parse_pipelines(&projects))
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        // The `.circleci/config.yml` lives in SCM; the Importer fetches it at
        // dry-run. Here we only confirm the pipeline exists + its classification.
        let classification = self
            .enumerate_pipelines()
            .await?
            .into_iter()
            .find(|p| p.id == pipeline_id)
            .map(|p| p.classification)
            .ok_or_else(|| AdapterError::NotFound(pipeline_id.to_string()))?;
        Ok(PipelineDefinition {
            id: pipeline_id.to_string(),
            classification,
            yaml: None,
        })
    }

    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError> {
        // CircleCI has no Azure-DevOps-style service connections; credentials live
        // in project env vars / contexts. Empty for v2.
        Ok(Vec::new())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let projects = self.get("api/v1.1/projects").await?;
        let mut out = Vec::new();
        if let Some(arr) = projects.as_array() {
            for p in arr {
                let (Some(org), Some(repo)) = (
                    p.get("username").and_then(Value::as_str),
                    p.get("reponame").and_then(Value::as_str),
                ) else {
                    continue;
                };
                let vcs_url = p.get("vcs_url").and_then(Value::as_str).unwrap_or("");
                let slug = project_slug(org, repo, vcs_url);
                let envvars = self
                    .get(&format!("api/v2/project/{slug}/envvar"))
                    .await
                    .unwrap_or(Value::Null);
                let group = parse_envvars(&envvars, &format!("{org}/{repo}"));
                if !group.variables.is_empty() {
                    out.push(group);
                }
            }
        }
        Ok(out)
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // CircleCI orbs/commands are a v2 nicety; the Importer's audit reports them.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTS: &str = include_str!("../../../fixtures/circleci/projects.json");
    const ENVVARS: &str = include_str!("../../../fixtures/circleci/envvars.json");

    #[test]
    fn projects_are_the_distinct_vcs_orgs() {
        let v: Value = serde_json::from_str(PROJECTS).unwrap();
        let names: Vec<_> = parse_projects(&v).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["acme".to_string(), "platform".to_string()]);
    }

    #[test]
    fn every_project_is_a_yaml_pipeline() {
        let v: Value = serde_json::from_str(PROJECTS).unwrap();
        let pipelines = parse_pipelines(&v);
        assert_eq!(pipelines.len(), 3);
        assert!(pipelines
            .iter()
            .all(|p| p.classification == Classification::Yaml));

        let web = pipelines.iter().find(|p| p.name == "web").unwrap();
        assert_eq!(web.id, "acme/web");
        assert_eq!(web.project, "acme");

        let infra = pipelines.iter().find(|p| p.name == "infra").unwrap();
        assert_eq!(infra.project, "platform");
    }

    #[test]
    fn envvars_record_names_only_and_never_masked_values() {
        let v: Value = serde_json::from_str(ENVVARS).unwrap();
        let group = parse_envvars(&v, "acme/web");
        assert_eq!(group.variables.len(), 2);
        // CircleCI env vars are always secret-flagged.
        assert!(group.variables.iter().all(|x| x.is_secret));
        assert!(group.variables.iter().any(|x| x.name == "DEPLOY_TOKEN"));
        // The masked value from the fixture must never survive parsing.
        let serialized = serde_json::to_string(&group).unwrap();
        assert!(!serialized.contains("xxxx"));
    }

    #[test]
    fn project_slug_picks_the_vcs_prefix() {
        assert_eq!(
            project_slug("acme", "web", "https://github.com/acme/web"),
            "gh/acme/web"
        );
        assert_eq!(
            project_slug("platform", "infra", "https://bitbucket.org/platform/infra"),
            "bb/platform/infra"
        );
    }

    /// Live: enumerate a real CircleCI account. Ignored by default.
    #[tokio::test]
    #[ignore = "requires a live CircleCI (CIRCLECI_TOKEN)"]
    async fn live_enumerate() {
        let adapter = CircleCiAdapter::from_env().expect("CIRCLECI_TOKEN set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
