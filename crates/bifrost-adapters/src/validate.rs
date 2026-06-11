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

/// Which run to capture: the workflow file + the branch it was dispatched on, in
/// a sandbox repo. The newest `workflow_dispatch` run on that branch is the one
/// we triggered in #58.
#[derive(Debug, Clone)]
pub struct RunQuery {
    /// `owner/repo` of the sandbox.
    pub repo: String,
    /// Workflow file name (e.g. `sarc-main.yml`) or its numeric id.
    pub workflow_file: String,
    /// Branch the workflow was dispatched on.
    pub git_ref: String,
}

/// One job within a captured run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunJob {
    pub name: String,
    /// `success` | `failure` | `cancelled` | `skipped` | … (`None` while running).
    pub conclusion: Option<String>,
}

/// One artifact produced by a captured run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArtifact {
    pub name: String,
    pub size_bytes: u64,
}

/// The captured outcome of a converted run (#59): status, jobs, and artifacts.
/// Declared outputs come from the workflow YAML (see [`declared_outputs`]) and
/// are attached by the caller — the run API itself doesn't surface them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunResult {
    /// GitHub run id (0 for the mock / when no run was found).
    pub run_id: u64,
    /// `queued` | `in_progress` | `completed`.
    pub status: String,
    /// `success` | `failure` | … — `None` until the run completes.
    pub conclusion: Option<String>,
    pub jobs: Vec<RunJob>,
    pub artifacts: Vec<RunArtifact>,
}

/// Extract the names of workflow- and job-level `outputs:` declared in a workflow
/// YAML. Smoke parity (#60) compares these against the ADO baseline's declared
/// outputs; capturing them here keeps the run record self-describing. Best-effort
/// line scan (we avoid a YAML dep): collects keys under any `outputs:` mapping.
pub fn declared_outputs(workflow_yaml: &str) -> Vec<String> {
    let mut outputs = Vec::new();
    let mut in_block = false;
    let mut block_indent = 0usize;
    for line in workflow_yaml.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() || trimmed.trim_start().starts_with('#') {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        let key = trimmed.trim_start();
        if !in_block {
            if key == "outputs:" {
                in_block = true;
                block_indent = indent;
            }
            continue;
        }
        // Inside an `outputs:` mapping: its keys are indented further than the
        // `outputs:` line itself. A line at or below that indent ends the block.
        if indent <= block_indent {
            in_block = false;
            if key == "outputs:" {
                in_block = true;
                block_indent = indent;
            }
            continue;
        }
        if let Some((name, _)) = key.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && !outputs.iter().any(|o| o == name) {
                outputs.push(name.to_string());
            }
        }
    }
    outputs
}

/// Captures the result of a converted run. Mockable; the real impl reads the
/// GitHub Actions API and is opt-in (a read-only call, but still external).
#[async_trait]
pub trait RunCollector: Send + Sync {
    async fn collect(&self, query: &RunQuery) -> Result<RunResult, ValidateError>;
}

/// Offline collector: reports a synthetic successful run so the validation chain
/// can be exercised without GitHub.
#[derive(Debug, Clone, Default)]
pub struct MockRunCollector;

#[async_trait]
impl RunCollector for MockRunCollector {
    async fn collect(&self, _query: &RunQuery) -> Result<RunResult, ValidateError> {
        Ok(RunResult {
            run_id: 0,
            status: "completed".into(),
            conclusion: Some("success".into()),
            jobs: vec![RunJob {
                name: "build".into(),
                conclusion: Some("success".into()),
            }],
            artifacts: vec![RunArtifact {
                name: "build-output".into(),
                size_bytes: 0,
            }],
        })
    }
}

/// Real collector: reads the newest `workflow_dispatch` run for the branch from
/// the GitHub Actions REST API, then its jobs and artifacts.
pub struct GitHubRunCollector {
    token: String,
    api_base: String,
    client: reqwest::Client,
}

impl GitHubRunCollector {
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

    async fn get_json(&self, url: &str) -> Result<serde_json::Value, ValidateError> {
        let resp = self
            .client
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .send()
            .await
            .map_err(|e| ValidateError::Api(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ValidateError::Api(format!("{status}: {body}")));
        }
        resp.json()
            .await
            .map_err(|e| ValidateError::Api(e.to_string()))
    }
}

