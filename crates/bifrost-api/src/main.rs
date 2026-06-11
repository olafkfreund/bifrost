//! Bifrost control-plane API server.
//!
//! Serves the portfolio the portal renders. The source is resolved once at
//! startup (and on `POST /api/refresh`), in priority order:
//!   1. live audit of `BIFROST_PROJECT` (ADO REST + Docker Importer), if creds present
//!   2. a portfolio JSON file named by `BIFROST_PORTFOLIO`
//!   3. the built-in sample
//!
//! Any failure falls back to the next source, so the server always starts.

mod jobs;
mod sample;
mod store;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{
    routing::{get, patch, post},
    Json, Router,
};
use bifrost_adapters::{
    convert_pipeline, declared_outputs, AzureDevOpsBaseline, BaselineRequest, BaselineSource,
    CommitRequest, ConversionOutcome, GitHubPublisher, GitHubRunCollector, GitHubSandboxTrigger,
    MockBaselineSource, MockImporter, MockPublisher, MockRunCollector, MockSandboxTrigger,
    Publisher, RunCollector, RunQuery, SandboxTrigger, TriggerRequest,
};
use bifrost_core::{
    compare_parity, Attestation, AuditLog, Classification, MigrationAttestation, ParityReport,
    Portfolio, ProposalStatus, RunFacts,
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

async fn portfolio(State(state): State<Shared>) -> Json<Portfolio> {
    let mut portfolio = state.portfolio.read().await.clone();
    // Overlay live review state so the portal's review queue reflects actions
    // taken this session: a converted pipeline shows its current proposal status,
    // and the latest audit event names who last acted and when.
    if let Ok(all) = state.store.list().await {
        let by_id: HashMap<String, &StoredProposal> =
            all.iter().map(|r| (r.proposal.id.clone(), r)).collect();
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

/// Re-resolve the portfolio (e.g. re-run the live audit) and update the cache.
async fn refresh(State(state): State<Shared>) -> Json<Portfolio> {
    let fresh = resolve_portfolio().await;
    *state.portfolio.write().await = fresh.clone();
    Json(fresh)
}

/// Convert one pipeline into a proposal (+ runbook), storing it for review.
///
/// Idempotent: a pipeline already converted returns its stored record (with any
/// edits/transitions intact) rather than reconverting, so review state survives
/// re-opening the panel. Returns `{ proposal, runbook, audit }`.
async fn convert(
    State(state): State<Shared>,
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

/// The publisher for the commit path: the real GitHub one when the live commit
/// path is explicitly enabled and configured, else the offline mock (never a
/// silent write to a customer repo).
fn select_publisher() -> Box<dyn Publisher> {
    let live = std::env::var("BIFROST_COMMIT_LIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if live {
        match GitHubPublisher::from_env() {
            Ok(p) => return Box::new(p),
            Err(e) => {
                tracing::warn!("BIFROST_COMMIT_LIVE set but publisher unavailable: {e}; using mock")
            }
        }
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
/// path is enabled + configured, else the mock (never a silent CI run).
fn select_trigger() -> Box<dyn SandboxTrigger> {
    let live = std::env::var("BIFROST_VALIDATE_LIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if live {
        match GitHubSandboxTrigger::from_env() {
            Ok(t) => return Box::new(t),
            Err(e) => {
                tracing::warn!("BIFROST_VALIDATE_LIVE set but trigger unavailable: {e}; using mock")
            }
        }
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
/// path is enabled + configured, else the mock (never a silent external call).
fn select_collector() -> Box<dyn RunCollector> {
    let live = std::env::var("BIFROST_VALIDATE_LIVE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    if live {
        match GitHubRunCollector::from_env() {
            Ok(c) => return Box::new(c),
            Err(e) => {
                tracing::warn!(
                    "BIFROST_VALIDATE_LIVE set but collector unavailable: {e}; using mock"
                )
            }
        }
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
    let job = jobs::spawn_convert_job(id.clone(), state.store.clone(), pairs);
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
    let mock_llm = MockLlmProvider;

    let live_llm = anthropic.is_some() || gemini.is_some() || copilot.is_some() || ollama.is_some();
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
        .route("/api/proposals/:id/runbook", patch(set_runbook_item))
        .route("/api/jobs/convert", post(start_convert_job))
        .route("/api/jobs/:id", get(job_status))
        .route("/api/jobs/:id/events", get(job_events))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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
    let org = std::env::var("AZDO_ORG_URL").unwrap_or_default();
    let config = AuditConfig {
        org: org
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("unknown")
            .to_string(),
        importer_version: version,
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
    let state: Shared = Arc::new(AppState {
        portfolio: RwLock::new(resolve_portfolio().await),
        store: store::from_env().await,
        jobs: RwLock::new(HashMap::new()),
        next_job: AtomicU64::new(1),
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
        })
    }

    #[tokio::test]
    async fn convert_stores_then_transition_walks_the_lifecycle() {
        let state = test_state();

        // Convert → a stored Draft with an empty audit trail.
        let body = convert(State(state.clone()), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        let pid = body["proposal"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["proposal"]["status"], "draft");
        assert_eq!(body["audit"].as_array().unwrap().len(), 0);

        // Re-convert is idempotent — same proposal, not a fresh one.
        let again = convert(State(state.clone()), Path("SARC-main".into()))
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
        let after = convert(State(state.clone()), Path("SARC-main".into()))
            .await
            .unwrap()
            .0;
        assert_eq!(after["audit"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn commit_requires_approval_then_opens_a_pr_and_moves_to_committed() {
        let state = test_state();
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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

    #[tokio::test]
    async fn validate_is_gated_on_required_manual_tasks() {
        let state = test_state();
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("SARC-main".into()))
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
        let body = convert(State(state.clone()), Path("web-portal-ci".into()))
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
        let pf = portfolio(State(state.clone())).await.0;
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
