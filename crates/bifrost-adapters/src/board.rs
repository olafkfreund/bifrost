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

/// Whether the step created the object, found it already present, or updated an
/// existing object's value (idempotency/sync is visible in the action log and the
/// result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisionOutcome {
    Created,
    Existing,
    /// A field value on an existing item was set/synced (e.g. a lifecycle status
    /// sync, #267) — not a create, not a no-op.
    Updated,
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

/// One single-select option on a provisioned field — the option name plus its
/// node id. Needed to sync a value (#267): `updateProjectV2ItemFieldValue` takes
/// the option *id*, not its name, so we record the name→id map here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedOption {
    pub name: String,
    pub node_id: String,
}

/// A provisioned (or found) custom field. For single-select fields, `options`
/// carries each option's name→id mapping so a value can later be synced by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedField {
    pub name: String,
    pub node_id: String,
    pub outcome: ProvisionOutcome,
    /// Single-select option ids (empty for non-select fields).
    #[serde(default)]
    pub options: Vec<ProvisionedOption>,
}

impl ProvisionResult {
    /// Look up the board context needed to sync a single status field value:
    /// `(project node id, Status field node id, option node id)` for the issue
    /// titled `issue_title` and the single-select option named `status_name`.
    ///
    /// Returns `None` when there is nothing to sync to — no matching issue, no
    /// Status field, or no option for that status name — so the caller treats a
    /// missing board as a clean no-op rather than an error (#267).
    pub fn status_sync_target(
        &self,
        issue_title: &str,
        status_name: &str,
    ) -> Option<StatusSyncTarget> {
        let item = self.issues.iter().find(|i| i.title == issue_title)?;
        let field = self.fields.iter().find(|f| f.name == "Status")?;
        let option = field.options.iter().find(|o| o.name == status_name)?;
        Some(StatusSyncTarget {
            project_node_id: self.project_node_id.clone(),
            item_node_id: item.node_id.clone(),
            field_node_id: field.node_id.clone(),
            option_node_id: option.node_id.clone(),
        })
    }
}

