//! Travis CI source adapter (#199).
//!
//! The sixth [`SourceAdapter`] implementation, discovering a Travis CI estate over
//! the v3 API. The official Importer owns the Travis → GitHub Actions conversion;
//! this adapter owns discovery/ingestion.
//!
//! Mapping to the domain model: a repo **owner** (`login`) is the pipeline grouping
//! → [`Project`]; a **repository** is one pipeline → [`SourcePipeline`]. Travis is
//! pipeline-as-code (`.travis.yml`) with no designer concept, so every repo is
//! [`Classification::Yaml`]. Repo environment variables map to a variable group,
//! recorded by name + the `public` flag (`is_secret = !public`) only — public
//! values are never read, secret values are not exposed by the API (hard rule).

use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use bifrost_llm::{retry, RetryPolicy};

use crate::source::{classify_adapter_error, AdapterError, SourceAdapter, HTTP_TIMEOUT};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Distinct repo owners (`owner.login`) across the repositories list.
pub fn parse_owners(repos: &Value) -> Vec<Project> {
    let mut owners: Vec<String> = repos
        .get("repositories")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    r.get("owner")
                        .and_then(|o| o.get("login"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    owners.sort();
    owners.dedup();
    owners
        .into_iter()
        .map(|o| Project {
            id: o.clone(),
            name: o,
        })
        .collect()
}

/// Parse the Travis `repositories` list into [`SourcePipeline`]s — one per repo.
/// The `slug` (`owner/repo`) is the stable id; the owner login is its grouping.
pub fn parse_repos(repos: &Value) -> Vec<SourcePipeline> {
    repos
        .get("repositories")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let slug = r.get("slug").and_then(Value::as_str)?;
                    let owner = r
                        .get("owner")
                        .and_then(|o| o.get("login"))
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| slug.split('/').next().unwrap_or(slug));
                    Some(SourcePipeline {
                        id: slug.to_string(),
                        name: r
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or(slug)
                            .to_string(),
                        project: owner.to_string(),
                        // Travis has no designer pipelines — always YAML.
                        classification: Classification::Yaml,
                        repository: Some(slug.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse a Travis `env_vars` list into a [`VariableGroup`]. A variable is
/// secret-flagged when it is not `public` (Travis omits secret values from the
/// API); we read only `name`, so no secret material is kept.
pub fn parse_env_vars(env_vars: &Value, project: &str) -> VariableGroup {
    let variables = env_vars
        .get("env_vars")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    let name = v.get("name").and_then(Value::as_str)?;
                    let public = v.get("public").and_then(Value::as_bool).unwrap_or(false);
                    Some(VariableRef {
                        name: name.to_string(),
                        is_secret: !public,
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

/// URL-encode a Travis repo slug for the v3 API (`owner/repo` → `owner%2Frepo`).
fn encode_slug(slug: &str) -> String {
    slug.replace('%', "%25").replace('/', "%2F")
}

// ---- live adapter ----------------------------------------------------------

/// Read-only Travis CI adapter over the v3 API. Auth is an API token
/// (`Authorization: token <token>`); the `Travis-API-Version: 3` header is required.
pub struct TravisAdapter {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl TravisAdapter {
    /// `base_url` defaults to `https://api.travis-ci.com` via [`Self::from_env`];
    /// use `https://api.travis-ci.org` for legacy / enterprise hosts.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    /// Build from `TRAVIS_TOKEN` (+ optional `TRAVIS_URL`, default `api.travis-ci.com`).
    pub fn from_env() -> Result<Self, AdapterError> {
        let base =
            std::env::var("TRAVIS_URL").unwrap_or_else(|_| "https://api.travis-ci.com".into());
        let token = std::env::var("TRAVIS_TOKEN")
            .map_err(|_| AdapterError::Auth("TRAVIS_TOKEN not set".into()))?;
        Ok(Self::new(base, token))
    }

    /// GET a v3 path with bounded retries + backoff (#106).
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        retry(
            RetryPolicy::from_env("BIFROST_TRAVIS"),
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
            .header("Authorization", format!("token {}", self.token))
            .header("Travis-API-Version", "3")
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
                "Travis returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("Travis returned {s}"))),
        }
    }
}

#[async_trait::async_trait]
impl SourceAdapter for TravisAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let repos = self.get("repos?limit=100").await?;
        Ok(parse_owners(&repos))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let repos = self.get("repos?limit=100").await?;
        Ok(parse_repos(&repos))
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        // The `.travis.yml` lives in SCM; the Importer fetches it at dry-run. Here
        // we only confirm the pipeline exists + its classification.
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
        // Travis has no Azure-DevOps-style service connections; credentials live in
        // repo env vars. Empty for v2.
        Ok(Vec::new())
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let repos = self.get("repos?limit=100").await?;
        let mut out = Vec::new();
        if let Some(arr) = repos.get("repositories").and_then(Value::as_array) {
            for r in arr {
                let Some(slug) = r.get("slug").and_then(Value::as_str) else {
                    continue;
                };
                let env = self
                    .get(&format!("repo/{}/env_vars", encode_slug(slug)))
                    .await
                    .unwrap_or(Value::Null);
                let group = parse_env_vars(&env, slug);
                if !group.variables.is_empty() {
                    out.push(group);
                }
            }
        }
        Ok(out)
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // Travis has no marketplace-task concept; the Importer's audit reports the
        // build matrix / language stack.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPOS: &str = include_str!("../../../fixtures/travis/repos.json");
    const ENV_VARS: &str = include_str!("../../../fixtures/travis/env_vars.json");

    #[test]
    fn owners_are_the_distinct_logins() {
        let v: Value = serde_json::from_str(REPOS).unwrap();
        let names: Vec<_> = parse_owners(&v).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["acme".to_string(), "platform".to_string()]);
    }

    #[test]
    fn every_repo_is_a_yaml_pipeline_keyed_by_slug() {
        let v: Value = serde_json::from_str(REPOS).unwrap();
        let pipelines = parse_repos(&v);
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
    fn env_vars_flag_secret_by_public_and_keep_names_only() {
        let v: Value = serde_json::from_str(ENV_VARS).unwrap();
        let group = parse_env_vars(&v, "acme/web");
        assert_eq!(group.variables.len(), 2);

        let public = group.variables.iter().find(|x| !x.is_secret).unwrap();
        assert_eq!(public.name, "PUBLIC_API_URL");
        // A non-public variable is secret-flagged by name only.
        let secret = group.variables.iter().find(|x| x.is_secret).unwrap();
        assert_eq!(secret.name, "DEPLOY_SECRET");
        // The public value in the fixture must never survive parsing.
        let serialized = serde_json::to_string(&group).unwrap();
        assert!(!serialized.contains("api.example.com"));
    }

    #[test]
    fn slug_is_percent_encoded_for_the_api() {
        assert_eq!(encode_slug("acme/web"), "acme%2Fweb");
    }

    /// Live: enumerate a real Travis account. Ignored by default.
    #[tokio::test]
    #[ignore = "requires a live Travis (TRAVIS_TOKEN)"]
    async fn live_enumerate() {
        let adapter = TravisAdapter::from_env().expect("TRAVIS_TOKEN set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
