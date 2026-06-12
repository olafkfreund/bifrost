//! Bamboo source adapter (#201) — the last Importer-supported source.
//!
//! The seventh [`SourceAdapter`] implementation, discovering an Atlassian Bamboo
//! estate over the REST API. The official Importer owns the Bamboo → GitHub
//! Actions conversion; this adapter owns discovery/ingestion.
//!
//! Mapping to the domain model: a Bamboo **project** (`key`) is the pipeline
//! grouping → [`Project`]; a **plan** is one pipeline → [`SourcePipeline`]. Bamboo
//! plans are predominantly designer-configured (the UI), with no committed YAML
//! source, so they default to [`Classification::Classic`] — the hard tail. Plan
//! variables map to a variable group, recorded by name + a `sensitive` flag only —
//! values are never read (hard rule).

use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use bifrost_llm::{retry, RetryPolicy};

use crate::source::{classify_adapter_error, AdapterError, SourceAdapter, HTTP_TIMEOUT};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Parse the Bamboo `/project` response (`{ projects: { project: [...] } }`).
pub fn parse_projects(v: &Value) -> Vec<Project> {
    v.get("projects")
        .and_then(|p| p.get("project"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let key = p.get("key").and_then(Value::as_str)?;
                    Some(Project {
                        id: key.to_string(),
                        name: p
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(key)
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the Bamboo `/plan` response (`{ plans: { plan: [...] } }`) into
/// [`SourcePipeline`]s. The plan `key` (e.g. `PLAT-WEB`) is the stable id; the
/// `projectKey` is its grouping. Designer plans → [`Classification::Classic`].
pub fn parse_plans(v: &Value) -> Vec<SourcePipeline> {
    v.get("plans")
        .and_then(|p| p.get("plan"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let key = p.get("key").and_then(Value::as_str)?;
                    let project = p
                        .get("projectKey")
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| key.split('-').next().unwrap_or(key));
                    Some(SourcePipeline {
                        id: key.to_string(),
                        name: p
                            .get("shortName")
                            .or_else(|| p.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or(key)
                            .to_string(),
                        project: project.to_string(),
                        // Bamboo plans are designer-configured (no committed YAML).
                        classification: Classification::Classic,
                        repository: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a Bamboo plan-variables response (`{ results: [...] }`) into a
/// [`VariableGroup`]. A variable is secret-flagged when Bamboo marks it
/// `sensitive`; we read only `key`, never `value`.
pub fn parse_variables(v: &Value, project: &str) -> VariableGroup {
    let variables = v
        .get("results")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|var| {
                    let name = var.get("key").and_then(Value::as_str)?;
                    let sensitive = var
                        .get("sensitive")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some(VariableRef {
                        name: name.to_string(),
                        is_secret: sensitive,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    VariableGroup {
        id: format!("{project}/plan-variables"),
        name: "Plan variables".to_string(),
        project: project.to_string(),
        variables,
    }
}

// ---- live adapter ----------------------------------------------------------

/// Read-only Bamboo adapter over the REST API. Auth is a personal access token
/// (`Authorization: Bearer <token>`).
pub struct BambooAdapter {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl BambooAdapter {
    /// `base_url` is the Bamboo host (e.g. `https://bamboo.example.com`); the
    /// `/rest/api/latest` prefix is added per request.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    /// Build from `BAMBOO_URL` + `BAMBOO_TOKEN`.
    pub fn from_env() -> Result<Self, AdapterError> {
        let base = std::env::var("BAMBOO_URL")
            .map_err(|_| AdapterError::Auth("BAMBOO_URL not set".into()))?;
        let token = std::env::var("BAMBOO_TOKEN")
            .map_err(|_| AdapterError::Auth("BAMBOO_TOKEN not set".into()))?;
        Ok(Self::new(base, token))
    }

    /// GET a REST path (relative to `/rest/api/latest`) with retries + backoff (#106).
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_BAMBOO"),
            classify_adapter_error,
            || self.attempt(path),
        )
        .await
    }

    async fn attempt(&self, path: &str) -> Result<Value, AdapterError> {
        let url = format!("{}/rest/api/latest/{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
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
                "Bamboo returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("Bamboo returned {s}"))),
        }
    }
}

#[async_trait::async_trait]
impl SourceAdapter for BambooAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let v = self.get("project.json?max-results=1000").await?;
        Ok(parse_projects(&v))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let v = self.get("plan.json?max-results=1000").await?;
        Ok(parse_plans(&v))
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        // Designer plans have no committed YAML source; confirm existence + class.
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
        // Bamboo has no Azure-DevOps-style service connections; shared credentials
        // are a v2 nicety. Empty.
        Ok(Vec::new())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let plans = self.get("plan.json?max-results=1000").await?;
        let mut out = Vec::new();
        if let Some(arr) = plans
            .get("plans")
            .and_then(|p| p.get("plan"))
            .and_then(Value::as_array)
        {
            for p in arr {
                let Some(key) = p.get("key").and_then(Value::as_str) else {
                    continue;
                };
                let vars = self
                    .get(&format!("plan/{key}/variable.json"))
                    .await
                    .unwrap_or(Value::Null);
                let group = parse_variables(&vars, key);
                if !group.variables.is_empty() {
                    out.push(group);
                }
            }
        }
        Ok(out)
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // Bamboo task/plugin inventory is a v2 nicety; the Importer's audit reports it.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTS: &str = include_str!("../../../fixtures/bamboo/projects.json");
    const PLANS: &str = include_str!("../../../fixtures/bamboo/plans.json");
    const VARS: &str = include_str!("../../../fixtures/bamboo/variables.json");

    #[test]
    fn projects_are_parsed_from_the_nested_shape() {
        let v: Value = serde_json::from_str(PROJECTS).unwrap();
        let names: Vec<_> = parse_projects(&v).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["Platform".to_string(), "Data".to_string()]);
    }

    #[test]
    fn plans_are_classic_and_grouped_by_project_key() {
        let v: Value = serde_json::from_str(PLANS).unwrap();
        let plans = parse_plans(&v);
        assert_eq!(plans.len(), 3);
        // Designer plans are the hard tail.
        assert!(plans
            .iter()
            .all(|p| p.classification == Classification::Classic));

        let web = plans.iter().find(|p| p.id == "PLAT-WEB").unwrap();
        assert_eq!(web.name, "Web Build");
        assert_eq!(web.project, "PLAT");

        let etl = plans.iter().find(|p| p.id == "DATA-ETL").unwrap();
        assert_eq!(etl.project, "DATA");
    }

    #[test]
    fn variables_record_names_and_sensitive_flag_only() {
        let v: Value = serde_json::from_str(VARS).unwrap();
        let group = parse_variables(&v, "PLAT-WEB");
        assert_eq!(group.variables.len(), 2);

        let public = group.variables.iter().find(|x| !x.is_secret).unwrap();
        assert_eq!(public.name, "ARTIFACT_REPO");
        let secret = group.variables.iter().find(|x| x.is_secret).unwrap();
        assert_eq!(secret.name, "SIGNING_KEY");
        // The value in the fixture must never survive parsing.
        let serialized = serde_json::to_string(&group).unwrap();
        assert!(!serialized.contains("nexus.example.com"));
    }

    /// Live: enumerate a real Bamboo. Ignored by default.
    #[tokio::test]
    #[ignore = "requires a live Bamboo (BAMBOO_URL / BAMBOO_TOKEN)"]
    async fn live_enumerate() {
        let adapter = BambooAdapter::from_env().expect("BAMBOO_* set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