/// The resolved ids a [`BoardProvisioner::sync_issue_status`] call needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusSyncTarget {
    pub project_node_id: String,
    pub item_node_id: String,
    pub field_node_id: String,
    pub option_node_id: String,
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

    /// Sync one issue's Status single-select to `status_name` by setting the
    /// resolved option on its project item (#267). Idempotent from the caller's
    /// view: it always issues the set (the GraphQL mutation is itself idempotent).
    /// Returns the [`ProvisionAction`] (outcome [`ProvisionOutcome::Updated`]) so
    /// the caller can append it to the attestation log. The mock records this
    /// without any network I/O; the real impl issues `updateProjectV2ItemFieldValue`.
    async fn sync_issue_status(
        &self,
        target: &StatusSyncTarget,
        status_name: &str,
    ) -> Result<ProvisionAction, ProvisionError>;
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
                // Single-select options get deterministic synthetic ids so a later
                // status sync can resolve an option by name offline (#267).
                let options = f
                    .options
                    .iter()
                    .map(|o| ProvisionedOption {
                        name: o.clone(),
                        node_id: mock_node_id("OPTION", &format!("{}-{o}", f.name)),
                    })
                    .collect();
                ProvisionedField {
                    name: f.name.clone(),
                    node_id,
                    outcome,
                    options,
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

    async fn sync_issue_status(
        &self,
        target: &StatusSyncTarget,
        status_name: &str,
    ) -> Result<ProvisionAction, ProvisionError> {
        // No network: just record the sync as an Updated action on the item.
        Ok(ProvisionAction {
            target: ProvisionTarget::Issue,
            name: format!("Status → {status_name}"),
            node_id: target.item_node_id.clone(),
            outcome: ProvisionOutcome::Updated,
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

    /// The existing fields on a project, as a name → (field id, single-select
    /// options) map (for idempotency and for resolving an existing Status field's
    /// option ids so a sync can target the right option, #267).
    async fn existing_fields(
        &self,
        project_id: &str,
    ) -> Result<std::collections::HashMap<String, (String, Vec<ProvisionedOption>)>, ProvisionError>
    {
        let data = self
            .graphql(
                "query($id:ID!){ node(id:$id){ ... on ProjectV2 { \
                 fields(first:100){ nodes{ \
                 ... on ProjectV2FieldCommon { id name } \
                 ... on ProjectV2SingleSelectField { options{ id name } } } } } } }",
                json!({ "id": project_id }),
            )
            .await?;
        let nodes = data["node"]["fields"]["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut out = std::collections::HashMap::new();
        for n in nodes {
            let Some(name) = n["name"].as_str() else {
                continue;
            };
            let id = n["id"].as_str().unwrap_or_default().to_string();
            let options = parse_options(&n["options"]);
            out.insert(name.to_string(), (id, options));
        }
        Ok(out)
    }

    /// Create one custom field. `data_type` maps the plan's lowercase types to the
    /// GraphQL `ProjectV2CustomFieldType` enum. Single-select options are passed
    /// with a default colour + description (both required by the API). Returns the
    /// field id plus, for single-select fields, the created options' name→id map.
    async fn create_field(
        &self,
        project_id: &str,
        name: &str,
        data_type: &str,
        options: &[String],
    ) -> Result<(String, Vec<ProvisionedOption>), ProvisionError> {
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
                 projectV2Field{ \
                 ... on ProjectV2FieldCommon { id } \
                 ... on ProjectV2SingleSelectField { options{ id name } } } } }",
                json!({ "input": input }),
            )
            .await?;
        let field = &data["createProjectV2Field"]["projectV2Field"];
        let id = field["id"].as_str().unwrap_or_default().to_string();
        Ok((id, parse_options(&field["options"])))
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

        // 3. Fields (query existing once, create the absent ones). The query also
        // returns each existing single-select field's option ids, so an existing
        // Status field can still be synced to later (#267).
        let have_fields = self.existing_fields(&project_node_id).await?;
        let mut fields = Vec::new();
        for f in &plan.fields {
            if let Some((node_id, options)) = have_fields.get(&f.name) {
                actions.push(ProvisionAction {
                    target: ProvisionTarget::Field,
                    name: f.name.clone(),
                    node_id: node_id.clone(),
                    outcome: ProvisionOutcome::Existing,
                });
                fields.push(ProvisionedField {
                    name: f.name.clone(),
                    node_id: node_id.clone(),
                    outcome: ProvisionOutcome::Existing,
                    options: options.clone(),
                });
                continue;
            }
            let (node_id, options) = self
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
                options,
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

    async fn sync_issue_status(
        &self,
        target: &StatusSyncTarget,
        status_name: &str,
    ) -> Result<ProvisionAction, ProvisionError> {
        // Set the single-select value on the item: updateProjectV2ItemFieldValue
        // takes { projectId, itemId, fieldId, value: { singleSelectOptionId } }.
        self.graphql(
            "mutation($projectId:ID!,$itemId:ID!,$fieldId:ID!,$optionId:String!){ \
             updateProjectV2ItemFieldValue(input:{ \
             projectId:$projectId,itemId:$itemId,fieldId:$fieldId, \
             value:{ singleSelectOptionId:$optionId } }){ \
             projectV2Item{ id } } }",
            json!({
                "projectId": target.project_node_id,
                "itemId": target.item_node_id,
                "fieldId": target.field_node_id,
                "optionId": target.option_node_id,
            }),
        )
        .await?;
        Ok(ProvisionAction {
            target: ProvisionTarget::Issue,
            name: format!("Status → {status_name}"),
            node_id: target.item_node_id.clone(),
            outcome: ProvisionOutcome::Updated,
        })
    }
}

/// Parse a GraphQL `options { id name }` array into the option name→id map.
fn parse_options(value: &serde_json::Value) -> Vec<ProvisionedOption> {
    value
        .as_array()
        .map(|opts| {
            opts.iter()
                .filter_map(|o| {
                    Some(ProvisionedOption {
                        name: o["name"].as_str()?.to_string(),
                        node_id: o["id"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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
                    options: vec!["Not started".into(), "Draft".into(), "In review".into()],
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

    /// The mock captures Status option ids so a status sync can resolve them
    /// offline, and `status_sync_target` returns the matching item/field/option.
    #[tokio::test]
    async fn mock_captures_status_options_and_resolves_a_sync_target() {
        let plan = sample_plan();
        let result = MockBoardProvisioner::new()
            .provision(&plan, "contoso")
            .await
            .unwrap();

        // The Status field carries an option id per plan option.
        let status = result.fields.iter().find(|f| f.name == "Status").unwrap();
        assert_eq!(status.options.len(), 3);
        assert!(status.options.iter().any(|o| o.name == "In review"));

        // Resolve a target for an existing issue + a real status name.
        let target = result
            .status_sync_target("Migrate A · CI", "In review")
            .expect("a sync target");
        assert_eq!(target.project_node_id, result.project_node_id);
        assert_eq!(target.item_node_id, result.issues[0].node_id);
        assert_eq!(target.field_node_id, status.node_id);
        assert_eq!(
            target.option_node_id,
            status
                .options
                .iter()
                .find(|o| o.name == "In review")
                .unwrap()
                .node_id
        );
    }

    /// No matching issue / unknown status option => None (a clean no-op upstream).
    #[tokio::test]
    async fn status_sync_target_is_none_when_unresolvable() {
        let plan = sample_plan();
        let result = MockBoardProvisioner::new()
            .provision(&plan, "contoso")
            .await
            .unwrap();
        assert!(result
            .status_sync_target("No such issue", "In review")
            .is_none());
        assert!(result
            .status_sync_target("Migrate A · CI", "No such status")
            .is_none());
    }

    /// The mock sync records an `Updated` action on the item without any network.
    #[tokio::test]
    async fn mock_sync_issue_status_records_an_updated_action() {
        let plan = sample_plan();
        let provisioner = MockBoardProvisioner::new();
        let result = provisioner.provision(&plan, "contoso").await.unwrap();
        let target = result
            .status_sync_target("Migrate A · CI", "In review")
            .unwrap();

        let action = provisioner
            .sync_issue_status(&target, "In review")
            .await
            .unwrap();
        assert_eq!(action.outcome, ProvisionOutcome::Updated);
        assert_eq!(action.target, ProvisionTarget::Issue);
        assert_eq!(action.node_id, result.issues[0].node_id);
        assert!(action.name.contains("In review"));
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
