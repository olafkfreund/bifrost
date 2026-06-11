//! Sandbox validation — trigger a converted workflow in an isolated sandbox
//! (#58), the first step of smoke-parity (plan §10). Behind the [`SandboxTrigger`]
//! trait so it is mockable and the real GitHub call is **opt-in** — triggering a
//! workflow runs CI and consumes minutes, so it never fires unless the operator
//! enables it.
//!
//! Uses the GitHub Actions `workflow_dispatch` API. Note: GitHub only dispatches
//! workflows that exist on the repo's **default branch**; a sandbox repo whose
//! default branch is the converted branch (or where the workflow has been merged)
//! satisfies this. Capturing the resulting run + diffing against the ADO baseline
//! are #59/#60.

use async_trait::async_trait;
use serde_json::json;

/// What to trigger: the workflow file and the git ref, in a sandbox repo.
#[derive(Debug, Clone)]
pub struct TriggerRequest {
    /// `owner/repo` of the sandbox.
    pub repo: String,
    /// Workflow file name (e.g. `sarc-main.yml`) or its numeric id.
    pub workflow_file: String,
    /// Git ref to run against (branch/tag).
    pub git_ref: String,
}

/// Outcome of a dispatch: which workflow/ref was triggered (the run itself is
/// captured in #59).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerResult {
    pub repo: String,
    pub workflow_file: String,
    pub git_ref: String,
    pub dispatched: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ValidateError {
    #[error("github API error: {0}")]
    Api(String),
    #[error("missing configuration: {0}")]
    Config(String),
}

/// Triggers a converted workflow in a sandbox. Mockable; the real impl is opt-in
/// so a conversion never silently runs CI.
#[async_trait]
pub trait SandboxTrigger: Send + Sync {
    async fn trigger(&self, req: &TriggerRequest) -> Result<TriggerResult, ValidateError>;
}

/// Offline trigger: records the request and reports a (synthetic) dispatch.
#[derive(Debug, Clone, Default)]
pub struct MockSandboxTrigger;

#[async_trait]
impl SandboxTrigger for MockSandboxTrigger {
    async fn trigger(&self, req: &TriggerRequest) -> Result<TriggerResult, ValidateError> {
        Ok(TriggerResult {
            repo: req.repo.clone(),
            workflow_file: req.workflow_file.clone(),
            git_ref: req.git_ref.clone(),
            dispatched: true,
        })
    }
}

/// Real trigger: GitHub Actions `workflow_dispatch` via the REST API.
pub struct GitHubSandboxTrigger {
    token: String,
    api_base: String,
    client: reqwest::Client,
}

impl GitHubSandboxTrigger {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            api_base: "https://api.github.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Build from `GITHUB_TOKEN` (required) and optional `GITHUB_API_BASE`.
    pub fn from_env() -> Result<Self, ValidateError> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| ValidateError::Config("GITHUB_TOKEN not set".into()))?;
        let api_base =
            std::env::var("GITHUB_API_BASE").unwrap_or_else(|_| "https://api.github.com".into());
        Ok(Self {
            token,
            api_base,
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl SandboxTrigger for GitHubSandboxTrigger {
    async fn trigger(&self, req: &TriggerRequest) -> Result<TriggerResult, ValidateError> {
        let url = format!(
            "{}/repos/{}/actions/workflows/{}/dispatches",
            self.api_base, req.repo, req.workflow_file
        );
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .json(&json!({ "ref": req.git_ref }))
            .send()
            .await
            .map_err(|e| ValidateError::Api(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ValidateError::Api(format!("{status}: {body}")));
        }
        Ok(TriggerResult {
            repo: req.repo.clone(),
            workflow_file: req.workflow_file.clone(),
            git_ref: req.git_ref.clone(),
            dispatched: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> TriggerRequest {
        TriggerRequest {
            repo: "olafkfreund/bifrost-sandbox".into(),
            workflow_file: "sarc-main.yml".into(),
            git_ref: "bifrost/convert-sarc-main".into(),
        }
    }

    #[tokio::test]
    async fn mock_trigger_reports_dispatched() {
        let r = MockSandboxTrigger.trigger(&req()).await.unwrap();
        assert!(r.dispatched);
        assert_eq!(r.workflow_file, "sarc-main.yml");
        assert_eq!(r.git_ref, "bifrost/convert-sarc-main");
    }

    /// Live: dispatches a REAL workflow run in `BIFROST_GH_REPO`. Ignored by
    /// default (it runs CI). The workflow must exist on the repo's default branch.
    #[tokio::test]
    #[ignore = "triggers a real workflow run — run only against a sandbox you own"]
    async fn live_dispatches_a_workflow() {
        let repo = std::env::var("BIFROST_GH_REPO").expect("BIFROST_GH_REPO set");
        let workflow = std::env::var("BIFROST_TEST_WORKFLOW").expect("BIFROST_TEST_WORKFLOW set");
        let trigger = GitHubSandboxTrigger::from_env().expect("GITHUB_TOKEN set");
        let r = trigger
            .trigger(&TriggerRequest {
                repo,
                workflow_file: workflow,
                git_ref: "main".into(),
            })
            .await
            .expect("dispatch succeeds");
        assert!(r.dispatched);
    }
}
