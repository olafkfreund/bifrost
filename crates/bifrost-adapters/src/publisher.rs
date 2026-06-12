//! Commit an approved workflow and open a PR (#56).
//!
//! Behind the [`Publisher`] trait so it can be mocked in tests and so the real
//! GitHub calls are opt-in — **never silent**: orchestration only calls a
//! publisher when a reviewer has approved and the operator has enabled the live
//! path. [`MockPublisher`] returns a synthetic PR URL for offline runs;
//! [`GitHubPublisher`] performs the real create-branch → commit-file → open-PR
//! sequence against the GitHub REST API.

use async_trait::async_trait;
use base64::Engine;
use serde_json::json;

/// What to commit and how to open the PR.
#[derive(Debug, Clone)]
pub struct CommitRequest {
    /// `owner/repo`.
    pub repo: String,
    /// Branch to create for the change.
    pub branch: String,
    /// Base branch to target (e.g. `main`).
    pub base: String,
    /// Path of the workflow file, e.g. `.github/workflows/sarc-main.yml`.
    pub workflow_path: String,
    /// The approved workflow YAML.
    pub workflow_yaml: String,
    /// PR title.
    pub title: String,
    /// PR body (links the proposal + manual-task checklist).
    pub body: String,
}

/// The opened PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    pub pr_url: String,
    pub branch: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("github API error: {0}")]
    Api(String),
    #[error("missing configuration: {0}")]
    Config(String),
}

/// Commits an approved workflow and opens a PR. Mockable; the real impl is
/// opt-in so a conversion never silently writes to a customer repo.
#[async_trait]
pub trait Publisher: Send + Sync {
    async fn commit_workflow(&self, req: &CommitRequest) -> Result<CommitResult, PublishError>;
}

/// Offline publisher: records the request shape and returns a synthetic PR URL.
#[derive(Debug, Clone, Default)]
pub struct MockPublisher;

#[async_trait]
impl Publisher for MockPublisher {
    async fn commit_workflow(&self, req: &CommitRequest) -> Result<CommitResult, PublishError> {
        Ok(CommitResult {
            pr_url: format!("https://github.com/{}/pull/MOCK", req.repo),
            branch: req.branch.clone(),
        })
    }
}

/// Real GitHub publisher: create-branch → commit-file → open-PR via the REST API
/// with a Bearer token. Used only when the live commit path is enabled.
pub struct GitHubPublisher {
    token: String,
    api_base: String,
    client: reqwest::Client,
}

impl GitHubPublisher {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            api_base: "https://api.github.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Override the API base (e.g. a GitHub Enterprise URL).
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Build from `GITHUB_TOKEN` (required) and optional `GITHUB_API_BASE`.
    pub fn from_env() -> Result<Self, PublishError> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| PublishError::Config("GITHUB_TOKEN not set".into()))?;
        let api_base =
            std::env::var("GITHUB_API_BASE").unwrap_or_else(|_| "https://api.github.com".into());
        Ok(Self {
            token,
            api_base,
            client: reqwest::Client::new(),
        })
    }

    async fn get_json(&self, url: &str) -> Result<serde_json::Value, PublishError> {
        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .send()
            .await
            .map_err(|e| PublishError::Api(e.to_string()))?;
        self.json_or_err(resp).await
    }

    async fn post_json(
        &self,
        url: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, PublishError> {
        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .json(&body)
            .send()
            .await
            .map_err(|e| PublishError::Api(e.to_string()))?;
        self.json_or_err(resp).await
    }

    async fn put_json(
        &self,
        url: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, PublishError> {
        let resp = self
            .client
            .put(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .json(&body)
            .send()
            .await
            .map_err(|e| PublishError::Api(e.to_string()))?;
        self.json_or_err(resp).await
    }

    async fn json_or_err(
        &self,
        resp: reqwest::Response,
    ) -> Result<serde_json::Value, PublishError> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| PublishError::Api(e.to_string()))?;
        if !status.is_success() {
            return Err(PublishError::Api(format!("{status}: {text}")));
        }
        serde_json::from_str(&text).map_err(|e| PublishError::Api(format!("{e}: {text}")))
    }
}

