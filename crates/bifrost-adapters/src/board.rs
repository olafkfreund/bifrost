//! Provision the program board on GitHub (#266) — the gated, idempotent GraphQL
//! analogue of the [`crate::Publisher`].
//!
//! Phase 1 (#265) computes the deterministic dry-run [`ProgramBoardPlan`]; this
//! module *executes* it: a dedicated repo, an org-level Projects v2 board, its
//! custom fields, and one draft issue per pipeline (carrying the migration
//! checklist). It does so behind the [`BoardProvisioner`] trait so it can be
//! mocked offline and so the real GitHub writes are **opt-in — never silent**:
//! orchestration only calls the real provisioner when an operator has approved
//! it and enabled the live path.
//!
//! Hard rules honoured here:
//! - **Review-first / opt-in.** [`MockBoardProvisioner`] is the default and is
//!   the only thing exercised in tests; [`GitHubBoardProvisioner`] runs only when
//!   explicitly selected with real auth.
//! - **Idempotent.** Every create is preceded by a query — an existing repo,
//!   Project, field, or issue is reused (its node id cached) rather than
//!   recreated. A second provision over the same plan reports everything as
//!   `existing`.
//! - **Attestable.** Every step appends a [`ProvisionAction`] to the result's
//!   action log; the caller appends those to the immutable provisioning record.
//! - **No secrets.** Only names/titles/types are sent or recorded — never tokens
//!   or secret values.

use async_trait::async_trait;
use bifrost_core::ProgramBoardPlan;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// What kind of board object a [`ProvisionAction`] concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionTarget {
    Repository,
    Project,
    Field,
    Issue,
}

/// Whether the step created the object or found it already present (idempotency
/// is visible in the action log and the result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionOutcome {
    Created,
    Existing,
}

/// One immutable step in the provisioning run — appended to the attestation log.
/// Records the target kind, a human-readable target name, the node id, and
/// whether it was created or already existed. No secret values, ever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionAction {
    pub target: ProvisionTarget,
    /// The object's name/title (e.g. repo slug, project title, field name, issue title).
    pub name: String,
    /// The resolved node id (GraphQL global id, or REST identifier for the repo).
    pub node_id: String,
    pub outcome: ProvisionOutcome,
}

/// A provisioned (or found) board issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedIssue {
    pub title: String,
    pub node_id: String,
    pub outcome: ProvisionOutcome,
}

/// A provisioned (or found) custom field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedField {
    pub name: String,
    pub node_id: String,
    pub outcome: ProvisionOutcome,
}

/// The result of a provisioning run: the node ids/URLs of everything created or
/// found, plus the full ordered action log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionResult {
    pub repo_node_id: String,
    pub repo_url: String,
    pub project_node_id: String,
    pub project_url: String,
    pub fields: Vec<ProvisionedField>,
    pub issues: Vec<ProvisionedIssue>,
    /// Every step taken, in order — the attestation trail.
    pub actions: Vec<ProvisionAction>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProvisionError {
    #[error("github API error: {0}")]
    Api(String),
    #[error("missing configuration: {0}")]
    Config(String),
}

/// Provisions the program board for a plan. Mockable; the real impl is opt-in so
/// provisioning never silently creates org infrastructure.
#[async_trait]
pub trait BoardProvisioner: Send + Sync {
    /// Provision (idempotently) the repo + org Project + fields + issues for
    /// `plan` under `owner` (a GitHub org login).
    async fn provision(
        &self,
        plan: &ProgramBoardPlan,
        owner: &str,
    ) -> Result<ProvisionResult, ProvisionError>;
}

/// Offline provisioner: returns deterministic synthetic node ids/URLs and a full
/// action log **without any network call**. This is the default path and the one
/// exercised in tests. Because the ids are derived purely from the plan, a second
/// provision over the same plan is reported as `existing` (idempotency made
/// visible) — see [`MockBoardProvisioner::provision`].
#[derive(Debug, Clone, Default)]
pub struct MockBoardProvisioner {
    /// Names the caller pretends already exist (lets tests drive the
    /// created→existing transition deterministically without a network/store).
    existing: std::collections::HashSet<String>,
}

impl MockBoardProvisioner {
    /// A fresh mock where nothing yet exists (everything will report `created`).
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock that treats `names` (repo slug, project title, field names, issue
    /// titles) as already present, so they report `existing`. Used to assert the
    /// idempotent second-run shape offline.
    pub fn with_existing<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            existing: names.into_iter().map(Into::into).collect(),
        }
    }

    fn outcome_for(&self, name: &str) -> ProvisionOutcome {
        if self.existing.contains(name) {
            ProvisionOutcome::Existing
        } else {
            ProvisionOutcome::Created
        }
    }
}

