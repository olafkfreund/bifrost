//! GitLab CI source adapter (#100).
//!
//! The third [`SourceAdapter`] implementation (after Azure DevOps and Jenkins),
//! discovering a GitLab estate over the v4 REST API so it gets the same portfolio
//! heatmap. The official Importer owns the actual GitLab CI → GitHub Actions
//! conversion; this adapter owns discovery/ingestion.
//!
//! Mapping to the domain model: a GitLab **group** (namespace) is the pipeline
//! grouping → [`Project`]; a GitLab **project** (repository) is one pipeline →
//! [`SourcePipeline`]. A project carrying a `.gitlab-ci.yml` is pipeline-as-code →
//! [`Classification::Yaml`]; an **Auto DevOps** project (no committed config, a
//! GitLab-generated pipeline) is the hard tail with no source YAML to read →
//! [`Classification::Classic`]. CI/CD variables map to variable groups, recorded
//! by name + secret flag only — never their values (hard rule).

use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use bifrost_llm::{retry, RetryPolicy};

use crate::source::{classify_adapter_error, AdapterError, SourceAdapter, HTTP_TIMEOUT};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Parse the GitLab `/groups` list into domain [`Project`]s. The group's
/// `full_path` is its stable id (groups can be nested, e.g. `org/team`).
pub fn parse_groups(v: &Value) -> Vec<Project> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| {
                    let full_path = g
                        .get("full_path")
                        .and_then(Value::as_str)
                        .or_else(|| g.get("name").and_then(Value::as_str))?;
                    Some(Project {
                        id: full_path.to_string(),
                        name: g
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(full_path)
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a GitLab project relies on Auto DevOps (a generated pipeline with no
/// committed `.gitlab-ci.yml`) — the hard tail, classified [`Classification::Classic`].
fn is_auto_devops(project: &Value) -> bool {
    let auto = project
        .get("auto_devops_enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_config = project
        .get("ci_config_path")
        .and_then(Value::as_str)
        .map(|p| !p.is_empty())
        .unwrap_or(false);
    auto && !has_config
}

/// Parse the GitLab `/projects` list into [`SourcePipeline`]s — one per project.
/// `path_with_namespace` (e.g. `storefront/web-portal`) is the stable id; the
/// project's namespace is its grouping [`Project`].
pub fn parse_pipelines(v: &Value) -> Vec<SourcePipeline> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let path = p.get("path_with_namespace").and_then(Value::as_str)?;
                    let group = p
                        .get("namespace")
                        .and_then(|n| n.get("full_path").and_then(Value::as_str))
                        .unwrap_or("root");
                    Some(SourcePipeline {
                        id: path.to_string(),
                        name: p
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(path)
                            .to_string(),
                        project: group.to_string(),
                        classification: if is_auto_devops(p) {
                            Classification::Classic
                        } else {
                            Classification::Yaml
                        },
                        repository: Some(path.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a GitLab CI/CD `/variables` list into a [`VariableGroup`]. A variable is
/// secret-flagged when GitLab marks it `masked` or `protected`. The API returns a
/// `value` field; we deliberately read only `key` so no secret material is kept.
pub fn parse_variables(v: &Value, project: &str) -> VariableGroup {
    let variables = v
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|var| {
                    let name = var.get("key").and_then(Value::as_str)?;
                    let masked = var.get("masked").and_then(Value::as_bool).unwrap_or(false);
                    let protected = var
                        .get("protected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some(VariableRef {
                        name: name.to_string(),
                        is_secret: masked || protected,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    VariableGroup {
        id: format!("{project}/ci-variables"),
        name: "CI/CD variables".to_string(),
        project: project.to_string(),
        variables,
    }
}

// ---- live adapter ----------------------------------------------------------

/// Read-only GitLab adapter over the v4 REST API. Auth is a personal/group access
/// token sent as the `PRIVATE-TOKEN` header.
pub struct GitLabAdapter {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl GitLabAdapter {
    /// `base_url` is the GitLab host (e.g. `https://gitlab.com`); the `/api/v4`
    /// prefix is added per request.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    /// Build from `GITLAB_URL` (defaulting to `https://gitlab.com`) + `GITLAB_TOKEN`.
    pub fn from_env() -> Result<Self, AdapterError> {
        let base = std::env::var("GITLAB_URL").unwrap_or_else(|_| "https://gitlab.com".into());
        let token = std::env::var("GITLAB_TOKEN")
            .map_err(|_| AdapterError::Auth("GITLAB_TOKEN not set".into()))?;
        Ok(Self::new(base, token))
    }

    /// GET JSON with bounded retries + backoff on transient failures (#106).
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_GITLAB"),
            classify_adapter_error,
            || self.get_attempt(path),
        )
        .await
    }

    async fn get_attempt(&self, path: &str) -> Result<Value, AdapterError> {
        let url = format!("{}/api/v4/{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("PRIVATE-TOKEN", &self.token)
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
                "GitLab returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("GitLab returned {s}"))),
        }
    }

    /// Fetch the raw text of a file in a project at a ref (used for `.gitlab-ci.yml`),
    /// with bounded retries + backoff on transient failures (#106).
    async fn get_raw(&self, path: &str) -> Result<String, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_GITLAB"),
            classify_adapter_error,
            || self.get_raw_attempt(path),
        )
        .await
    }

    async fn get_raw_attempt(&self, path: &str) -> Result<String, AdapterError> {
        let url = format!("{}/api/v4/{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .header("PRIVATE-TOKEN", &self.token)
            .timeout(HTTP_TIMEOUT)
            .send()
            .await
            .map_err(|e| AdapterError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            200 => resp
                .text()
                .await
                .map_err(|e| AdapterError::Transport(e.to_string())),
            401 | 403 => Err(AdapterError::Auth(format!(
                "GitLab returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("GitLab returned {s}"))),
        }
    }
}

#[async_trait::async_trait]
impl SourceAdapter for GitLabAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let v = self.get("groups?per_page=100&all_available=true").await?;
        Ok(parse_groups(&v))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let v = self.get("projects?per_page=100&membership=true").await?;
        Ok(parse_pipelines(&v))
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        let pipe = self
            .enumerate_pipelines()
            .await?
            .into_iter()
            .find(|p| p.id == pipeline_id)
            .ok_or_else(|| AdapterError::NotFound(pipeline_id.to_string()))?;
        // Auto DevOps pipelines have no committed source to read.
        let yaml = if pipe.classification == Classification::Yaml {
            let enc = urlencode(pipeline_id);
            self.get_raw(&format!(
                "projects/{enc}/repository/files/.gitlab-ci.yml/raw?ref=HEAD"
            ))
            .await
            .ok()
        } else {
            None
        };
        Ok(PipelineDefinition {
            id: pipe.id,
            classification: pipe.classification,
            yaml,
        })
    }

    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError> {
        // GitLab has no Azure-DevOps-style service connections; credentials live in
        // CI/CD variables (see fetch_variable_groups). Project integrations are a v2
        // nicety. Returning empty keeps the model honest rather than inventing rows.
        Ok(Vec::new())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let projects = self.enumerate_pipelines().await?;
        let mut out = Vec::new();
        for p in projects {
            let enc = urlencode(&p.id);
            let v = self
                .get(&format!("projects/{enc}/variables?per_page=100"))
                .await
                .unwrap_or(Value::Null);
            let group = parse_variables(&v, &p.id);
            if !group.variables.is_empty() {
                out.push(group);
            }
        }
        Ok(out)
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // GitLab CI has no marketplace-task concept; step/include inventory is a v2
        // nicety the Importer's audit reports.
        Ok(Vec::new())
    }
}

/// Minimal percent-encoding for a GitLab path id (`group/project` → `group%2Fproject`).
fn urlencode(s: &str) -> String {
    s.replace('%', "%25").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTS: &str = include_str!("../../../fixtures/gitlab/projects.json");
    const GROUPS: &str = include_str!("../../../fixtures/gitlab/groups.json");
    const VARIABLES: &str = include_str!("../../../fixtures/gitlab/variables.json");

    #[test]
    fn groups_become_projects() {
        let v: Value = serde_json::from_str(GROUPS).unwrap();
        let names: Vec<_> = parse_groups(&v).into_iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec!["storefront".to_string(), "platform".to_string()]
        );
    }

    #[test]
    fn projects_classify_auto_devops_as_classic_and_committed_as_yaml() {
        let v: Value = serde_json::from_str(PROJECTS).unwrap();
        let pipelines = parse_pipelines(&v);
        assert_eq!(pipelines.len(), 3);

        let web = pipelines.iter().find(|p| p.name == "web-portal").unwrap();
        assert_eq!(web.classification, Classification::Yaml);
        assert_eq!(web.project, "storefront");
        assert_eq!(web.id, "storefront/web-portal");

        // Auto DevOps (generated pipeline, no committed config) is the hard tail.
        let payments = pipelines.iter().find(|p| p.name == "payments-svc").unwrap();
        assert_eq!(payments.classification, Classification::Classic);

        // Explicit ci_config_path is pipeline-as-code.
        let infra = pipelines
            .iter()
            .find(|p| p.name == "infra-terraform")
            .unwrap();
        assert_eq!(infra.classification, Classification::Yaml);
        assert_eq!(infra.project, "platform");
    }

    #[test]
    fn variables_record_names_and_secret_flag_only() {
        let v: Value = serde_json::from_str(VARIABLES).unwrap();
        let group = parse_variables(&v, "storefront/web-portal");
        assert_eq!(group.variables.len(), 2);

        let public = group.variables.iter().find(|x| !x.is_secret).unwrap();
        assert_eq!(public.name, "DOCKER_REGISTRY_URL");

        // A masked+protected variable is secret-flagged by name only.
        let secret = group.variables.iter().find(|x| x.is_secret).unwrap();
        assert_eq!(secret.name, "AZURE_CLIENT_SECRET");

        // The secret VALUE from the fixture must never survive parsing.
        let serialized = serde_json::to_string(&group).unwrap();
        assert!(!serialized.contains("super-secret-value-must-never-be-stored"));
    }

    #[test]
    fn path_ids_are_percent_encoded_for_the_api() {
        assert_eq!(
            urlencode("storefront/web-portal"),
            "storefront%2Fweb-portal"
        );
    }

    /// Live: enumerate a real GitLab. Ignored by default (needs GITLAB_* env).
    #[tokio::test]
    #[ignore = "requires a live GitLab (GITLAB_URL / GITLAB_TOKEN)"]
    async fn live_enumerate() {
        let adapter = GitLabAdapter::from_env().expect("GITLAB_* set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
