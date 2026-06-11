//! ADO baseline source (#60): the last successful Azure DevOps run for a
//! pipeline, reduced to the smoke-parity facts ([`bifrost_core::RunFacts`]) the
//! converted run is diffed against.
//!
//! Behind the [`BaselineSource`] trait so it is mockable and the real ADO read is
//! **opt-in** (it hits the ADO REST API). We record artifact *names* and the
//! run's success — never contents, never secrets. ADO does not expose
//! workflow-level "declared outputs" the way GitHub Actions does, so the ADO
//! side's outputs are left empty; the smoke-parity caveat already states this
//! limit.

use async_trait::async_trait;
use bifrost_core::RunFacts;

/// Which baseline to fetch: a pipeline (by name) within an ADO project.
#[derive(Debug, Clone)]
pub struct BaselineRequest {
    pub project: String,
    /// The ADO build definition name (matches the pipeline's name in the portfolio).
    pub pipeline_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum BaselineError {
    #[error("azure devops API error: {0}")]
    Api(String),
    #[error("missing configuration: {0}")]
    Config(String),
    #[error("no baseline run found: {0}")]
    NotFound(String),
}

/// Fetches the last successful ADO run for a pipeline as [`RunFacts`]. Mockable;
/// the real impl is opt-in so parity never silently calls ADO.
#[async_trait]
pub trait BaselineSource: Send + Sync {
    async fn baseline(&self, req: &BaselineRequest) -> Result<RunFacts, BaselineError>;
}

/// Offline baseline: a synthetic successful run, so the parity diff can be
/// exercised without ADO.
#[derive(Debug, Clone, Default)]
pub struct MockBaselineSource;

#[async_trait]
impl BaselineSource for MockBaselineSource {
    async fn baseline(&self, _req: &BaselineRequest) -> Result<RunFacts, BaselineError> {
        Ok(RunFacts {
            succeeded: true,
            artifacts: vec!["build-output".to_string()],
            outputs: Vec::new(),
        })
    }
}

const API_VERSION: &str = "7.1";

/// Real baseline: reads the latest succeeded build + its artifacts from the ADO
/// REST API.
pub struct AzureDevOpsBaseline {
    client: reqwest::Client,
    org_url: String,
    pat: String,
}

impl AzureDevOpsBaseline {
    pub fn new(org_url: impl Into<String>, pat: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            org_url: org_url.into().trim_end_matches('/').to_string(),
            pat: pat.into(),
        }
    }

    /// Build from `AZDO_ORG_URL` + `AZDO_PAT` (set by `.envrc`).
    pub fn from_env() -> Result<Self, BaselineError> {
        let org = std::env::var("AZDO_ORG_URL")
            .map_err(|_| BaselineError::Config("AZDO_ORG_URL not set".into()))?;
        let pat = std::env::var("AZDO_PAT")
            .map_err(|_| BaselineError::Config("AZDO_PAT not set".into()))?;
        Ok(Self::new(org, pat))
    }

    async fn get(&self, project: &str, path: &str) -> Result<serde_json::Value, BaselineError> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}/{}/_apis/{path}{sep}api-version={API_VERSION}",
            self.org_url, project
        );
        let resp = self
            .client
            .get(&url)
            .basic_auth("", Some(&self.pat))
            .send()
            .await
            .map_err(|e| BaselineError::Api(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BaselineError::Api(format!("{status}: {body}")));
        }
        resp.json()
            .await
            .map_err(|e| BaselineError::Api(e.to_string()))
    }
}

#[async_trait]
impl BaselineSource for AzureDevOpsBaseline {
    async fn baseline(&self, req: &BaselineRequest) -> Result<RunFacts, BaselineError> {
        // Resolve the build definition by name.
        let defs = self
            .get(
                &req.project,
                &format!("build/definitions?name={}", req.pipeline_name),
            )
            .await?;
        let def_id = defs["value"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|d| d["id"].as_u64())
            .ok_or_else(|| {
                BaselineError::NotFound(format!("no definition named '{}'", req.pipeline_name))
            })?;

        // Latest *succeeded* completed build for that definition.
        let builds = self
            .get(
                &req.project,
                &format!(
                    "build/builds?definitions={def_id}&resultFilter=succeeded\
                     &statusFilter=completed&$top=1&queryOrder=finishTimeDescending"
                ),
            )
            .await?;
        let build_id = builds["value"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|b| b["id"].as_u64())
            .ok_or_else(|| {
                BaselineError::NotFound(format!("no successful build for '{}'", req.pipeline_name))
            })?;

        // Artifact names for that build.
        let artifacts_json = self
            .get(&req.project, &format!("build/builds/{build_id}/artifacts"))
            .await?;
        let artifacts = artifacts_json["value"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(RunFacts {
            succeeded: true,
            artifacts,
            outputs: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> BaselineRequest {
        BaselineRequest {
            project: "SARC".into(),
            pipeline_name: "sarc-main".into(),
        }
    }

    #[tokio::test]
    async fn mock_baseline_reports_a_successful_run() {
        let facts = MockBaselineSource.baseline(&req()).await.unwrap();
        assert!(facts.succeeded);
        assert_eq!(facts.artifacts, vec!["build-output"]);
        assert!(facts.outputs.is_empty());
    }

    /// Live: reads the last successful ADO run for `BIFROST_BASELINE_PIPELINE` in
    /// `BIFROST_BASELINE_PROJECT`. Ignored by default (calls the ADO API).
    #[tokio::test]
    #[ignore = "reads the real ADO API — run only with AZDO_* set against a project you own"]
    async fn live_fetches_ado_baseline() {
        let project = std::env::var("BIFROST_BASELINE_PROJECT").expect("BIFROST_BASELINE_PROJECT");
        let pipeline =
            std::env::var("BIFROST_BASELINE_PIPELINE").expect("BIFROST_BASELINE_PIPELINE");
        let src = AzureDevOpsBaseline::from_env().expect("AZDO_* env set");
        let facts = src
            .baseline(&BaselineRequest {
                project,
                pipeline_name: pipeline,
            })
            .await
            .expect("baseline fetch succeeds");
        assert!(facts.succeeded);
    }
}
