//! Bifrost control-plane API server.
//!
//! Serves the portfolio the portal renders. The source is resolved once at
//! startup (and on `POST /api/refresh`), in priority order:
//!   1. live audit of `BIFROST_PROJECT` (ADO REST + Docker Importer), if creds present
//!   2. a portfolio JSON file named by `BIFROST_PORTFOLIO`
//!   3. the built-in sample
//!
//! Any failure falls back to the next source, so the server always starts.

mod auth;
mod jobs;
mod sample;
mod secrets;
mod store;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{
    routing::{get, patch, post},
    Extension, Json, Router,
};
use bifrost_adapters::{
    convert_pipeline, declared_outputs, github_token_from_env, AzureDevOpsBaseline,
    BaselineRequest, BaselineSource, CommitRequest, ConversionOutcome, GitHubPublisher,
    GitHubRunCollector, GitHubSandboxTrigger, MockBaselineSource, MockImporter, MockPublisher,
    MockRunCollector, MockSandboxTrigger, Publisher, RunCollector, RunQuery, SandboxTrigger,
    TriggerRequest,
};
use bifrost_core::{
    compare_parity, Attestation, AuditLog, AuditPack, Classification, Connection, ConnectionKind,
    Identity, MigrationAttestation, ParityReport, Portfolio, ProposalStatus, Role, RunFacts,
    SecretRef,
};
use bifrost_llm::{MockLlmProvider, Router as LlmRouter, RoutingPolicy};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use store::{ProposalStore, StoredProposal};

/// Shared server state: the portfolio, the proposal store, and the job registry.
struct AppState {
    portfolio: RwLock<Portfolio>,
    store: Arc<dyn ProposalStore>,
    jobs: jobs::JobRegistry,
    next_job: AtomicU64,
    /// Resolves the acting identity from a bearer token (#65).
    auth: Arc<dyn auth::Authenticator>,
    /// Whether a valid token is required on `/api/*` (else open / local admin).
    auth_enabled: bool,
    /// Resolves connection secret references at use-time (#154; consumed by the
    /// per-connection multi-org audit in #156).
    secrets: Arc<dyn secrets::SecretResolver>,
}

type Shared = Arc<AppState>;

/// Map a store/persistence error to a 500 response.
fn internal(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// The proposal id the conversion loop assigns for a pipeline (see `run_conversion`).
fn proposal_id_for(pipeline_id: &str) -> String {
    format!("prop-{pipeline_id}")
}

/// Serialize a stored proposal for the wire: `{ proposal, runbook, audit }`.
fn record_json(rec: &StoredProposal) -> Value {
    json!({
        "proposal": rec.proposal,
        "runbook": rec.runbook,
        "audit": rec.audit.events(),
    })
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "bifrost-api" }))
}

async fn portfolio(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
) -> Json<Portfolio> {
    let mut portfolio = state.portfolio.read().await.clone();
    // Overlay live review state so the portal's review queue reflects actions
    // taken this session: a converted pipeline shows its current proposal status,
    // and the latest audit event names who last acted and when. Only the caller's
    // own tenant's proposals are overlaid (#66).
    if let Ok(all) = state.store.list().await {
        let by_id: HashMap<String, &StoredProposal> = all
            .iter()
            .filter(|r| r.tenant == caller.tenant)
            .map(|r| (r.proposal.id.clone(), r))
            .collect();
        for p in &mut portfolio.pipelines {
            if let Some(rec) = by_id.get(&proposal_id_for(&p.id)) {
                p.status = rec.proposal.status;
                if let Some(last) = rec.audit.events().last() {
                    p.reviewer = Some(last.actor.clone());
                    p.reviewed_at = Some(last.at.clone());
                }
            }
        }
    }
    Json(portfolio)
}

/// Re-resolve the portfolio and update the cache. When the caller's tenant has
/// **ADO connections** (#154), audit across all of them and merge into one
/// org-tagged portfolio (#156); otherwise fall back to the single-org env path.
async fn refresh(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
) -> Json<Portfolio> {
    let fresh = match build_from_connections(&state, &caller.tenant).await {
        Some(p) => p,
        None => resolve_portfolio().await,
    };
    *state.portfolio.write().await = fresh.clone();
    Json(fresh)
}

/// Resolve an ADO connection's `(org_url, pat)` — the PAT via the secret resolver.
/// `None` for a non-ADO connection or an unresolvable secret.
async fn ado_inputs(
    conn: &Connection,
    resolver: &dyn secrets::SecretResolver,
) -> Option<(String, String)> {
    let ConnectionKind::AzureDevOps { org_url, auth } = &conn.kind else {
        return None;
    };
    let pat = resolver.resolve(auth).await.ok()?;
    Some((org_url.clone(), pat))
}

/// Build a tenant-wide, org-tagged portfolio by auditing every ADO connection the
/// tenant owns and merging the results (#156). Live (Docker + ADO); returns `None`
/// when the tenant has no ADO connections or none could be audited, so the caller
/// falls back to the single-org path.
async fn build_from_connections(state: &AppState, tenant: &str) -> Option<Portfolio> {
    use bifrost_adapters::docker_importer::org_from_url;
    use bifrost_adapters::{
        audit_portfolio, merge_portfolios, AuditConfig, AzureDevOpsAdapter, DockerImporter,
        Importer, SourceAdapter,
    };

    let conns = state.store.list_connections(tenant).await.ok()?;
    let air_gap = std::env::var("BIFROST_AIR_GAP")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);

    let mut portfolios = Vec::new();
    for conn in &conns {
        let Some((org_url, pat)) = ado_inputs(conn, state.secrets.as_ref()).await else {
            continue;
        };
        let org = org_from_url(&org_url).to_string();
        // Pick a project to drive the Importer (org-level audit; per-project
        // precision is #31). Enumerate is across the whole org via the adapter.
        let probe = AzureDevOpsAdapter::new(&org_url, "", &pat);
        let Some(project) = probe
            .discover()
            .await
            .ok()
            .and_then(|ps| ps.into_iter().next())
        else {
            continue;
        };
        let adapter = AzureDevOpsAdapter::new(&org_url, &project.name, &pat);
        let importer = DockerImporter::new(&org, &project.name);
        let config = AuditConfig {
            org: org.clone(),
            importer_version: importer
                .version()
                .await
                .unwrap_or_else(|_| "unknown".into()),
            importer_image_digest: importer.image_digest().await.unwrap_or_default(),
            ado2gh_version: "n/a".into(),
            air_gap,
            generated_at: now_iso8601(),
        };
        if let Ok(p) = audit_portfolio(&adapter, &importer, config).await {
            portfolios.push(p);
        }
    }
    if portfolios.is_empty() {
        return None;
    }
    Some(merge_portfolios(portfolios, now_iso8601(), air_gap))
}

/// Convert one pipeline into a proposal (+ runbook), storing it for review.
///
/// Idempotent: a pipeline already converted returns its stored record (with any
/// edits/transitions intact) rather than reconverting, so review state survives
/// re-opening the panel. Returns `{ proposal, runbook, audit }`.
async fn convert(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let proposal_id = proposal_id_for(&id);

    if let Some(rec) = state.store.get(&proposal_id).await.map_err(internal)? {
        return Ok(Json(record_json(&rec)));
    }

    // The pipeline's ADO project (for the live Docker importer), looked up in the
    // portfolio; falls back to BIFROST_PROJECT inside run_conversion.
    let project = state
        .portfolio
        .read()
        .await
        .pipelines
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.project.clone());
    let outcome = run_conversion(&id, project.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let rec = StoredProposal {
        proposal: outcome.proposal,
        runbook: outcome.runbook,
        audit: AuditLog::new(),
        // Owned by the caller's tenant (#66).
        tenant: caller.tenant,
    };
    state.store.put(&rec).await.map_err(internal)?;
    Ok(Json(record_json(&rec)))
}

/// Body of `POST /api/proposals/:id/transition`: the target lifecycle state and
/// the acting identity (placeholder until auth — #65).
#[derive(Deserialize)]
struct TransitionBody {
    to: ProposalStatus,
    #[serde(default)]
    actor: Option<String>,
}