#[async_trait]
impl Publisher for GitHubPublisher {
    async fn commit_workflow(&self, req: &CommitRequest) -> Result<CommitResult, PublishError> {
        // Never write to the base/default branch: changes only ever land as a PR on
        // a separate branch (review-first, #204).
        if req.branch == req.base {
            return Err(PublishError::Api(format!(
                "refusing to commit to the base branch '{}' — converted workflows land on a \
                 separate branch and open a pull request",
                req.base
            )));
        }
        let base = &self.api_base;
        let repo = &req.repo;

        // 1. Base branch head SHA.
        let base_ref = self
            .get_json(&format!("{base}/repos/{repo}/git/ref/heads/{}", req.base))
            .await?;
        let base_sha = base_ref["object"]["sha"]
            .as_str()
            .ok_or_else(|| PublishError::Api("no base sha".into()))?
            .to_string();

        // 2. Create the feature branch.
        self.post_json(
            &format!("{base}/repos/{repo}/git/refs"),
            json!({ "ref": format!("refs/heads/{}", req.branch), "sha": base_sha }),
        )
        .await?;

        // 3. Commit the workflow file on the branch.
        let content =
            base64::engine::general_purpose::STANDARD.encode(req.workflow_yaml.as_bytes());
        self.put_json(
            &format!("{base}/repos/{repo}/contents/{}", req.workflow_path),
            json!({
                "message": req.title,
                "content": content,
                "branch": req.branch,
            }),
        )
        .await?;

        // 4. Open the PR.
        let pr = self
            .post_json(
                &format!("{base}/repos/{repo}/pulls"),
                json!({
                    "title": req.title,
                    "head": req.branch,
                    "base": req.base,
                    "body": req.body,
                }),
            )
            .await?;
        let pr_url = pr["html_url"]
            .as_str()
            .ok_or_else(|| PublishError::Api("no PR url".into()))?
            .to_string();

        Ok(CommitResult {
            pr_url,
            branch: req.branch.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_req() -> CommitRequest {
        CommitRequest {
            repo: "olafkfreund/bifrost-sandbox".into(),
            branch: "bifrost/convert-sarc-main".into(),
            base: "main".into(),
            workflow_path: ".github/workflows/sarc-main.yml".into(),
            workflow_yaml: "name: SARC-main\non: push\n".into(),
            title: "Bifrost: convert SARC-main".into(),
            body: "Converted by Bifrost.".into(),
        }
    }

    #[tokio::test]
    async fn mock_publisher_returns_a_synthetic_pr_url() {
        let result = MockPublisher.commit_workflow(&sample_req()).await.unwrap();
        assert_eq!(
            result.pr_url,
            "https://github.com/olafkfreund/bifrost-sandbox/pull/MOCK"
        );
        assert_eq!(result.branch, "bifrost/convert-sarc-main");
    }

    /// The PR-only guard: committing to the base branch is refused before any HTTP
    /// call, so a converted workflow can never be pushed to the default branch.
    #[tokio::test]
    async fn refuses_to_commit_to_the_base_branch() {
        let publisher = GitHubPublisher::new("token");
        let mut req = sample_req();
        req.branch = req.base.clone(); // branch == base
        let err = publisher.commit_workflow(&req).await.unwrap_err();
        assert!(matches!(err, PublishError::Api(_)));
    }

    /// Live smoke test — creates a REAL branch + PR in `BIFROST_GH_REPO`. Ignored
    /// by default (it is an outward action). Run intentionally with:
    ///   GITHUB_TOKEN=… BIFROST_GH_REPO=owner/repo \
    ///     cargo test -p bifrost-adapters -- --ignored live_opens_a_pr
    #[tokio::test]
    #[ignore = "creates a real PR — run only against a sandbox repo you own"]
    async fn live_opens_a_pr() {
        let repo = std::env::var("BIFROST_GH_REPO").expect("BIFROST_GH_REPO set");
        let publisher = GitHubPublisher::from_env().expect("GITHUB_TOKEN set");
        let mut req = sample_req();
        req.repo = repo;
        let result = publisher.commit_workflow(&req).await.expect("opens a PR");
        assert!(result.pr_url.contains("/pull/"));
    }
}