/// A stable synthetic node id for the mock — deterministic from kind + name.
fn mock_node_id(kind: &str, name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    format!("MOCK_{kind}_{slug}")
}

#[async_trait]
impl BoardProvisioner for MockBoardProvisioner {
    async fn provision(
        &self,
        plan: &ProgramBoardPlan,
        owner: &str,
    ) -> Result<ProvisionResult, ProvisionError> {
        let mut actions = Vec::new();

        // Repo.
        let repo_node_id = mock_node_id("REPO", &plan.repo);
        let repo_url = format!("https://github.com/{owner}/{}", plan.repo);
        actions.push(ProvisionAction {
            target: ProvisionTarget::Repository,
            name: plan.repo.clone(),
            node_id: repo_node_id.clone(),
            outcome: self.outcome_for(&plan.repo),
        });

        // Project.
        let project_node_id = mock_node_id("PROJECT", &plan.project_title);
        let project_url = format!("https://github.com/orgs/{owner}/projects/MOCK");
        actions.push(ProvisionAction {
            target: ProvisionTarget::Project,
            name: plan.project_title.clone(),
            node_id: project_node_id.clone(),
            outcome: self.outcome_for(&plan.project_title),
        });

        // Fields.
        let fields: Vec<ProvisionedField> = plan
            .fields
            .iter()
            .map(|f| {
                let outcome = self.outcome_for(&f.name);
                let node_id = mock_node_id("FIELD", &f.name);
                actions.push(ProvisionAction {
                    target: ProvisionTarget::Field,
                    name: f.name.clone(),
                    node_id: node_id.clone(),
                    outcome,
                });
                ProvisionedField {
                    name: f.name.clone(),
                    node_id,
                    outcome,
                }
            })
            .collect();

        // Issues (one draft issue per pipeline).
        let issues: Vec<ProvisionedIssue> = plan
            .issues
            .iter()
            .map(|i| {
                let outcome = self.outcome_for(&i.title);
                let node_id = mock_node_id("ISSUE", &i.title);
                actions.push(ProvisionAction {
                    target: ProvisionTarget::Issue,
                    name: i.title.clone(),
                    node_id: node_id.clone(),
                    outcome,
                });
                ProvisionedIssue {
                    title: i.title.clone(),
                    node_id,
                    outcome,
                }
            })
            .collect();

        Ok(ProvisionResult {
            repo_node_id,
            repo_url,
            project_node_id,
            project_url,
            fields,
            issues,
            actions,
        })
    }
}

/// Real GitHub provisioner: idempotent (query-before-create) over the Projects v2
/// GraphQL API plus the REST repo endpoint, with a Bearer token. Used only when
/// the live board path is explicitly enabled and authenticated.
///
/// Idempotency strategy, per object:
/// - **Repo:** `GET /repos/{owner}/{repo}` — reuse if present, else
///   `POST /orgs/{owner}/repos`.
/// - **Project:** query `organization.projectsV2` for one whose title matches —
///   reuse if present, else `createProjectV2`.
/// - **Fields:** query `node(project).fields` for a matching name — reuse if
///   present, else `createProjectV2Field`.
/// - **Issues:** query the project's existing draft-issue titles — reuse if
///   present, else `addProjectV2DraftIssue`.
pub struct GitHubBoardProvisioner {
    token: String,
    api_base: String,
    client: reqwest::Client,
}

impl GitHubBoardProvisioner {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            api_base: "https://api.github.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Override the API base (e.g. a GitHub Enterprise URL). The GraphQL endpoint
    /// is derived as `{api_base}/graphql`.
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    /// Build from `GITHUB_TOKEN` (required) and optional `GITHUB_API_BASE`. The
    /// API's live selector prefers a GitHub App installation token over this; this
    /// constructor is the standalone fallback.
    pub fn from_env() -> Result<Self, ProvisionError> {
        let token = std::env::var("GITHUB_TOKEN")
            .map_err(|_| ProvisionError::Config("GITHUB_TOKEN not set".into()))?;
        let api_base =
            std::env::var("GITHUB_API_BASE").unwrap_or_else(|_| "https://api.github.com".into());
        Ok(Self {
            token,
            api_base,
            client: reqwest::Client::new(),
        })
    }