/// Move a proposal through the lifecycle state machine, recording the audit event.
///
/// 404 if the proposal is unknown; 409 if the edge is illegal (the state machine
/// rejects it and nothing is logged).
async fn transition(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<TransitionBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    // Gate the terminal state on the manual-task tracker (#57): a migration is
    // not "done" until every required runbook task is resolved.
    if body.to == ProposalStatus::Validated {
        let remaining = rec.runbook.required_remaining();
        if remaining > 0 {
            return Err((
                StatusCode::CONFLICT,
                format!("{remaining} required manual task(s) still open — resolve them before validating"),
            ));
        }
    }

    let actor = body.actor.unwrap_or_else(|| "reviewer@portal".into());
    rec.proposal
        .transition(body.to, actor, now_iso8601(), &mut rec.audit)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    state.store.put(&rec).await.map_err(internal)?;
    Ok(Json(record_json(&rec)))
}

/// Body of `PATCH /api/proposals/:id/runbook`: mark a manual task done/undone.
#[derive(Deserialize)]
struct RunbookItemBody {
    index: usize,
    done: bool,
}

/// Toggle a runbook item's completion (#57). 404 for an unknown proposal or
/// out-of-range index.
async fn set_runbook_item(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<RunbookItemBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;
    let item = rec.runbook.items.get_mut(body.index).ok_or((
        StatusCode::NOT_FOUND,
        format!("no runbook item {}", body.index),
    ))?;
    item.done = body.done;
    state.store.put(&rec).await.map_err(internal)?;
    Ok(Json(record_json(&rec)))
}

/// Body of `PATCH /api/proposals/:id`: the reviewer's edited workflow.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditBody {
    proposed_yaml: String,
    #[serde(default)]
    actor: Option<String>,
}

/// Replace a proposal's workflow with a reviewer edit, recording it.
///
/// 404 if unknown; 409 if the proposal is past approval (frozen).
async fn edit(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<EditBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;
    let actor = body.actor.unwrap_or_else(|| "reviewer@portal".into());
    rec.proposal
        .record_edit(body.proposed_yaml, actor, now_iso8601(), &mut rec.audit)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    state.store.put(&rec).await.map_err(internal)?;
    Ok(Json(record_json(&rec)))
}

/// A filesystem-safe slug for a pipeline id (branch + workflow filename).
fn slugify(s: &str) -> String {
    let slug: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    slug.trim_matches('-').to_string()
}

/// PR body linking the proposal + the manual-task checklist (#56/#57).
fn pr_body(rec: &StoredProposal) -> String {
    let p = &rec.proposal;
    let mut s = format!(
        "## Bifrost-converted workflow\n\nConverted from the Azure DevOps pipeline `{}` and reviewed in Bifrost.\n\n- **Risk:** {:?} ({})\n- **Proposal:** {}\n",
        p.pipeline_id, p.risk_band, p.risk_score, p.id
    );
    if !rec.runbook.items.is_empty() {
        s.push_str("\n### Manual tasks (complete before this is production-ready)\n");
        for item in &rec.runbook.items {
            s.push_str(&format!("- [ ] {} — {}\n", item.title, item.detail));
        }
    }
    s.push_str("\n🤖 Converted with Bifrost — review-first; auto-commit is opt-in.\n");
    s
}

/// Whether a live-path env flag is enabled.
fn live_enabled(var: &str) -> bool {
    std::env::var(var)
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Resolve a GitHub token for a live path, gated on `live_var`. Prefers a GitHub
/// App installation token (least privilege, #64) when `GITHUB_APP_*` is set, else
/// `GITHUB_TOKEN`. `None` (with a warning) means stay on the mock — a live GitHub
/// call never fires without real auth.
async fn github_token(live_var: &str) -> Option<String> {
    if !live_enabled(live_var) {
        return None;
    }
    match github_token_from_env().await {
        Ok(Some(t)) => Some(t),
        Ok(None) => {
            tracing::warn!(
                "{live_var} set but no GitHub auth (GITHUB_APP_* or GITHUB_TOKEN); using mock"
            );
            None
        }
        Err(e) => {
            tracing::warn!("{live_var} set but GitHub auth failed: {e}; using mock");
            None
        }
    }
}

/// The publisher for the commit path: the real GitHub one when the live commit
/// path is explicitly enabled and authenticated, else the offline mock (never a
/// silent write to a customer repo).
async fn select_publisher() -> Box<dyn Publisher> {
    if let Some(token) = github_token("BIFROST_COMMIT_LIVE").await {
        let mut p = GitHubPublisher::new(token);
        if let Ok(base) = std::env::var("GITHUB_API_BASE") {
            p = p.with_api_base(base);
        }
        return Box::new(p);
    }
    Box::new(MockPublisher)
}

/// Commit an approved proposal's workflow and open a PR (#56). The proposal must
/// be `Approved`; on success it moves to `Committed` (audit-logged) and carries
/// the PR URL. Opt-in: writes to a real repo only when `BIFROST_COMMIT_LIVE` is
/// set (else a mock PR URL).
async fn commit(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    if rec.proposal.status != ProposalStatus::Approved {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "proposal must be approved to commit (is {:?})",
                rec.proposal.status
            ),
        ));
    }

    let slug = slugify(&rec.proposal.pipeline_id);
    let request = CommitRequest {
        repo: std::env::var("BIFROST_GH_REPO").unwrap_or_else(|_| "example/sandbox".into()),
        branch: format!("bifrost/convert-{slug}"),
        base: std::env::var("BIFROST_GH_BASE").unwrap_or_else(|_| "main".into()),
        workflow_path: format!(".github/workflows/{slug}.yml"),
        workflow_yaml: rec.proposal.proposed_yaml.clone(),
        title: format!("Bifrost: convert {}", rec.proposal.pipeline_id),
        body: pr_body(&rec),
    };

    let result = select_publisher()
        .await
        .commit_workflow(&request)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    rec.proposal.pr_url = Some(result.pr_url.clone());
    rec.proposal
        .transition(
            ProposalStatus::Committed,
            "reviewer@portal",
            now_iso8601(),
            &mut rec.audit,
        )
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    state.store.put(&rec).await.map_err(internal)?;

    Ok(Json(json!({
        "proposal": rec.proposal,
        "runbook": rec.runbook,
        "audit": rec.audit.events(),
        "prUrl": result.pr_url,
    })))
}

/// The sandbox trigger: real GitHub `workflow_dispatch` when the live validation
/// path is enabled + authenticated, else the mock (never a silent CI run).
async fn select_trigger() -> Box<dyn SandboxTrigger> {
    if let Some(token) = github_token("BIFROST_VALIDATE_LIVE").await {
        let mut t = GitHubSandboxTrigger::new(token);
        if let Ok(base) = std::env::var("GITHUB_API_BASE") {
            t = t.with_api_base(base);
        }
        return Box::new(t);
    }
    Box::new(MockSandboxTrigger)
}

/// Trigger the committed workflow in the sandbox (#58) — the first step of
/// smoke-parity. The proposal must be `Committed` (the workflow exists in the
/// repo). Opt-in: a real `workflow_dispatch` runs only when `BIFROST_VALIDATE_LIVE`
/// is set (else mock). Capturing the run + diffing the baseline are #59/#60.
async fn validate(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    if rec.proposal.status != ProposalStatus::Committed {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "proposal must be committed to validate (is {:?})",
                rec.proposal.status
            ),
        ));
    }

    let slug = slugify(&rec.proposal.pipeline_id);
    let request = TriggerRequest {
        repo: std::env::var("BIFROST_GH_REPO").unwrap_or_else(|_| "example/sandbox".into()),
        workflow_file: format!("{slug}.yml"),
        git_ref: format!("bifrost/convert-{slug}"),
    };
    let result = select_trigger()
        .await
        .trigger(&request)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    Ok(Json(json!({
        "proposal": rec.proposal,
        "trigger": {
            "repo": result.repo,
            "workflowFile": result.workflow_file,
            "gitRef": result.git_ref,
            "dispatched": result.dispatched,
        },
    })))
}