#[async_trait]
impl RunCollector for GitHubRunCollector {
    async fn collect(&self, query: &RunQuery) -> Result<RunResult, ValidateError> {
        // Newest workflow_dispatch run on the branch (default sort is newest first).
        let runs_url = format!(
            "{}/repos/{}/actions/workflows/{}/runs?branch={}&event=workflow_dispatch&per_page=1",
            self.api_base, query.repo, query.workflow_file, query.git_ref
        );
        let runs = self.get_json(&runs_url).await?;
        let run = runs["workflow_runs"]
            .as_array()
            .and_then(|a| a.first())
            .ok_or_else(|| ValidateError::Api("no workflow_dispatch run found".into()))?;
        let run_id = run["id"].as_u64().unwrap_or(0);
        let status = run["status"].as_str().unwrap_or("unknown").to_string();
        let conclusion = run["conclusion"].as_str().map(|s| s.to_string());

        let jobs_url = format!(
            "{}/repos/{}/actions/runs/{}/jobs",
            self.api_base, query.repo, run_id
        );
        let jobs_json = self.get_json(&jobs_url).await?;
        let jobs = jobs_json["jobs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|j| RunJob {
                        name: j["name"].as_str().unwrap_or_default().to_string(),
                        conclusion: j["conclusion"].as_str().map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let artifacts_url = format!(
            "{}/repos/{}/actions/runs/{}/artifacts",
            self.api_base, query.repo, run_id
        );
        let artifacts_json = self.get_json(&artifacts_url).await?;
        let artifacts = artifacts_json["artifacts"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| RunArtifact {
                        name: x["name"].as_str().unwrap_or_default().to_string(),
                        size_bytes: x["size_in_bytes"].as_u64().unwrap_or(0),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(RunResult {
            run_id,
            status,
            conclusion,
            jobs,
            artifacts,
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

    fn run_query() -> RunQuery {
        RunQuery {
            repo: "olafkfreund/bifrost-sandbox".into(),
            workflow_file: "sarc-main.yml".into(),
            git_ref: "bifrost/convert-sarc-main".into(),
        }
    }

    #[tokio::test]
    async fn mock_collector_reports_success() {
        let r = MockRunCollector.collect(&run_query()).await.unwrap();
        assert_eq!(r.status, "completed");
        assert_eq!(r.conclusion.as_deref(), Some("success"));
        assert_eq!(r.jobs.len(), 1);
        assert_eq!(r.jobs[0].name, "build");
        assert_eq!(r.artifacts.len(), 1);
    }

    #[test]
    fn declared_outputs_collects_workflow_and_job_outputs() {
        let yaml = r#"
name: ci
on: [push]
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      image_tag: ${{ steps.meta.outputs.tag }}
      digest: ${{ steps.build.outputs.digest }}
    steps:
      - run: echo hi
  publish:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo done
"#;
        let outputs = declared_outputs(yaml);
        assert_eq!(outputs, vec!["image_tag", "digest"]);
    }

    #[test]
    fn declared_outputs_empty_when_none() {
        let yaml = "name: ci\non: [push]\njobs:\n  build:\n    steps:\n      - run: echo hi\n";
        assert!(declared_outputs(yaml).is_empty());
    }

    /// Live: captures the newest dispatched run in `BIFROST_GH_REPO`. Ignored by
    /// default (reads the GitHub API for a repo you own).
    #[tokio::test]
    #[ignore = "reads a real GitHub run — run only against a sandbox you own"]
    async fn live_captures_a_run() {
        let repo = std::env::var("BIFROST_GH_REPO").expect("BIFROST_GH_REPO set");
        let workflow = std::env::var("BIFROST_TEST_WORKFLOW").expect("BIFROST_TEST_WORKFLOW set");
        let collector = GitHubRunCollector::from_env().expect("GITHUB_TOKEN set");
        let r = collector
            .collect(&RunQuery {
                repo,
                workflow_file: workflow,
                git_ref: "main".into(),
            })
            .await
            .expect("collect succeeds");
        assert!(!r.status.is_empty());
    }
}
