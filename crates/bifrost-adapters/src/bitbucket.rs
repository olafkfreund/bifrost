//! Bitbucket Pipelines source adapter (#182).
//!
//! The fourth [`SourceAdapter`] implementation (after Azure DevOps, Jenkins, and
//! GitLab), discovering a Bitbucket Cloud estate over the v2 REST API. The
//! official Importer owns the Bitbucket Pipelines → GitHub Actions conversion;
//! this adapter owns discovery/ingestion.
//!
//! Mapping to the domain model: a Bitbucket **project** (workspace > project) is
//! the pipeline grouping → [`Project`]; a **repository** is one pipeline →
//! [`SourcePipeline`]. A repo with Pipelines enabled (a `bitbucket-pipelines.yml`)
//! is pipeline-as-code → [`Classification::Yaml`]; a repo with Pipelines disabled
//! is the hard tail with no source to convert → [`Classification::Classic`].
//! Repository variables map to a variable group, recorded by name + the `secured`
//! flag only — never their values (hard rule; Bitbucket omits secured values).

use std::collections::HashSet;

use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use bifrost_llm::{retry, RetryPolicy};

use crate::source::{classify_adapter_error, AdapterError, SourceAdapter, HTTP_TIMEOUT};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Distinct Bitbucket projects implied by the repositories list (workspace >
/// project > repo). Deriving projects from the repos keeps grouping referentially
/// consistent with [`parse_repos`].
pub fn parse_projects(repos: &Value) -> Vec<Project> {
    let mut seen: Vec<(String, String)> = Vec::new();
    if let Some(values) = repos.get("values").and_then(Value::as_array) {
        for repo in values {
            if let Some(project) = repo.get("project") {
                let key = project.get("key").and_then(Value::as_str);
                if let Some(key) = key {
                    let name = project.get("name").and_then(Value::as_str).unwrap_or(key);
                    if !seen.iter().any(|(k, _)| k == key) {
                        seen.push((key.to_string(), name.to_string()));
                    }
                }
            }
        }
    }
    seen.sort();
    seen.into_iter()
        .map(|(id, name)| Project { id, name })
        .collect()
}

/// Parse the Bitbucket `/repositories` list into [`SourcePipeline`]s — one per
/// repo. `enabled` is the set of repo `full_name`s with Pipelines enabled (the
/// live adapter builds it from each repo's `pipelines_config`); a repo in the set
/// is pipeline-as-code (Yaml), otherwise it is the hard tail (Classic).
pub fn parse_repos(repos: &Value, enabled: &HashSet<String>) -> Vec<SourcePipeline> {
    repos
        .get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|repo| {
                    let full_name = repo.get("full_name").and_then(Value::as_str)?;
                    let project = repo
                        .get("project")
                        .and_then(|p| p.get("key").and_then(Value::as_str))
                        .unwrap_or("UNGROUPED");
                    Some(SourcePipeline {
                        id: full_name.to_string(),
                        name: repo
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(full_name)
                            .to_string(),
                        project: project.to_string(),
                        classification: if enabled.contains(full_name) {
                            Classification::Yaml
                        } else {
                            Classification::Classic
                        },
                        repository: Some(full_name.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a Bitbucket repository-variables list into a [`VariableGroup`]. A variable
/// is secret-flagged when Bitbucket marks it `secured` (its value is then omitted
/// by the API); we read only `key`, so no secret material is kept.
pub fn parse_variables(vars: &Value, project: &str) -> VariableGroup {
    let variables = vars
        .get("values")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|var| {
                    let name = var.get("key").and_then(Value::as_str)?;
                    let secured = var.get("secured").and_then(Value::as_bool).unwrap_or(false);
                    Some(VariableRef {
                        name: name.to_string(),
                        is_secret: secured,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    VariableGroup {
        id: format!("{project}/pipeline-variables"),
        name: "Repository variables".to_string(),
        project: project.to_string(),
        variables,
    }
}

// ---- live adapter ----------------------------------------------------------

/// Read-only Bitbucket Cloud adapter over the v2 REST API. Auth is HTTP Basic with
/// a username + app password (Bitbucket's app-password scheme).
pub struct BitbucketAdapter {
    client: reqwest::Client,
    base_url: String,
    workspace: String,
    user: String,
    app_password: String,
}

impl BitbucketAdapter {
    pub fn new(
        workspace: impl Into<String>,
        user: impl Into<String>,
        app_password: impl Into<String>,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://api.bitbucket.org/2.0".to_string(),
            workspace: workspace.into(),
            user: user.into(),
            app_password: app_password.into(),
        }
    }

    /// Build from `BITBUCKET_WORKSPACE` + `BITBUCKET_USER` + `BITBUCKET_APP_PASSWORD`.
    pub fn from_env() -> Result<Self, AdapterError> {
        let workspace = std::env::var("BITBUCKET_WORKSPACE")
            .map_err(|_| AdapterError::Auth("BITBUCKET_WORKSPACE not set".into()))?;
        let user = std::env::var("BITBUCKET_USER")
            .map_err(|_| AdapterError::Auth("BITBUCKET_USER not set".into()))?;
        let app_password = std::env::var("BITBUCKET_APP_PASSWORD")
            .map_err(|_| AdapterError::Auth("BITBUCKET_APP_PASSWORD not set".into()))?;
        Ok(Self::new(workspace, user, app_password))
    }

    /// GET a v2 path (relative to `/2.0`) with bounded retries + backoff (#106).
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_BITBUCKET"),
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
            .basic_auth(&self.user, Some(&self.app_password))
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
                "Bitbucket returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("Bitbucket returned {s}"))),
        }
    }

    /// The set of repo `full_name`s with Pipelines enabled, by probing each repo's
    /// `pipelines_config` (404/disabled → not enabled).
    async fn enabled_repos(&self, repos: &Value) -> HashSet<String> {
        let mut enabled = HashSet::new();
        if let Some(values) = repos.get("values").and_then(Value::as_array) {
            for repo in values {
                if let Some(full_name) = repo.get("full_name").and_then(Value::as_str) {
                    let cfg = self
                        .get(&format!("repositories/{full_name}/pipelines_config"))
                        .await;
                    if let Ok(cfg) = cfg {
                        if cfg.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
                            enabled.insert(full_name.to_string());
                        }
                    }
                }
            }
        }
        enabled
    }
}

#[async_trait::async_trait]
impl SourceAdapter for BitbucketAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let repos = self
            .get(&format!("repositories/{}?pagelen=100", self.workspace))
            .await?;
        Ok(parse_projects(&repos))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let repos = self
            .get(&format!("repositories/{}?pagelen=100", self.workspace))
            .await?;
        let enabled = self.enabled_repos(&repos).await;
        Ok(parse_repos(&repos, &enabled))
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
        // Disabled repos have no committed pipeline source to read.
        let yaml = if pipe.classification == Classification::Yaml {
            // Resolve the main branch, then fetch bitbucket-pipelines.yml.
            let repo = self.get(&format!("repositories/{pipeline_id}")).await.ok();
            let branch = repo
                .as_ref()
                .and_then(|r| r.get("mainbranch"))
                .and_then(|b| b.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("HEAD");
            self.attempt_raw(&format!(
                "repositories/{pipeline_id}/src/{branch}/bitbucket-pipelines.yml"
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
        // Bitbucket has no Azure-DevOps-style service connections; credentials live
        // in repository variables (see fetch_variable_groups). Empty for v2.
        Ok(Vec::new())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let repos = self
            .get(&format!("repositories/{}?pagelen=100", self.workspace))
            .await?;
        let mut out = Vec::new();
        if let Some(values) = repos.get("values").and_then(Value::as_array) {
            for repo in values {
                if let Some(full_name) = repo.get("full_name").and_then(Value::as_str) {
                    let vars = self
                        .get(&format!(
                            "repositories/{full_name}/pipelines_config/variables/?pagelen=100"
                        ))
                        .await
                        .unwrap_or(Value::Null);
                    let group = parse_variables(&vars, full_name);
                    if !group.variables.is_empty() {
                        out.push(group);
                    }
                }
            }
        }
        Ok(out)
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // Bitbucket Pipelines has no marketplace-task concept; the Importer's audit
        // reports step/pipe usage. v2 nicety.
        Ok(Vec::new())
    }
}

impl BitbucketAdapter {
    /// Fetch a raw file body (for bitbucket-pipelines.yml).
    async fn attempt_raw(&self, path: &str) -> Result<String, AdapterError> {
        let url = format!("{}/{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.user, Some(&self.app_password))
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
                "Bitbucket returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("Bitbucket returned {s}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOS: &str = include_str!("../../../fixtures/bitbucket/repositories.json");
    const VARS: &str = include_str!("../../../fixtures/bitbucket/variables.json");

    fn enabled() -> HashSet<String> {
        // web-app + data-jobs have Pipelines enabled; legacy-svc does not.
        ["acme/web-app".to_string(), "acme/data-jobs".to_string()]
            .into_iter()
            .collect()
    }

    #[test]
    fn projects_are_the_distinct_workspace_projects() {
        let v: Value = serde_json::from_str(REPOS).unwrap();
        let names: Vec<_> = parse_projects(&v).into_iter().map(|p| p.name).collect();
        // Sorted by key: DATA ("Data") before PLAT ("Platform").
        assert_eq!(names, vec!["Data".to_string(), "Platform".to_string()]);
    }

    #[test]
    fn repos_classify_enabled_as_yaml_and_disabled_as_classic() {
        let v: Value = serde_json::from_str(REPOS).unwrap();
        let pipelines = parse_repos(&v, &enabled());
        assert_eq!(pipelines.len(), 3);

        let web = pipelines.iter().find(|p| p.name == "web-app").unwrap();
        assert_eq!(web.classification, Classification::Yaml);
        assert_eq!(web.project, "PLAT");
        assert_eq!(web.id, "acme/web-app");

        // Pipelines disabled → the hard tail.
        let legacy = pipelines.iter().find(|p| p.name == "legacy-svc").unwrap();
        assert_eq!(legacy.classification, Classification::Classic);

        let data = pipelines.iter().find(|p| p.name == "data-jobs").unwrap();
        assert_eq!(data.classification, Classification::Yaml);
        assert_eq!(data.project, "DATA");
    }

    #[test]
    fn variables_record_names_and_secured_flag_only() {
        let v: Value = serde_json::from_str(VARS).unwrap();
        let group = parse_variables(&v, "acme/web-app");
        assert_eq!(group.variables.len(), 2);

        let public = group.variables.iter().find(|x| !x.is_secret).unwrap();
        assert_eq!(public.name, "REGISTRY_URL");
        // A secured variable is secret-flagged by name only.
        let secret = group.variables.iter().find(|x| x.is_secret).unwrap();
        assert_eq!(secret.name, "DEPLOY_KEY");
        // The model carries no field that could hold the secured value.
        let serialized = serde_json::to_string(&group).unwrap();
        assert!(!serialized.contains("value"));
    }

    /// Live: enumerate a real Bitbucket workspace. Ignored by default.
    #[tokio::test]
    #[ignore = "requires a live Bitbucket (BITBUCKET_WORKSPACE / BITBUCKET_USER / BITBUCKET_APP_PASSWORD)"]
    async fn live_enumerate() {
        let adapter = BitbucketAdapter::from_env().expect("BITBUCKET_* set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