/// The run collector: reads the real GitHub Actions run when the live validation
/// path is enabled + authenticated, else the mock (never a silent external call).
async fn select_collector() -> Box<dyn RunCollector> {
    if let Some(token) = github_token("BIFROST_VALIDATE_LIVE").await {
        let mut c = GitHubRunCollector::new(token);
        if let Ok(base) = std::env::var("GITHUB_API_BASE") {
            c = c.with_api_base(base);
        }
        return Box::new(c);
    }
    Box::new(MockRunCollector)
}

/// Capture the result of the converted run (#59): status, jobs, artifacts, and the
/// outputs the workflow declares. The proposal must be `Committed` (the workflow
/// exists in the repo to have been dispatched). Reads the real GitHub run only when
/// `BIFROST_VALIDATE_LIVE` is set (else mock). Diffing against the ADO baseline is #60.
async fn run_result(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    if rec.proposal.status != ProposalStatus::Committed {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "proposal must be committed to capture a run (is {:?})",
                rec.proposal.status
            ),
        ));
    }

    let slug = slugify(&rec.proposal.pipeline_id);
    let query = RunQuery {
        repo: std::env::var("BIFROST_GH_REPO").unwrap_or_else(|_| "example/sandbox".into()),
        workflow_file: format!("{slug}.yml"),
        git_ref: format!("bifrost/convert-{slug}"),
    };
    let run = select_collector()
        .await
        .collect(&query)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let outputs = declared_outputs(&rec.proposal.proposed_yaml);

    Ok(Json(json!({
        "proposal": rec.proposal,
        "run": {
            "runId": run.run_id,
            "status": run.status,
            "conclusion": run.conclusion,
            "jobs": run.jobs.iter().map(|j| json!({
                "name": j.name,
                "conclusion": j.conclusion,
            })).collect::<Vec<_>>(),
            "artifacts": run.artifacts.iter().map(|a| json!({
                "name": a.name,
                "sizeBytes": a.size_bytes,
            })).collect::<Vec<_>>(),
        },
        "declaredOutputs": outputs,
    })))
}

/// The ADO baseline source: the real ADO REST read when the live validation path
/// is enabled + configured, else the mock (never a silent external call).
fn select_baseline() -> Box<dyn BaselineSource> {
    let live = std::env::var("BIFROST_VALIDATE_LIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if live {
        match AzureDevOpsBaseline::from_env() {
            Ok(b) => return Box::new(b),
            Err(e) => {
                tracing::warn!(
                    "BIFROST_VALIDATE_LIVE set but baseline unavailable: {e}; using mock"
                )
            }
        }
    }
    Box::new(MockBaselineSource)
}

/// Compute smoke parity for a committed proposal: capture the converted run
/// (#59), fetch the ADO baseline (#60), and diff them. Shared by the read-only
/// `parity` endpoint and the `attest` endpoint. Errors if the proposal isn't
/// `Committed`. Both external reads are opt-in behind `BIFROST_VALIDATE_LIVE`.
async fn compute_parity(
    state: &AppState,
    rec: &StoredProposal,
) -> Result<(RunFacts, RunFacts, ParityReport), (StatusCode, String)> {
    if rec.proposal.status != ProposalStatus::Committed {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "proposal must be committed to compare parity (is {:?})",
                rec.proposal.status
            ),
        ));
    }

    // Converted side: the captured GitHub run + the outputs the workflow declares.
    let slug = slugify(&rec.proposal.pipeline_id);
    let query = RunQuery {
        repo: std::env::var("BIFROST_GH_REPO").unwrap_or_else(|_| "example/sandbox".into()),
        workflow_file: format!("{slug}.yml"),
        git_ref: format!("bifrost/convert-{slug}"),
    };
    let run = select_collector()
        .await
        .collect(&query)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let converted = RunFacts {
        succeeded: run.conclusion.as_deref() == Some("success"),
        artifacts: run.artifacts.iter().map(|a| a.name.clone()).collect(),
        outputs: declared_outputs(&rec.proposal.proposed_yaml),
    };

    // Baseline side: the last successful ADO run for this pipeline. Resolve the
    // pipeline's name + ADO project from the portfolio (the ADO definition is
    // looked up by name).
    let (name, project) = {
        let portfolio = state.portfolio.read().await;
        portfolio
            .pipelines
            .iter()
            .find(|p| p.id == rec.proposal.pipeline_id)
            .map(|p| (p.name.clone(), p.project.clone()))
            .unwrap_or_else(|| (rec.proposal.pipeline_id.clone(), String::new()))
    };
    let baseline = select_baseline()
        .baseline(&BaselineRequest {
            project,
            pipeline_name: name,
        })
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;

    let report = compare_parity(&baseline, &converted);
    Ok((baseline, converted, report))
}

/// Smoke-parity report (#60): capture the converted run (#59) and diff it against
/// the last successful ADO run on three signals — success, artifact names, and
/// declared output names. Deliberately *not* full equivalence (the report carries
/// that caveat). The proposal must be `Committed`. Read-only — to record the
/// result as an attestation, POST to `/attest` (#61).
async fn parity(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;
    let (baseline, converted, report) = compute_parity(&state, &rec).await?;
    Ok(Json(json!({
        "proposal": rec.proposal,
        "baseline": baseline,
        "converted": converted,
        "parity": report,
    })))
}

/// Body of `POST /api/proposals/:id/attest`: the acting identity (placeholder
/// until auth — #65).
#[derive(Deserialize, Default)]
struct AttestBody {
    #[serde(default)]
    actor: Option<String>,
}

/// Record the smoke-parity result as an **attestation** on the proposal (#61):
/// compute parity, write the verdict + full report onto the proposal, and append
/// it to the immutable audit log — the evidence a reviewer sees before the final
/// `Committed → Validated` approval. The proposal must be `Committed`. Signing +
/// export of the attestation is #62.
async fn attest(
    State(state): State<Shared>,
    Path(id): Path<String>,
    body: Option<Json<AttestBody>>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let mut rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    let (_baseline, _converted, report) = compute_parity(&state, &rec).await?;
    let actor = body
        .and_then(|Json(b)| b.actor)
        .unwrap_or_else(|| "reviewer@portal".into());
    let attestation = Attestation {
        subject: rec.proposal.pipeline_id.clone(),
        verdict: report.verdict,
        report,
        actor,
        at: now_iso8601(),
    };
    rec.proposal
        .record_parity(attestation, &mut rec.audit)
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    state.store.put(&rec).await.map_err(internal)?;

    Ok(Json(record_json(&rec)))
}

/// The HMAC signing key + its id. From `BIFROST_SIGNING_KEY` (production); else a
/// clearly-labelled dev key so the endpoint always works offline — the `key_id`
/// tells a verifier which key signed it.
fn signing_key() -> (Vec<u8>, String) {
    match std::env::var("BIFROST_SIGNING_KEY") {
        Ok(k) if !k.is_empty() => {
            let key_id = std::env::var("BIFROST_SIGNING_KEY_ID")
                .unwrap_or_else(|_| "bifrost-configured".into());
            (k.into_bytes(), key_id)
        }
        _ => {
            tracing::warn!(
                "BIFROST_SIGNING_KEY not set — signing attestation with the dev key \
                 (key_id=bifrost-dev). Do not rely on this in production."
            );
            (b"bifrost-dev-key".to_vec(), "bifrost-dev".to_string())
        }
    }
}

/// Export the signed, verifiable migration attestation (#62): the proposal's
/// deterministic risk, every recorded decision/approval, and the smoke-parity
/// attestation, assembled into an in-toto-inspired statement and signed with
/// HMAC-SHA256. Air-gap safe — no signing service or network. Returns the signed
/// JSON document (consumers can save it as the migration's attestation record).
async fn attestation(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let rec = state
        .store
        .get(&id)
        .await
        .map_err(internal)?
        .ok_or((StatusCode::NOT_FOUND, format!("no proposal '{id}'")))?;

    let (key, key_id) = signing_key();
    let signed = MigrationAttestation::build(&rec.proposal, rec.audit.events()).sign(&key, key_id);
    serde_json::to_value(&signed)
        .map(Json)
        .map_err(|e| internal(e.into()))
}