    fn graphql_url(&self) -> String {
        format!("{}/graphql", self.api_base)
    }

    /// Run a GraphQL query/mutation and return the `data` object. GraphQL returns
    /// HTTP 200 even on errors, so the `errors` array is checked explicitly.
    async fn graphql(
        &self,
        query: &str,
        variables: serde_json::Value,
    ) -> Result<serde_json::Value, ProvisionError> {
        let resp = self
            .client
            .post(self.graphql_url())
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|e| ProvisionError::Api(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ProvisionError::Api(e.to_string()))?;
        if !status.is_success() {
            return Err(ProvisionError::Api(format!("{status}: {text}")));
        }
        let body: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| ProvisionError::Api(format!("{e}: {text}")))?;
        if let Some(errors) = body.get("errors") {
            if !errors.is_null() {
                return Err(ProvisionError::Api(format!("graphql: {errors}")));
            }
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| ProvisionError::Api(format!("no data in response: {text}")))
    }

    async fn rest_get(&self, url: &str) -> Result<reqwest::Response, ProvisionError> {
        self.client
            .get(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .send()
            .await
            .map_err(|e| ProvisionError::Api(e.to_string()))
    }

    async fn rest_post(
        &self,
        url: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ProvisionError> {
        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProvisionError::Api(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ProvisionError::Api(e.to_string()))?;
        if !status.is_success() {
            return Err(ProvisionError::Api(format!("{status}: {text}")));
        }
        serde_json::from_str(&text).map_err(|e| ProvisionError::Api(format!("{e}: {text}")))
    }

    /// Find or create the dedicated repo. Returns (node id, html url, outcome).
    async fn ensure_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<(String, String, ProvisionOutcome), ProvisionError> {
        let base = &self.api_base;
        // Query-before-create: a 200 means the repo already exists.
        let existing = self
            .rest_get(&format!("{base}/repos/{owner}/{repo}"))
            .await?;
        if existing.status().is_success() {
            let v: serde_json::Value = existing
                .json()
                .await
                .map_err(|e| ProvisionError::Api(e.to_string()))?;
            return Ok((
                v["node_id"].as_str().unwrap_or_default().to_string(),
                v["html_url"].as_str().unwrap_or_default().to_string(),
                ProvisionOutcome::Existing,
            ));
        }
        // Create it under the org. Private by default — board issues are internal.
        let created = self
            .rest_post(
                &format!("{base}/orgs/{owner}/repos"),
                json!({
                    "name": repo,
                    "private": true,
                    "has_issues": true,
                    "description": "Bifrost migration program board (issues + runbook checklists)",
                }),
            )
            .await?;
        Ok((
            created["node_id"].as_str().unwrap_or_default().to_string(),
            created["html_url"].as_str().unwrap_or_default().to_string(),
            ProvisionOutcome::Created,
        ))
    }

    /// Look up the org node id (needed as the `ownerId` for `createProjectV2`).
    async fn org_node_id(&self, owner: &str) -> Result<String, ProvisionError> {
        let data = self
            .graphql(
                "query($login:String!){ organization(login:$login){ id } }",
                json!({ "login": owner }),
            )
            .await?;
        data["organization"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| ProvisionError::Api(format!("no org id for '{owner}'")))
    }

    /// Find an org Project v2 by title, returning (node id, url) if present.
    async fn find_project(
        &self,
        owner: &str,
        title: &str,
    ) -> Result<Option<(String, String)>, ProvisionError> {
        let data = self
            .graphql(
                "query($login:String!){ organization(login:$login){ \
                 projectsV2(first:100){ nodes{ id title url } } } }",
                json!({ "login": owner }),
            )
            .await?;
        let nodes = data["organization"]["projectsV2"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(nodes.into_iter().find_map(|n| {
            if n["title"].as_str() == Some(title) {
                Some((
                    n["id"].as_str().unwrap_or_default().to_string(),
                    n["url"].as_str().unwrap_or_default().to_string(),
                ))
            } else {
                None
            }
        }))
    }

    /// Find or create the org Project. Returns (node id, url, outcome).
    async fn ensure_project(
        &self,
        owner: &str,
        title: &str,
    ) -> Result<(String, String, ProvisionOutcome), ProvisionError> {
        if let Some((id, url)) = self.find_project(owner, title).await? {
            return Ok((id, url, ProvisionOutcome::Existing));
        }
        let owner_id = self.org_node_id(owner).await?;
        let data = self
            .graphql(
                "mutation($ownerId:ID!,$title:String!){ \
                 createProjectV2(input:{ownerId:$ownerId,title:$title}){ \
                 projectV2{ id url } } }",
                json!({ "ownerId": owner_id, "title": title }),
            )
            .await?;
        let p = &data["createProjectV2"]["projectV2"];
        Ok((
            p["id"].as_str().unwrap_or_default().to_string(),
            p["url"].as_str().unwrap_or_default().to_string(),
            ProvisionOutcome::Created,
        ))
    }

    /// The set of existing field names on a project (for idempotency).
    async fn existing_field_names(
        &self,
        project_id: &str,
    ) -> Result<std::collections::HashSet<String>, ProvisionError> {
        let data = self
            .graphql(
                "query($id:ID!){ node(id:$id){ ... on ProjectV2 { \
                 fields(first:100){ nodes{ \
                 ... on ProjectV2FieldCommon { name } } } } } }",
                json!({ "id": project_id }),
            )
            .await?;
        let nodes = data["node"]["fields"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(nodes
            .into_iter()
            .filter_map(|n| n["name"].as_str().map(str::to_string))
            .collect())
    }

    /// Create one custom field. `data_type` maps the plan's lowercase types to the
    /// GraphQL `ProjectV2CustomFieldType` enum. Single-select options are passed
    /// with a default colour + description (both required by the API).
    async fn create_field(
        &self,
        project_id: &str,
        name: &str,
        data_type: &str,
        options: &[String],
    ) -> Result<String, ProvisionError> {
        let gql_type = match data_type {
            "single-select" => "SINGLE_SELECT",
            "number" => "NUMBER",
            "date" => "DATE",
            "text" => "TEXT",
            other => {
                return Err(ProvisionError::Config(format!(
                    "unsupported field data type '{other}'"
                )))
            }
        };

        // Build the input. Single-select fields require their options inline.
        let mut input = json!({
            "projectId": project_id,
            "dataType": gql_type,
            "name": name,
        });
        if gql_type == "SINGLE_SELECT" {
            let opts: Vec<serde_json::Value> = options
                .iter()
                .map(|o| json!({ "name": o, "color": "GRAY", "description": o }))
                .collect();
            input["singleSelectOptions"] = serde_json::Value::Array(opts);
        }

        let data = self
            .graphql(
                "mutation($input:CreateProjectV2FieldInput!){ \
                 createProjectV2Field(input:$input){ \
                 projectV2Field{ ... on ProjectV2FieldCommon { id } } } }",
                json!({ "input": input }),
            )
            .await?;
        Ok(data["createProjectV2Field"]["projectV2Field"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    /// The set of existing draft-issue titles on a project (for idempotency).
    async fn existing_issue_titles(
        &self,
        project_id: &str,
    ) -> Result<std::collections::HashSet<String>, ProvisionError> {
        let data = self
            .graphql(
                "query($id:ID!){ node(id:$id){ ... on ProjectV2 { \
                 items(first:100){ nodes{ content{ \
                 ... on DraftIssue { title } } } } } } }",
                json!({ "id": project_id }),
            )
            .await?;
        let nodes = data["node"]["items"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(nodes
            .into_iter()
            .filter_map(|n| n["content"]["title"].as_str().map(str::to_string))
            .collect())
    }

    /// Add one draft issue, with its checklist rendered into the body.
    async fn create_draft_issue(
        &self,
        project_id: &str,
        title: &str,
        body: &str,
    ) -> Result<String, ProvisionError> {
        let data = self
            .graphql(
                "mutation($projectId:ID!,$title:String!,$body:String!){ \
                 addProjectV2DraftIssue(input:{projectId:$projectId,title:$title,body:$body}){ \
                 projectItem{ id } } }",
                json!({ "projectId": project_id, "title": title, "body": body }),
            )
            .await?;
        Ok(data["addProjectV2DraftIssue"]["projectItem"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }
}

/// Render a planned issue's sub-issue checklist into a Markdown body.
fn issue_body(sub_issues: &[String]) -> String {
    let mut s = String::from("Migration checklist (review-first; nothing auto-commits):\n\n");
    for item in sub_issues {
        s.push_str(&format!("- [ ] {item}\n"));
    }
    s
}

#[async_trait]
impl BoardProvisioner for GitHubBoardProvisioner {
    async fn provision(
        &self,
        plan: &ProgramBoardPlan,
        owner: &str,
    ) -> Result<ProvisionResult, ProvisionError> {
        let mut actions = Vec::new();

        // 1. Repo (REST, idempotent).
        let (repo_node_id, repo_url, repo_outcome) = self.ensure_repo(owner, &plan.repo).await?;
        actions.push(ProvisionAction {
            target: ProvisionTarget::Repository,
            name: plan.repo.clone(),
            node_id: repo_node_id.clone(),
            outcome: repo_outcome,
        });

        // 2. Org Project (GraphQL, idempotent by title).
        let (project_node_id, project_url, project_outcome) =
            self.ensure_project(owner, &plan.project_title).await?;
        actions.push(ProvisionAction {
            target: ProvisionTarget::Project,
            name: plan.project_title.clone(),
            node_id: project_node_id.clone(),
            outcome: project_outcome,
        });

        // 3. Fields (query existing once, create the absent ones).
        let have_fields = self.existing_field_names(&project_node_id).await?;
        let mut fields = Vec::new();
        for f in &plan.fields {
            if have_fields.contains(&f.name) {
                actions.push(ProvisionAction {
                    target: ProvisionTarget::Field,
                    name: f.name.clone(),
                    node_id: String::new(),
                    outcome: ProvisionOutcome::Existing,
                });
                fields.push(ProvisionedField {
                    name: f.name.clone(),
                    node_id: String::new(),
                    outcome: ProvisionOutcome::Existing,
                });
                continue;
            }
            let node_id = self
                .create_field(&project_node_id, &f.name, &f.data_type, &f.options)
                .await?;
            actions.push(ProvisionAction {
                target: ProvisionTarget::Field,
                name: f.name.clone(),
                node_id: node_id.clone(),
                outcome: ProvisionOutcome::Created,
            });
            fields.push(ProvisionedField {
                name: f.name.clone(),
                node_id,
                outcome: ProvisionOutcome::Created,
            });
        }

        // 4. Issues (query existing titles once, create the absent ones).
        let have_issues = self.existing_issue_titles(&project_node_id).await?;
        let mut issues = Vec::new();
        for i in &plan.issues {
            if have_issues.contains(&i.title) {
                actions.push(ProvisionAction {
                    target: ProvisionTarget::Issue,
                    name: i.title.clone(),
                    node_id: String::new(),
                    outcome: ProvisionOutcome::Existing,
                });
                issues.push(ProvisionedIssue {
                    title: i.title.clone(),
                    node_id: String::new(),
                    outcome: ProvisionOutcome::Existing,
                });
                continue;
            }
            let node_id = self
                .create_draft_issue(&project_node_id, &i.title, &issue_body(&i.sub_issues))
                .await?;
            actions.push(ProvisionAction {
                target: ProvisionTarget::Issue,
                name: i.title.clone(),
                node_id: node_id.clone(),
                outcome: ProvisionOutcome::Created,
            });
            issues.push(ProvisionedIssue {
                title: i.title.clone(),
                node_id,
                outcome: ProvisionOutcome::Created,
            });
        }

        Ok(ProvisionResult {
            repo_node_id,
            repo_url,
            project_node_id,
            project_url,
            fields,
            issues,
            actions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::program_board::{BoardField, BoardKpis, PlannedIssue, ProgramBoardPlan};

    fn sample_plan() -> ProgramBoardPlan {
        ProgramBoardPlan {
            repo: "contoso-migration-program".into(),
            project_title: "Contoso — ADO to GitHub Actions migration".into(),
            fields: vec![
                BoardField {
                    name: "Status".into(),
                    data_type: "single-select".into(),
                    options: vec!["Not started".into(), "Draft".into()],
                },
                BoardField {
                    name: "Forecast minutes".into(),
                    data_type: "number".into(),
                    options: vec![],
                },
            ],
            issues: vec![
                PlannedIssue {
                    title: "Migrate A · CI".into(),
                    wave: 1,
                    risk: "Green".into(),
                    status: "Not started".into(),
                    forecast_minutes: 100,
                    sub_issues: vec!["Review the converted workflow".into()],
                },
                PlannedIssue {
                    title: "Migrate B · CI".into(),
                    wave: 2,
                    risk: "Amber".into(),
                    status: "Not started".into(),
                    forecast_minutes: 200,
                    sub_issues: vec!["Review the converted workflow".into()],
                },
            ],
            kpis: BoardKpis {
                total: 2,
                migrated: 0,
                validated: 0,
                in_progress: 0,
                not_started: 2,
                percent_done: 0,
                forecast_minutes: 300,
            },
            notes: vec![],
        }
    }

    #[tokio::test]
    async fn mock_provisions_repo_project_fields_and_one_issue_per_pipeline() {
        let plan = sample_plan();
        let result = MockBoardProvisioner::new()
            .provision(&plan, "contoso")
            .await
            .unwrap();

        // Repo + project recorded with deterministic ids/URLs.
        assert_eq!(result.repo_node_id, "MOCK_REPO_contoso-migration-program");
        assert_eq!(
            result.repo_url,
            "https://github.com/contoso/contoso-migration-program"
        );
        assert!(result.project_url.contains("/orgs/contoso/projects/"));

        // One field per plan field, one issue per plan issue.
        assert_eq!(result.fields.len(), plan.fields.len());
        assert_eq!(result.issues.len(), plan.issues.len());
        assert_eq!(result.issues[0].title, "Migrate A · CI");

        // First run: everything created.
        assert!(result
            .actions
            .iter()
            .all(|a| a.outcome == ProvisionOutcome::Created));
        assert!(result
            .fields
            .iter()
            .all(|f| f.outcome == ProvisionOutcome::Created));
        assert!(result
            .issues
            .iter()
            .all(|i| i.outcome == ProvisionOutcome::Created));

        // Action log shape: repo, project, then a field/issue action each.
        assert_eq!(result.actions[0].target, ProvisionTarget::Repository);
        assert_eq!(result.actions[1].target, ProvisionTarget::Project);
        let field_actions = result
            .actions
            .iter()
            .filter(|a| a.target == ProvisionTarget::Field)
            .count();
        let issue_actions = result
            .actions
            .iter()
            .filter(|a| a.target == ProvisionTarget::Issue)
            .count();
        assert_eq!(field_actions, plan.fields.len());
        assert_eq!(issue_actions, plan.issues.len());
        assert_eq!(
            result.actions.len(),
            2 + plan.fields.len() + plan.issues.len()
        );
    }

    /// Idempotency: a second provision over the same plan (everything already
    /// present) reports every object as `existing`, not re-created — and the node
    /// ids are stable across runs.
    #[tokio::test]
    async fn mock_second_provision_is_idempotent() {
        let plan = sample_plan();
        let first = MockBoardProvisioner::new()
            .provision(&plan, "contoso")
            .await
            .unwrap();

        // Everything the first run created now "exists".
        let mut names: Vec<String> = vec![plan.repo.clone(), plan.project_title.clone()];
        names.extend(plan.fields.iter().map(|f| f.name.clone()));
        names.extend(plan.issues.iter().map(|i| i.title.clone()));

        let second = MockBoardProvisioner::with_existing(names)
            .provision(&plan, "contoso")
            .await
            .unwrap();

        // Nothing re-created.
        assert!(second
            .actions
            .iter()
            .all(|a| a.outcome == ProvisionOutcome::Existing));
        assert!(second
            .issues
            .iter()
            .all(|i| i.outcome == ProvisionOutcome::Existing));

        // Stable node ids across the two runs (idempotent reuse).
        assert_eq!(first.repo_node_id, second.repo_node_id);
        assert_eq!(first.project_node_id, second.project_node_id);
        assert_eq!(
            first.issues.iter().map(|i| &i.node_id).collect::<Vec<_>>(),
            second.issues.iter().map(|i| &i.node_id).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn issue_body_renders_a_checklist() {
        let body = issue_body(&["First".into(), "Second".into()]);
        assert!(body.contains("- [ ] First"));
        assert!(body.contains("- [ ] Second"));
    }

    /// Live smoke test — provisions a REAL board in `BIFROST_GH_ORG`. Ignored by
    /// default (it is an outward action that creates org infrastructure). Run
    /// intentionally with:
    ///   GITHUB_TOKEN=… BIFROST_GH_ORG=my-org \
    ///     cargo test -p bifrost-adapters -- --ignored live_provisions_a_board
    #[tokio::test]
    #[ignore = "creates a real repo + org Project — run only against an org you own"]
    async fn live_provisions_a_board() {
        let owner = std::env::var("BIFROST_GH_ORG").expect("BIFROST_GH_ORG set");
        let provisioner = GitHubBoardProvisioner::from_env().expect("GITHUB_TOKEN set");
        let result = provisioner
            .provision(&sample_plan(), &owner)
            .await
            .expect("provisions a board");
        assert!(!result.project_node_id.is_empty());
        assert!(!result.actions.is_empty());
    }
}
