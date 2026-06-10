//! Bifrost control-plane API server.
//!
//! Serves the portfolio the portal renders. The source is resolved once at
//! startup (and on `POST /api/refresh`), in priority order:
//!   1. live audit of `BIFROST_PROJECT` (ADO REST + Docker Importer), if creds present
//!   2. a portfolio JSON file named by `BIFROST_PORTFOLIO`
//!   3. the built-in sample
//!
//! Any failure falls back to the next source, so the server always starts.

mod sample;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{
    routing::{get, post},
    Json, Router,
};
use bifrost_adapters::{convert_pipeline, ConversionOutcome, MockImporter};
use bifrost_core::{Classification, Portfolio};
use bifrost_llm::{MockLlmProvider, Router as LlmRouter, RoutingPolicy};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

type Shared = Arc<RwLock<Portfolio>>;

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "bifrost-api" }))
}

async fn portfolio(State(state): State<Shared>) -> Json<Portfolio> {
    Json(state.read().await.clone())
}

/// Re-resolve the portfolio (e.g. re-run the live audit) and update the cache.
async fn refresh(State(state): State<Shared>) -> Json<Portfolio> {
    let fresh = resolve_portfolio().await;
    *state.write().await = fresh.clone();
    Json(fresh)
}

/// Convert one pipeline into a [`ConversionOutcome`] (proposal + runbook).
///
/// Returns `{ proposal, runbook }` as JSON, or a 500 with the error message.
async fn convert(Path(id): Path<String>) -> Result<Json<Value>, (StatusCode, String)> {
    let outcome = run_conversion(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "proposal": outcome.proposal,
        "runbook": outcome.runbook,
    })))
}

/// Run the conversion loop for `pipeline_id` with the offline provider set.
///
/// Uses `MockImporter` + a local `MockLlmProvider` so the endpoint works with
/// zero setup (no Docker, no API key). Live providers (Docker Importer +
/// Anthropic/Ollama via the air-gap-aware router) are env-gated future work,
/// mirroring how the portfolio source is resolved.
async fn run_conversion(
    pipeline_id: &str,
) -> Result<ConversionOutcome, bifrost_adapters::ConversionError> {
    let importer = MockImporter;
    let provider = MockLlmProvider;
    // Route every task class to the local mock provider.
    let policy = RoutingPolicy {
        bulk: vec!["mock".into()],
        hard: vec!["mock".into()],
        docs: vec!["mock".into()],
    };
    let router = LlmRouter::new(vec![&provider], /* air_gap */ false).with_policy(policy);

    convert_pipeline(
        &importer,
        &router,
        pipeline_id,
        &format!("prop-{pipeline_id}"),
        Classification::Yaml,
        "languages: unknown",
    )
    .await
}

fn app(state: Shared) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/portfolio", get(portfolio))
        .route("/api/refresh", post(refresh))
        .route("/api/pipelines/:id/convert", post(convert))
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
    let state: Shared = Arc::new(RwLock::new(resolve_portfolio().await));

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
        let outcome = run_conversion("SARC-main")
            .await
            .expect("offline conversion succeeds");
        assert_eq!(outcome.proposal.status, ProposalStatus::Draft);
        assert_eq!(outcome.proposal.pipeline_id, "SARC-main");
        // Assembled workflow + populated manual-task runbook are present.
        assert!(outcome.proposal.proposed_yaml.contains("REVIEW BEFORE USE"));
        assert!(!outcome.runbook.is_empty());
    }
}