/// Export the per-org **compliance audit pack** (#63): every migration's signed
/// attestation (who/what/why/when + parity), bundled into one tamper-evident,
/// signed artifact with a summary roll-up — the single file an auditor needs.
/// Air-gap safe. Each attestation is individually signed; the pack is signed too.
async fn audit_pack(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let all = state.store.list().await.map_err(internal)?;
    let (key, key_id) = signing_key();
    // Only the caller's own tenant's migrations go in the pack (#66).
    let signed: Vec<_> = all
        .iter()
        .filter(|rec| rec.tenant == caller.tenant)
        .map(|rec| {
            MigrationAttestation::build(&rec.proposal, rec.audit.events())
                .sign(&key, key_id.clone())
        })
        .collect();
    let pack = AuditPack::build(now_iso8601(), signed).sign(&key, key_id);
    serde_json::to_value(&pack)
        .map(Json)
        .map_err(|e| internal(e.into()))
}

// ── Connections (#154) ────────────────────────────────────────────────────────

/// How a secret is supplied when creating a connection. `inline` carries a
/// plaintext that the server **encrypts immediately** (never stored raw); every
/// other variant is a reference, so no secret value is transmitted or stored.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum SecretInput {
    EnvVar {
        name: String,
    },
    KeyVault {
        uri: String,
    },
    GitHubApp {
        installation_id: String,
    },
    EntraWif {
        tenant_id: String,
        client_id: String,
    },
    Inline {
        value: String,
    },
}

impl SecretInput {
    fn into_ref(self) -> Result<SecretRef, (StatusCode, String)> {
        Ok(match self {
            SecretInput::EnvVar { name } => SecretRef::EnvVar { name },
            SecretInput::KeyVault { uri } => SecretRef::KeyVault { uri },
            SecretInput::GitHubApp { installation_id } => SecretRef::GitHubApp { installation_id },
            SecretInput::EntraWif {
                tenant_id,
                client_id,
            } => SecretRef::EntraWif {
                tenant_id,
                client_id,
            },
            SecretInput::Inline { value } => secrets::encrypt_inline(&value)
                .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        })
    }
}

/// The create-connection body — a tagged union (`kind`) carrying the connection's
/// name + details. (A single tagged enum rather than name + flattened-enum, which
/// serde can't deserialize reliably.)
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ConnectionInput {
    #[serde(rename = "azure-devops")]
    AzureDevOps {
        name: String,
        org_url: String,
        auth: SecretInput,
    },
    #[serde(rename = "github")]
    GitHub {
        name: String,
        org: String,
        auth: SecretInput,
    },
    Llm {
        name: String,
        provider: String,
        #[serde(default)]
        base_url: Option<String>,
        model: String,
        #[serde(default)]
        key: Option<SecretInput>,
        #[serde(default)]
        is_local: bool,
        #[serde(default)]
        residency: Option<String>,
    },
}

impl ConnectionInput {
    fn into_named_kind(self) -> Result<(String, ConnectionKind), (StatusCode, String)> {
        Ok(match self {
            ConnectionInput::AzureDevOps {
                name,
                org_url,
                auth,
            } => (
                name,
                ConnectionKind::AzureDevOps {
                    org_url,
                    auth: auth.into_ref()?,
                },
            ),
            ConnectionInput::GitHub { name, org, auth } => (
                name,
                ConnectionKind::GitHub {
                    org,
                    auth: auth.into_ref()?,
                },
            ),
            ConnectionInput::Llm {
                name,
                provider,
                base_url,
                model,
                key,
                is_local,
                residency,
            } => (
                name,
                ConnectionKind::Llm {
                    provider,
                    base_url,
                    model,
                    key: key.map(SecretInput::into_ref).transpose()?,
                    is_local,
                    residency,
                },
            ),
        })
    }
}

/// Create or update a connection (#154). Admin-only (enforced by the middleware).
/// Inline secrets are encrypted before storage; the response is **redacted** (no
/// secret material). Owned by the caller's tenant.
async fn create_connection(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
    Json(input): Json<ConnectionInput>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let (name, kind) = input.into_named_kind()?;
    let conn = Connection {
        id: format!("conn-{}-{}", caller.tenant, slugify(&name)),
        tenant: caller.tenant.clone(),
        name,
        kind,
        updated_by: caller.actor(),
        updated_at: now_iso8601(),
    };
    state.store.put_connection(&conn).await.map_err(internal)?;
    tracing::info!(
        "connection '{}' upserted in tenant '{}' by {}",
        conn.id,
        conn.tenant,
        conn.updated_by
    );
    Ok(Json(serde_json::json!({ "connection": conn.redacted() })))
}

/// List the caller-tenant's connections (redacted). Admin-only.
async fn list_connections(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let conns = state
        .store
        .list_connections(&caller.tenant)
        .await
        .map_err(internal)?;
    let redacted: Vec<_> = conns.iter().map(Connection::redacted).collect();
    Ok(Json(serde_json::json!({ "connections": redacted })))
}

/// Delete a connection by id within the caller's tenant. Admin-only. 404 if absent.
async fn delete_connection(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let removed = state
        .store
        .delete_connection(&caller.tenant, &id)
        .await
        .map_err(internal)?;
    if !removed {
        return Err((StatusCode::NOT_FOUND, format!("no connection '{id}'")));
    }
    tracing::info!(
        "connection '{id}' deleted in tenant '{}' by {}",
        caller.tenant,
        caller.actor()
    );
    Ok(Json(serde_json::json!({ "deleted": id })))
}

/// Body of `POST /api/jobs/convert`: which pipelines to convert. Omit
/// `pipelineIds` to convert every not-yet-started pipeline in the portfolio.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConvertJobBody {
    #[serde(default)]
    pipeline_ids: Option<Vec<String>>,
}

/// Kick off a conversion job (fan-out across pipelines). Returns `{ jobId, total }`;
/// progress streams from `/api/jobs/:id/events` and snapshots at `/api/jobs/:id`.
async fn start_convert_job(
    State(state): State<Shared>,
    Extension(caller): Extension<Identity>,
    body: Option<Json<ConvertJobBody>>,
) -> Json<Value> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    // Build (pipeline_id, project) pairs so each conversion audits the right ADO
    // project live. Explicit ids resolve their project from the portfolio too.
    let portfolio = state.portfolio.read().await;
    let project_of = |id: &str| {
        portfolio
            .pipelines
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.project.clone())
    };
    let pairs: Vec<(String, Option<String>)> = match body.pipeline_ids {
        Some(ids) => ids
            .into_iter()
            .map(|id| (id.clone(), project_of(&id)))
            .collect(),
        None => portfolio
            .pipelines
            .iter()
            .filter(|p| p.status == ProposalStatus::NotStarted)
            .map(|p| (p.id.clone(), Some(p.project.clone())))
            .collect(),
    };
    drop(portfolio);

    let n = state.next_job.fetch_add(1, Ordering::Relaxed);
    let id = format!("job-{n}");
    let job = jobs::spawn_convert_job(id.clone(), state.store.clone(), pairs, caller.tenant);
    state.jobs.write().await.insert(id.clone(), job.clone());
    Json(json!({ "jobId": id, "total": job.total }))
}

/// Current job progress snapshot. 404 if the job id is unknown.
async fn job_status(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let job = state
        .jobs
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(job.snapshot().await))
}

