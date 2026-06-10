//! Bifrost control-plane API server.
//!
//! A minimal axum skeleton. Today it serves a health probe and the portfolio
//! view from sample data; the ADO adapter + Importer wrapper will replace the
//! sample source without changing the route contract.

mod sample;

use axum::{routing::get, Json, Router};
use serde_json::{json, Value};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "bifrost-api" }))
}

async fn portfolio() -> Json<bifrost_core::Portfolio> {
    Json(sample::portfolio())
}

fn app() -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/portfolio", get(portfolio))
        // The portal dev server proxies /api, but allow direct cross-origin
        // access too so the SPA can hit the API standalone.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bifrost_api=info,tower_http=info".into()),
        )
        .init();

    let addr = std::env::var("BIFROST_API_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("bifrost-api listening on http://{addr}");

    axum::serve(listener, app()).await?;
    Ok(())
}