/// Live job progress as Server-Sent Events (#44). The first event is a snapshot
/// (so late subscribers catch up), followed by live `item` / `done` events.
async fn job_events(
    State(state): State<Shared>,
    Path(id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    let job = state
        .jobs
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;

    let snapshot = Event::default()
        .event("snapshot")
        .json_data(job.snapshot().await)
        .unwrap_or_default();
    let live = BroadcastStream::new(job.subscribe()).map(|r| {
        let event = match r {
            Ok(ev) => Event::default().json_data(&ev).unwrap_or_default(),
            Err(_) => Event::default().comment("lagged"),
        };
        Ok(event)
    });
    let stream = tokio_stream::once(Ok::<Event, Infallible>(snapshot)).chain(live);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Run the conversion loop for `pipeline_id`, using live tooling where it is
/// configured and falling back to the offline mocks otherwise.
///
/// The live path is **opt-in** via `BIFROST_CONVERT_LIVE` — merely having an API
/// key or ADO creds in the environment never silently triggers paid calls or a
/// Docker run, and keeps tests deterministic. With it set:
/// - **Importer**: the Docker `gh actions-importer` when `BIFROST_PROJECT` +
///   `AZDO_ORG_URL` are set; otherwise `MockImporter`.
/// - **LLM**: Anthropic (`ANTHROPIC_API_KEY`) and Gemini (`GEMINI_API_KEY`) when
///   not air-gap, Ollama when `OLLAMA_BASE_URL` is set or air-gap is on;
///   otherwise `MockLlmProvider`.
/// - `BIFROST_AIR_GAP` forces local-only routing (the [`Router`] never returns a
///   frontier), so no pipeline data leaves the box.
///
/// Unset, this is the zero-config mock path.
async fn run_conversion(
    pipeline_id: &str,
    project: Option<&str>,
) -> Result<ConversionOutcome, bifrost_adapters::ConversionError> {
    use bifrost_adapters::{DockerImporter, Importer};
    use bifrost_llm::{
        AnthropicProvider, CopilotProvider, GeminiProvider, LlmProvider, OllamaProvider,
        OpenAiCompatibleProvider,
    };

    let truthy = |v: String| matches!(v.as_str(), "1" | "true" | "yes");
    let live = std::env::var("BIFROST_CONVERT_LIVE")
        .map(truthy)
        .unwrap_or(false);
    let air_gap = live
        && std::env::var("BIFROST_AIR_GAP")
            .map(truthy)
            .unwrap_or(false);

    // Real providers, included only in live mode and when explicitly configured
    // (Ollama's `from_env` defaults its URL, so gate it on the var being set).
    let anthropic = (live && !air_gap && std::env::var("ANTHROPIC_API_KEY").is_ok())
        .then(AnthropicProvider::from_env)
        .and_then(Result::ok);
    let gemini = (live && !air_gap && std::env::var("GEMINI_API_KEY").is_ok())
        .then(GeminiProvider::from_env)
        .and_then(Result::ok);
    // GitHub Models — gated on a dedicated var so the importer's GITHUB_TOKEN
    // doesn't silently pull it in.
    let copilot = (live && !air_gap && std::env::var("GITHUB_MODELS_TOKEN").is_ok())
        .then(CopilotProvider::from_env)
        .and_then(Result::ok);
    let ollama = (live && (air_gap || std::env::var("OLLAMA_BASE_URL").is_ok()))
        .then(OllamaProvider::from_env);
    // Generic OpenAI-compatible endpoint (#155): Antigravity, vLLM, a local
    // Gemma, Ollama's /v1 … Gated on its base URL (not the frontier `!air_gap`
    // gate) — its own `is_local` flag decides whether the Router uses it in
    // air-gap mode.
    let openai_compat = (live && std::env::var("BIFROST_OPENAI_BASE_URL").is_ok())
        .then(OpenAiCompatibleProvider::from_env)
        .and_then(Result::ok);
    let mock_llm = MockLlmProvider;

    let live_llm = anthropic.is_some()
        || gemini.is_some()
        || copilot.is_some()
        || ollama.is_some()
        || openai_compat.is_some();
    let mut providers: Vec<&dyn LlmProvider> = Vec::new();
    if let Some(a) = anthropic.as_ref() {
        providers.push(a);
    }
    if let Some(g) = gemini.as_ref() {
        providers.push(g);
    }
    if let Some(c) = copilot.as_ref() {
        providers.push(c);
    }
    if let Some(o) = ollama.as_ref() {
        providers.push(o);
    }
    if let Some(oc) = openai_compat.as_ref() {
        providers.push(oc);
    }
    let policy = if live_llm {
        RoutingPolicy::from_env()
    } else {
        providers.push(&mock_llm);
        RoutingPolicy {
            bulk: vec!["mock".into()],
            hard: vec!["mock".into()],
            docs: vec!["mock".into()],
        }
    };
    let router = LlmRouter::new(providers, air_gap).with_policy(policy);

    // Real Importer when live + a project (the pipeline's, else BIFROST_PROJECT)
    // + ADO org are configured; else the mock.
    let docker = live
        .then(|| {
            project
                .map(str::to_string)
                .or_else(|| std::env::var("BIFROST_PROJECT").ok())
        })
        .flatten()
        .and_then(|p| DockerImporter::from_env(p).ok());
    let mock_importer = MockImporter;
    let importer: &dyn Importer = match docker.as_ref() {
        Some(d) => d,
        None => &mock_importer,
    };

    tracing::info!(
        importer = if docker.is_some() { "docker" } else { "mock" },
        llm = if live_llm { "live" } else { "mock" },
        air_gap,
        "converting pipeline '{pipeline_id}'"
    );

    convert_pipeline(
        importer,
        &router,
        pipeline_id,
        &proposal_id_for(pipeline_id),
        Classification::Yaml,
        // Empty base — convert_pipeline detects languages/build tools from the
        // pipeline itself (#108).
        "",
    )
    .await
}

fn app(state: Shared) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/portfolio", get(portfolio))
        .route("/api/refresh", post(refresh))
        .route("/api/pipelines/:id/convert", post(convert))
        .route("/api/proposals/:id/transition", post(transition))
        .route("/api/proposals/:id", patch(edit))
        .route("/api/proposals/:id/commit", post(commit))
        .route("/api/proposals/:id/validate", post(validate))
        .route("/api/proposals/:id/run", get(run_result))
        .route("/api/proposals/:id/parity", get(parity))
        .route("/api/proposals/:id/attest", post(attest))
        .route("/api/proposals/:id/attestation", get(attestation))
        .route("/api/audit-pack", get(audit_pack))
        .route(
            "/api/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/api/connections/:id",
            axum::routing::delete(delete_connection),
        )
        .route("/api/proposals/:id/runbook", patch(set_runbook_item))
        .route("/api/jobs/convert", post(start_convert_job))
        .route("/api/jobs/:id", get(job_status))
        .route("/api/jobs/:id/events", get(job_events))
        .route("/api/me", get(me))
        // Authenticate (and, when enabled, gate) every /api/* request, attaching
        // the resolved Identity to the request for handlers/RBAC (#65/#66).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The minimum role a request needs, by method + path (#66 RBAC). Reads need
/// `Viewer`; the org-wide compliance export needs `Admin`; everything else that
/// mutates needs `Reviewer`.
fn required_role(method: &axum::http::Method, path: &str) -> Role {
    use axum::http::Method;
    // The org-wide compliance export and all connection/config management are
    // admin-only (sensitive, even to read).
    if path == "/api/audit-pack" || path.starts_with("/api/connections") {
        return Role::Admin;
    }
    match *method {
        Method::GET | Method::HEAD | Method::OPTIONS => Role::Viewer,
        _ => Role::Reviewer,
    }
}

/// Extract the proposal id from a `/api/proposals/<id>[/...]` path, for the
/// middleware's tenant check. `None` for any other path.
fn proposal_id_from_path(path: &str) -> Option<&str> {
    path.strip_prefix("/api/proposals/")
        .map(|rest| rest.split('/').next().unwrap_or(rest))
        .filter(|id| !id.is_empty())
}

/// Authenticate the request, enforce RBAC + tenant isolation, and attach the
/// [`Identity`]. `/api/health` is always open. With auth disabled every request
/// is the local admin (single tenant). With auth enabled: a valid bearer is
/// required (401), the identity's role must meet [`required_role`] (403), and a
/// proposal addressed by id must belong to the caller's tenant (404 otherwise) —
/// so one tenant can neither see nor touch another's migrations (#65/#66).
async fn auth_middleware(State(state): State<Shared>, mut req: Request, next: Next) -> Response {
    if req.uri().path() == "/api/health" {
        return next.run(req).await;
    }
    if !state.auth_enabled {
        req.extensions_mut().insert(Identity::local_admin());
        return next.run(req).await;
    }

    // Authenticate.
    let bearer = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    let identity = match bearer {
        Some(token) => match state.auth.authenticate(token).await {
            Ok(id) => id,
            Err(e) => {
                tracing::debug!("auth rejected: {e}");
                return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
            }
        },
        None => return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response(),
    };

    // Authorize: role.
    let path = req.uri().path().to_string();
    let need = required_role(req.method(), &path);
    if !identity.has_role(need) {
        return (StatusCode::FORBIDDEN, format!("requires {need:?}")).into_response();
    }

    // Authorize: tenant ownership of a proposal addressed by id.
    if let Some(pid) = proposal_id_from_path(&path) {
        if let Ok(Some(rec)) = state.store.get(pid).await {
            if rec.tenant != identity.tenant {
                // 404 (not 403) so existence doesn't leak across tenants.
                return (StatusCode::NOT_FOUND, format!("no proposal '{pid}'")).into_response();
            }
        }
    }

    req.extensions_mut().insert(identity);
    next.run(req).await
}

/// Who am I — the authenticated identity for the current request (#65).
async fn me(identity: Option<Extension<Identity>>) -> Result<Json<Identity>, (StatusCode, String)> {
    match identity {
        Some(Extension(id)) => Ok(Json(id)),
        None => Err((StatusCode::UNAUTHORIZED, "not authenticated".into())),
    }
}

/// Resolve the portfolio source (see module docs). Never panics.
async fn resolve_portfolio() -> Portfolio {
    if let Ok(project) = std::env::var("BIFROST_PROJECT") {
        match build_live(&project).await {
            Ok(p) => {
                tracing::info!("serving live audit of project '{project}'");
                return p;
            }
            Err(e) => tracing::warn!("live audit of '{project}' failed: {e}; falling back"),
        }
    }
    if let Ok(path) = std::env::var("BIFROST_PORTFOLIO") {
        match std::fs::read_to_string(&path).map(|s| serde_json::from_str(&s)) {
            Ok(Ok(p)) => {
                tracing::info!("serving portfolio from {path}");
                return p;
            }
            Ok(Err(e)) => tracing::warn!("BIFROST_PORTFOLIO parse error: {e}; using sample"),
            Err(e) => tracing::warn!("BIFROST_PORTFOLIO read error: {e}; using sample"),
        }
    }
    tracing::info!("serving sample portfolio");
    sample::portfolio()
}

/// Run a live audit and assemble the portfolio.
async fn build_live(project: &str) -> anyhow::Result<Portfolio> {
    use bifrost_adapters::{
        audit_portfolio, AuditConfig, AzureDevOpsAdapter, DockerImporter, Importer,
    };
    let adapter = AzureDevOpsAdapter::from_env(project)?;
    let importer = DockerImporter::from_env(project)?;
    let version = importer
        .version()
        .await
        .unwrap_or_else(|_| "unknown".into());
    let importer_image_digest = importer.image_digest().await.unwrap_or_default();
    let org = std::env::var("AZDO_ORG_URL").unwrap_or_default();
    let config = AuditConfig {
        org: org
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("unknown")
            .to_string(),
        importer_version: version,
        importer_image_digest,
        ado2gh_version: "n/a".into(),
        air_gap: false,
        generated_at: now_iso8601(),
    };
    Ok(audit_portfolio(&adapter, &importer, config).await?)
}

/// Current UTC timestamp via coreutils `date` (avoids a chrono dependency).
fn now_iso8601() -> String {
    std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bifrost_api=info,tower_http=info".into()),
        )
        .init();

    // Resolve the portfolio once at startup (a live audit may take a while).
    let (authn, auth_enabled) = auth::select_authenticator();
    let state: Shared = Arc::new(AppState {
        portfolio: RwLock::new(resolve_portfolio().await),
        store: store::from_env().await,
        jobs: RwLock::new(HashMap::new()),
        next_job: AtomicU64::new(1),
        auth: authn,
        auth_enabled,
        secrets: Arc::new(secrets::DefaultSecretResolver),
    });

    let addr = std::env::var("BIFROST_API_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("bifrost-api listening on http://{addr}");

    axum::serve(listener, app(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::ProposalStatus;

    #[tokio::test]
    async fn conversion_helper_produces_a_draft_proposal_and_runbook() {
        let outcome = run_conversion("SARC-main", None)
            .await
            .expect("offline conversion succeeds");
        assert_eq!(outcome.proposal.status, ProposalStatus::Draft);
        assert_eq!(outcome.proposal.pipeline_id, "SARC-main");
        // Assembled workflow + populated manual-task runbook are present.
        assert!(outcome.proposal.proposed_yaml.contains("REVIEW BEFORE USE"));
        assert!(!outcome.runbook.is_empty());
    }

    fn test_state() -> Shared {
        Arc::new(AppState {
            portfolio: RwLock::new(sample::portfolio()),
            store: Arc::new(store::InMemoryStore::default()),
            jobs: RwLock::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            auth: Arc::new(auth::MockAuthenticator::default()),
            auth_enabled: false,
            secrets: Arc::new(secrets::DefaultSecretResolver),
        })
    }

    /// The caller-identity extension handlers receive from the auth middleware;
    /// in direct handler tests we pass the local admin (tenant `default`).
    fn admin() -> Extension<Identity> {
        Extension(Identity::local_admin())
    }

    #[tokio::test]
    async fn convert_stores_then_transition_walks_the_lifecycle() {
        let state = test_state();

        // Convert → a stored Draft with an empty audit trail.
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["proposal"]["status"], "draft");
        assert_eq!(body["audit"].as_array().unwrap().len(), 0);

        // Re-convert is idempotent — same proposal, not a fresh one.
        let again = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        assert_eq!(again["proposal"]["id"], body["proposal"]["id"]);

        // draft → in_review → approved, each recorded.
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let r = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody {
                    to,
                    actor: Some("rev@x".into()),
                }),
            )
            .await
            .unwrap()
            .0;
            assert_eq!(r["proposal"]["status"], serde_json::to_value(to).unwrap());
        }
        let after = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        assert_eq!(after["audit"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn commit_requires_approval_then_opens_a_pr_and_moves_to_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();

        // Committing a Draft is rejected.
        let err = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        // Walk to Approved.
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }

        // Commit → mock PR URL, status Committed, prUrl carried on the proposal.
        let res = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(res["proposal"]["status"], "committed");
        assert!(res["prUrl"].as_str().unwrap().contains("/pull/"));
        assert_eq!(res["proposal"]["prUrl"], res["prUrl"]);

        // Committing again is illegal (Committed → Committed is not an edge).
        let err = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        // Validate (sandbox trigger) is allowed once Committed → mock dispatch.
        let v = validate(State(state.clone()), Path(pid)).await.unwrap().0;
        assert_eq!(v["trigger"]["dispatched"], true);
        assert!(v["trigger"]["workflowFile"]
            .as_str()
            .unwrap()
            .ends_with(".yml"));
    }

    #[tokio::test]
    async fn validate_requires_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        // Draft can't be validated.
        let err = validate(State(state.clone()), Path(pid)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn run_result_requires_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        // A run can't be captured before the workflow is committed/dispatched.
        let err = run_result(State(state.clone()), Path(pid))
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn run_result_after_commit_reports_mock_run() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();

        // Resolve required tasks, then approve → commit.
        let n = body["runbook"]["items"].as_array().unwrap().len();
        for i in 0..n {
            let _ = set_runbook_item(
                State(state.clone()),
                Path(pid.clone()),
                Json(RunbookItemBody {
                    index: i,
                    done: true,
                }),
            )
            .await
            .unwrap();
        }
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }
        let _ = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap();

        // Capture the run → mock collector reports a completed/success run.
        let res = run_result(State(state.clone()), Path(pid)).await.unwrap().0;
        assert_eq!(res["run"]["status"], "completed");
        assert_eq!(res["run"]["conclusion"], "success");
        assert!(!res["run"]["jobs"].as_array().unwrap().is_empty());
        // Declared outputs come from the proposed workflow YAML (may be empty).
        assert!(res["declaredOutputs"].is_array());
    }

    #[tokio::test]
    async fn parity_requires_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        let err = parity(State(state.clone()), Path(pid)).await.unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn parity_after_commit_reports_a_verdict() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();

        // Resolve required tasks, approve → commit.
        let n = body["runbook"]["items"].as_array().unwrap().len();
        for i in 0..n {
            let _ = set_runbook_item(
                State(state.clone()),
                Path(pid.clone()),
                Json(RunbookItemBody {
                    index: i,
                    done: true,
                }),
            )
            .await
            .unwrap();
        }
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }
        let _ = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap();

        // Parity diff → deterministic verdict ("pass" or "gaps") + the caveat.
        let res = parity(State(state.clone()), Path(pid)).await.unwrap().0;
        let verdict = res["parity"]["verdict"].as_str().unwrap();
        assert!(verdict == "pass" || verdict == "gaps");
        assert!(res["parity"]["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().starts_with("Smoke parity only")));
    }

    #[tokio::test]
    async fn attest_requires_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        let err = attest(State(state.clone()), Path(pid), None)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn attest_records_parity_on_proposal_and_in_audit_log() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();

        // Resolve required tasks, approve → commit.
        let n = body["runbook"]["items"].as_array().unwrap().len();
        for i in 0..n {
            let _ = set_runbook_item(
                State(state.clone()),
                Path(pid.clone()),
                Json(RunbookItemBody {
                    index: i,
                    done: true,
                }),
            )
            .await
            .unwrap();
        }
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }
        let _ = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap();

        // Attest → the parity verdict + report are recorded on the proposal and
        // an attestation event is appended to the immutable audit log.
        let res = attest(State(state.clone()), Path(pid.clone()), None)
            .await
            .unwrap()
            .0;
        let verdict = res["proposal"]["parity"]["verdict"].as_str().unwrap();
        assert!(verdict == "pass" || verdict == "gaps");
        assert_eq!(res["proposal"]["parity"]["subject"], "SARC-main");
        // The attestation note is in the audit trail.
        assert!(res["audit"].as_array().unwrap().iter().any(|e| e["note"]
            .as_str()
            .is_some_and(|n| n.contains("parity attested"))));

        // Re-attesting is allowed while committed (re-runs the diff).
        let res2 = attest(State(state.clone()), Path(pid.clone()), None)
            .await
            .unwrap()
            .0;
        assert!(res2["proposal"]["parity"].is_object());

        // The signed attestation export carries the decisions + parity and verifies.
        let doc = attestation(State(state.clone()), Path(pid))
            .await
            .unwrap()
            .0;
        assert_eq!(doc["subject"], "SARC-main");
        assert_eq!(doc["signature"]["algorithm"], "hmac-sha256");
        assert!(!doc["predicate"]["decisions"].as_array().unwrap().is_empty());
        assert!(doc["predicate"]["parity"].is_object());
        // Round-trips through the core verifier with the dev key.
        let signed: bifrost_core::SignedMigrationAttestation = serde_json::from_value(doc).unwrap();
        assert!(signed.verify(b"bifrost-dev-key"));
    }

    #[tokio::test]
    async fn attestation_exports_a_signed_record_for_any_proposal() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        // Even a fresh draft exports a (signed) attestation of its current state.
        let doc = attestation(State(state.clone()), Path(pid))
            .await
            .unwrap()
            .0;
        let signed: bifrost_core::SignedMigrationAttestation = serde_json::from_value(doc).unwrap();
        assert!(signed.verify(b"bifrost-dev-key"));
        assert!(!signed.verify(b"wrong-key"));
    }

    #[test]
    fn required_role_gates_by_method_and_path() {
        use axum::http::Method;
        assert_eq!(required_role(&Method::GET, "/api/portfolio"), Role::Viewer);
        assert_eq!(
            required_role(&Method::POST, "/api/proposals/p/commit"),
            Role::Reviewer
        );
        assert_eq!(
            required_role(&Method::PATCH, "/api/proposals/p"),
            Role::Reviewer
        );
        // The org-wide compliance export is admin-only.
        assert_eq!(required_role(&Method::GET, "/api/audit-pack"), Role::Admin);
    }

    #[test]
    fn proposal_id_from_path_extracts_the_id() {
        assert_eq!(
            proposal_id_from_path("/api/proposals/prop-x"),
            Some("prop-x")
        );
        assert_eq!(
            proposal_id_from_path("/api/proposals/prop-x/commit"),
            Some("prop-x")
        );
        assert_eq!(proposal_id_from_path("/api/portfolio"), None);
        assert_eq!(proposal_id_from_path("/api/proposals/"), None);
    }

    /// A test authenticator that reads the bearer as `"<tenant>/<role>"`, so
    /// router tests can present different principals.
    #[derive(Debug)]
    struct TenantRoleAuth;

    #[async_trait::async_trait]
    impl auth::Authenticator for TenantRoleAuth {
        async fn authenticate(&self, bearer: &str) -> Result<Identity, auth::AuthError> {
            let (tenant, role) = bearer
                .split_once('/')
                .ok_or_else(|| auth::AuthError::Token("want tenant/role".into()))?;
            let roles = Role::from_claim(role).into_iter().collect();
            Ok(Identity {
                subject: format!("{tenant}:{role}"),
                name: None,
                email: Some(format!("{role}@{tenant}")),
                tenant: tenant.to_string(),
                roles,
            })
        }
    }

    fn enforced_state() -> Shared {
        Arc::new(AppState {
            portfolio: RwLock::new(sample::portfolio()),
            store: Arc::new(store::InMemoryStore::default()),
            jobs: RwLock::new(HashMap::new()),
            next_job: AtomicU64::new(1),
            auth: Arc::new(TenantRoleAuth),
            auth_enabled: true,
            secrets: Arc::new(secrets::DefaultSecretResolver),
        })
    }

    async fn send(state: &Shared, method: &str, uri: &str, bearer: Option<&str>) -> StatusCode {
        use tower::ServiceExt;
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(b) = bearer {
            req = req.header(axum::http::header::AUTHORIZATION, format!("Bearer {b}"));
        }
        let resp = app(state.clone())
            .oneshot(req.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn enforced_auth_requires_token_role_and_tenant() {
        let state = enforced_state();
        // Seed a proposal owned by tenant "acme".
        let rec = StoredProposal {
            proposal: bifrost_core::Proposal::new(
                "prop-x",
                "x",
                "",
                "",
                "",
                vec![],
                vec![],
                "p",
                1.0,
                &bifrost_core::assess(&bifrost_core::RiskSignals::default()),
            ),
            runbook: bifrost_core::Runbook::default(),
            audit: AuditLog::new(),
            tenant: "acme".into(),
        };
        state.store.put(&rec).await.unwrap();

        // Health is always open.
        assert_eq!(
            send(&state, "GET", "/api/health", None).await,
            StatusCode::OK
        );
        // No token → 401.
        assert_eq!(
            send(&state, "GET", "/api/portfolio", None).await,
            StatusCode::UNAUTHORIZED
        );
        // Viewer cannot POST a conversion (needs Reviewer) → 403.
        assert_eq!(
            send(
                &state,
                "POST",
                "/api/pipelines/x/convert",
                Some("acme/viewer")
            )
            .await,
            StatusCode::FORBIDDEN
        );
        // Viewer can read the portfolio → 200.
        assert_eq!(
            send(&state, "GET", "/api/portfolio", Some("acme/viewer")).await,
            StatusCode::OK
        );
        // The audit pack is admin-only: reviewer → 403, admin → 200.
        assert_eq!(
            send(&state, "GET", "/api/audit-pack", Some("acme/reviewer")).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&state, "GET", "/api/audit-pack", Some("acme/admin")).await,
            StatusCode::OK
        );
        // Tenant isolation: another tenant can't touch acme's proposal → 404,
        // even with sufficient role.
        assert_eq!(
            send(
                &state,
                "GET",
                "/api/proposals/prop-x/run",
                Some("globex/reviewer")
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn connection_crud_encrypts_inline_redacts_and_scopes_by_tenant() {
        std::env::set_var("BIFROST_SECRET_KEY", "conn-test-key");
        let state = test_state();
        // Create an ADO connection with an inline (plaintext) PAT.
        let body: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Prod ADO",
            "kind": "azure-devops",
            "org_url": "https://dev.azure.com/acme",
            "auth": { "type": "inline", "value": "PLAINTEXT-PAT" }
        }))
        .unwrap();
        let res = create_connection(State(state.clone()), admin(), Json(body))
            .await
            .unwrap()
            .0;
        // Response is redacted: no plaintext, no ciphertext.
        let s = serde_json::to_string(&res).unwrap();
        assert!(!s.contains("PLAINTEXT-PAT"));
        assert!(s.contains("dev.azure.com/acme"));

        // List shows it (still redacted).
        let listed = list_connections(State(state.clone()), admin())
            .await
            .unwrap()
            .0;
        assert_eq!(listed["connections"].as_array().unwrap().len(), 1);
        let id = listed["connections"][0]["id"].as_str().unwrap().to_string();

        // The stored ciphertext decrypts back to the original (use-time).
        let stored = state.store.list_connections("default").await.unwrap();
        let bifrost_core::ConnectionKind::AzureDevOps { auth, .. } = &stored[0].kind else {
            panic!("expected ADO connection");
        };
        let bifrost_core::SecretRef::EncryptedInline { ciphertext, nonce } = auth else {
            panic!("expected encrypted inline");
        };
        assert_eq!(
            secrets::decrypt_with("conn-test-key", ciphertext, nonce).unwrap(),
            "PLAINTEXT-PAT"
        );

        // Delete is tenant-scoped: another tenant can't remove it.
        assert!(!state.store.delete_connection("other", &id).await.unwrap());
        let del = delete_connection(State(state.clone()), admin(), Path(id.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(del["deleted"], id);
        std::env::remove_var("BIFROST_SECRET_KEY");
    }

    #[tokio::test]
    async fn ado_inputs_resolves_org_url_and_secret() {
        // An ADO connection with an inline (encrypted) PAT resolves to (org_url, pat).
        let auth = secrets::encrypt_with("k", "the-pat").unwrap();
        let conn = Connection {
            id: "c".into(),
            tenant: "acme".into(),
            name: "ado".into(),
            kind: ConnectionKind::AzureDevOps {
                org_url: "https://dev.azure.com/acme".into(),
                auth,
            },
            updated_by: "a".into(),
            updated_at: "t".into(),
        };
        std::env::set_var("BIFROST_SECRET_KEY", "k");
        let resolver = secrets::DefaultSecretResolver;
        let (url, pat) = ado_inputs(&conn, &resolver).await.unwrap();
        assert_eq!(url, "https://dev.azure.com/acme");
        assert_eq!(pat, "the-pat");
        std::env::remove_var("BIFROST_SECRET_KEY");

        // A non-ADO (LLM) connection yields None.
        let llm = Connection {
            kind: ConnectionKind::Llm {
                provider: "ollama".into(),
                base_url: None,
                model: "x".into(),
                key: None,
                is_local: true,
                residency: None,
            },
            ..conn
        };
        assert!(ado_inputs(&llm, &resolver).await.is_none());
    }

    #[tokio::test]
    async fn sample_portfolio_tags_org() {
        let state = test_state();
        let pf = portfolio(State(state.clone()), admin()).await.0;
        assert!(pf.summary.totals.orgs >= 1, "totals carry an org count");
        assert!(
            pf.pipelines.iter().all(|p| !p.org.is_empty()),
            "every pipeline is org-tagged"
        );
    }

    #[tokio::test]
    async fn connections_are_admin_only() {
        // RBAC: connection management requires Admin.
        assert_eq!(
            required_role(&axum::http::Method::GET, "/api/connections"),
            Role::Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::POST, "/api/connections"),
            Role::Admin
        );
        assert_eq!(
            required_role(&axum::http::Method::DELETE, "/api/connections/c1"),
            Role::Admin
        );
    }

    #[tokio::test]
    async fn me_returns_identity_when_present_and_401_when_absent() {
        // The middleware injects an Identity; /api/me echoes it.
        let id = Identity::local_admin();
        let out = me(Some(Extension(id.clone()))).await.unwrap().0;
        assert_eq!(out.subject, id.subject);
        assert!(out.has_role(bifrost_core::Role::Admin));
        // No identity (would only happen outside the middleware) → 401.
        let err = me(None).await.unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn audit_pack_bundles_every_migration_and_verifies() {
        let state = test_state();
        // Two converted pipelines → two migrations in the pack.
        for id in ["SARC-main", "SARC-deploy"] {
            let _ = convert(State(state.clone()), admin(), Path(id.into()))
                .await
                .unwrap();
        }
        let doc = audit_pack(State(state.clone()), admin()).await.unwrap().0;
        assert_eq!(doc["summary"]["total"], 2);
        assert_eq!(doc["attestations"].as_array().unwrap().len(), 2);
        assert_eq!(doc["signature"]["algorithm"], "hmac-sha256");
        // The whole pack verifies through the core verifier with the dev key.
        let pack: bifrost_core::SignedAuditPack = serde_json::from_value(doc).unwrap();
        assert!(pack.verify(b"bifrost-dev-key"));
        assert!(!pack.verify(b"nope"));
    }

    #[tokio::test]
    async fn validate_is_gated_on_required_manual_tasks() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        let n = body["runbook"]["items"].as_array().unwrap().len();
        assert!(n > 0, "SARC-main has manual tasks");

        // Walk to Committed (approve → commit).
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }
        let _ = commit(State(state.clone()), Path(pid.clone()))
            .await
            .unwrap();

        // Committed → Validated is blocked while required tasks are open.
        let err = transition(
            State(state.clone()),
            Path(pid.clone()),
            Json(TransitionBody {
                to: ProposalStatus::Validated,
                actor: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        // Resolve every manual task.
        for i in 0..n {
            let _ = set_runbook_item(
                State(state.clone()),
                Path(pid.clone()),
                Json(RunbookItemBody {
                    index: i,
                    done: true,
                }),
            )
            .await
            .unwrap();
        }

        // Now validation goes through.
        let res = transition(
            State(state.clone()),
            Path(pid),
            Json(TransitionBody {
                to: ProposalStatus::Validated,
                actor: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res["proposal"]["status"], "validated");
    }

    #[tokio::test]
    async fn illegal_transition_and_edit_after_approval_are_409_unknown_is_404() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();

        // draft → approved skips in_review: rejected with 409.
        let err = transition(
            State(state.clone()),
            Path(pid.clone()),
            Json(TransitionBody {
                to: ProposalStatus::Approved,
                actor: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        // Editable while still Draft.
        let edited = edit(
            State(state.clone()),
            Path(pid.clone()),
            Json(EditBody {
                proposed_yaml: "steps: []\n".into(),
                actor: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(edited["proposal"]["proposedYaml"], "steps: []\n");

        // Walk to Approved, then editing is frozen (409).
        for to in [ProposalStatus::InReview, ProposalStatus::Approved] {
            let _ = transition(
                State(state.clone()),
                Path(pid.clone()),
                Json(TransitionBody { to, actor: None }),
            )
            .await
            .unwrap();
        }
        let err = edit(
            State(state.clone()),
            Path(pid.clone()),
            Json(EditBody {
                proposed_yaml: "x".into(),
                actor: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::CONFLICT);

        // Unknown proposal → 404.
        let err = transition(
            State(state.clone()),
            Path("prop-nope".into()),
            Json(TransitionBody {
                to: ProposalStatus::InReview,
                actor: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn portfolio_overlays_live_review_state_from_the_store() {
        let state = test_state();
        let body = convert(State(state.clone()), admin(), Path("web-portal-ci".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        let _ = transition(
            State(state.clone()),
            Path(pid),
            Json(TransitionBody {
                to: ProposalStatus::InReview,
                actor: Some("olaf".into()),
            }),
        )
        .await
        .unwrap();

        // The served portfolio reflects the live proposal status + last actor.
        let pf = portfolio(State(state.clone()), admin()).await.0;
        let p = pf
            .pipelines
            .iter()
            .find(|p| p.id == "web-portal-ci")
            .expect("sample pipeline present");
        assert_eq!(p.status, ProposalStatus::InReview);
        assert_eq!(p.reviewer.as_deref(), Some("olaf"));
        assert!(p.reviewed_at.is_some());
    }
}
