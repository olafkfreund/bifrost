//! Audit orchestration.
//!
//! Ties the read side together: enumerate pipelines via a [`SourceAdapter`],
//! dry-run each via an [`Importer`], derive deterministic risk signals from the
//! gaps, assess them, and aggregate into a [`Portfolio`]. Both collaborators are
//! traits, so the whole flow runs and is tested against mocks (no ADO, no Docker).

use bifrost_core::{
    build_pipeline, signals_from_dry_run, GapKind, PipelineMeta, Portfolio, PortfolioSummary,
    ProposalStatus, RiskSignals,
};

use crate::importer::{Importer, ImporterError};
use crate::source::{AdapterError, SourceAdapter};

/// Provenance/config for an audit run, recorded on the portfolio summary.
#[derive(Debug, Clone)]
pub struct AuditConfig {
    pub org: String,
    pub importer_version: String,
    /// Immutable Importer image digest (`repo@sha256:…`) for this run (#30).
    pub importer_image_digest: String,
    pub ado2gh_version: String,
    pub air_gap: bool,
    /// Timestamp for the run (passed in — the core stays clock-free/deterministic).
    pub generated_at: String,
}

/// Errors the orchestrator surfaces, wrapping its collaborators.
#[derive(Debug, thiserror::Error)]
pub enum OrchestrationError {
    #[error(transparent)]
    Adapter(#[from] AdapterError),
    #[error(transparent)]
    Importer(#[from] ImporterError),
}

/// Audit an org into a fully-computed [`Portfolio`].
///
/// For each pipeline: dry-run → [`signals_from_dry_run`] → [`build_pipeline`].
/// Risk is computed by the deterministic engine; the LLM is not involved.
///
/// Runs sequentially for now; bounded-concurrency fan-out is a follow-up (#47).
pub async fn audit_org(
    adapter: &dyn SourceAdapter,
    importer: &dyn Importer,
    config: AuditConfig,
) -> Result<Portfolio, OrchestrationError> {
    let sources = adapter.enumerate_pipelines().await?;
    // Forecast is supplementary — a failure must not abort the audit.
    let forecast = importer.forecast().await.unwrap_or_default();
    let mut pipelines = Vec::with_capacity(sources.len());

    for src in sources {
        let dry_run = importer.dry_run(&src.id).await?;
        let signals = signals_from_dry_run(&dry_run, src.classification);
        let meta = PipelineMeta {
            id: src.id,
            name: src.name.clone(),
            project: src.project,
            org: config.org.clone(),
            status: ProposalStatus::NotStarted,
            unsupported_steps: dry_run.gaps_of(GapKind::UnsupportedStep).count() as u32,
            manual_tasks: dry_run.gaps_of(GapKind::ManualTask).count() as u32,
            forecast_minutes: forecast_for(&forecast, &src.name),
        };
        pipelines.push(build_pipeline(meta, &signals));
    }

    let mut totals = Portfolio::totals_from(&pipelines);
    if forecast.total_minutes > 0 {
        totals.forecast_minutes = forecast.total_minutes;
    }
    Ok(Portfolio {
        summary: PortfolioSummary {
            org: config.org,
            importer_version: config.importer_version,
            importer_image_digest: config.importer_image_digest,
            ado2gh_version: config.ado2gh_version,
            air_gap: config.air_gap,
            generated_at: config.generated_at,
            totals,
        },
        pipelines,
        // Per-pipeline dry-run path doesn't aggregate the report-level audit detail.
        audit: bifrost_core::PortfolioAudit::default(),
    })
}

/// The forecast estimate for a pipeline by name (0 when the report doesn't list it).
fn forecast_for(forecast: &crate::Forecast, name: &str) -> u32 {
    forecast
        .per_pipeline
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, m)| *m)
        .unwrap_or(0)
}

/// Build a [`Portfolio`] from a single org **audit** plus the adapter inventory —
/// the path used when per-pipeline dry-runs aren't available (e.g. the Docker
/// importer). The audit's build-step ratio + the org's connections/secrets are
/// attributed to each pipeline; this is exact for single-pipeline orgs and an
/// approximation otherwise (per-pipeline precision needs dry-runs / #31).
pub async fn audit_portfolio(
    adapter: &dyn SourceAdapter,
    importer: &dyn Importer,
    config: AuditConfig,
) -> Result<Portfolio, OrchestrationError> {
    let sources = adapter.enumerate_pipelines().await?;
    let audit = importer.audit().await?;
    let forecast = importer.forecast().await.unwrap_or_default();
    let connections = adapter.fetch_service_connections().await?;
    let groups = adapter.fetch_variable_groups().await?;

    let secret_vars = groups
        .iter()
        .flat_map(|g| &g.variables)
        .filter(|v| v.is_secret)
        .count() as u32;
    let converted_ratio = if audit.build_steps.total > 0 {
        audit.build_steps.successful as f64 / audit.build_steps.total as f64
    } else {
        1.0
    };

    let mut pipelines = Vec::with_capacity(sources.len());
    for src in sources {
        let signals = RiskSignals {
            classification: src.classification,
            converted_ratio,
            secrets: secret_vars,
            variable_groups: groups.len() as u32,
            service_connections: connections.len() as u32,
            custom_or_marketplace_tasks: audit.unsupported_steps.len() as u32,
            ..Default::default()
        };
        let meta = PipelineMeta {
            id: src.id,
            name: src.name.clone(),
            project: src.project,
            org: config.org.clone(),
            status: ProposalStatus::NotStarted,
            unsupported_steps: audit.build_steps.unsupported,
            manual_tasks: audit.manual_tasks.len() as u32,
            forecast_minutes: forecast_for(&forecast, &src.name),
        };
        pipelines.push(build_pipeline(meta, &signals));
    }

    let mut totals = Portfolio::totals_from(&pipelines);
    if forecast.total_minutes > 0 {
        totals.forecast_minutes = forecast.total_minutes;
    }
    Ok(Portfolio {
        summary: PortfolioSummary {
            org: config.org,
            importer_version: config.importer_version,
            importer_image_digest: config.importer_image_digest,
            ado2gh_version: config.ado2gh_version,
            air_gap: config.air_gap,
            generated_at: config.generated_at,
            totals,
        },
        pipelines,
        // Carry the detail a change-management report needs (#220).
        audit: bifrost_core::PortfolioAudit {
            manual_tasks: audit.manual_tasks,
            unsupported_steps: audit.unsupported_steps,
            actions: audit.actions,
            service_connections: connections,
            variable_groups: groups,
        },
    })
}

/// Merge several per-org [`Portfolio`] audits into one tenant-wide portfolio
/// (#156). Pipelines (already org-tagged) are concatenated and totals recomputed
/// (`totals.orgs` counts the distinct orgs); the summary's org field lists them.
/// Tool provenance is taken from the first audit (orgs are audited with the same
/// pinned Importer).
pub fn merge_portfolios(
    portfolios: Vec<Portfolio>,
    generated_at: impl Into<String>,
    air_gap: bool,
) -> Portfolio {
    let mut pipelines = Vec::new();
    let mut importer_version = String::new();
    let mut importer_image_digest = String::new();
    let mut ado2gh_version = String::new();
    let mut audit = bifrost_core::PortfolioAudit::default();
    for p in portfolios {
        if importer_version.is_empty() {
            importer_version = p.summary.importer_version;
            importer_image_digest = p.summary.importer_image_digest;
            ado2gh_version = p.summary.ado2gh_version;
        }
        pipelines.extend(p.pipelines);
        // Merge each source's report detail (per-project rows stay distinct).
        audit.manual_tasks.extend(p.audit.manual_tasks);
        audit.unsupported_steps.extend(p.audit.unsupported_steps);
        audit.actions.extend(p.audit.actions);
        audit
            .service_connections
            .extend(p.audit.service_connections);
        audit.variable_groups.extend(p.audit.variable_groups);
    }
    audit.actions.sort();
    audit.actions.dedup();
    let totals = Portfolio::totals_from(&pipelines);
    let mut orgs: Vec<&str> = pipelines
        .iter()
        .map(|p| p.org.as_str())
        .filter(|s| !s.is_empty())
        .collect();
    orgs.sort_unstable();
    orgs.dedup();
    Portfolio {
        summary: PortfolioSummary {
            org: orgs.join(", "),
            importer_version,
            importer_image_digest,
            ado2gh_version,
            air_gap,
            generated_at: generated_at.into(),
            totals,
        },
        pipelines,
        audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::MockImporter;
    use crate::source::MockSourceAdapter;
    use bifrost_core::{Classification, RiskBand};

    fn config() -> AuditConfig {
        AuditConfig {
            org: "contoso".into(),
            importer_version: "mock".into(),
            importer_image_digest: "sha256:test".into(),
            ado2gh_version: "mock".into(),
            air_gap: true,
            generated_at: "2026-06-10T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn audits_org_into_a_computed_portfolio() {
        let portfolio = audit_org(&MockSourceAdapter::new(), &MockImporter, config())
            .await
            .expect("audit succeeds");

        // MockSourceAdapter has two pipelines: one YAML, one classic.
        assert_eq!(portfolio.summary.totals.pipelines, 2);
        assert_eq!(portfolio.summary.totals.yaml, 1);
        assert_eq!(portfolio.summary.totals.classic, 1);
        assert_eq!(portfolio.summary.org, "contoso");

        // Every entry has a computed score and a factor breakdown.
        for p in &portfolio.pipelines {
            assert!(
                p.risk_score > 0,
                "shared dry-run produces gaps for every pipeline"
            );
            assert!(!p.factors.is_empty());
        }

        // The classic pipeline scores higher than the YAML one (same gaps + the
        // classic weight), demonstrating per-pipeline computation.
        let classic = portfolio
            .pipelines
            .iter()
            .find(|p| p.classification == Classification::Classic)
            .unwrap();
        let yaml = portfolio
            .pipelines
            .iter()
            .find(|p| p.classification == Classification::Yaml)
            .unwrap();
        assert!(classic.risk_score > yaml.risk_score);
        assert_eq!(classic.risk_band, RiskBand::Red);
    }

    #[tokio::test]
    async fn totals_are_consistent_with_entries() {
        let portfolio = audit_org(&MockSourceAdapter::new(), &MockImporter, config())
            .await
            .unwrap();
        let t = &portfolio.summary.totals;
        assert_eq!(t.green + t.amber + t.red, t.pipelines);
    }

    #[tokio::test]
    async fn merge_combines_orgs_and_recomputes_totals() {
        let cfg = |org: &str| AuditConfig {
            org: org.into(),
            ..config()
        };
        // Two orgs, each audited separately (MockSourceAdapter has 2 pipelines).
        let alpha = audit_org(&MockSourceAdapter::new(), &MockImporter, cfg("alpha"))
            .await
            .unwrap();
        let beta = audit_org(&MockSourceAdapter::new(), &MockImporter, cfg("beta"))
            .await
            .unwrap();

        let merged = merge_portfolios(vec![alpha, beta], "2026-06-11T00:00:00Z", true);
        assert_eq!(merged.pipelines.len(), 4);
        assert_eq!(merged.summary.totals.pipelines, 4);
        assert_eq!(merged.summary.totals.orgs, 2);
        assert_eq!(merged.summary.org, "alpha, beta");
        // Every pipeline carries its source org.
        assert_eq!(
            merged.pipelines.iter().filter(|p| p.org == "alpha").count(),
            2
        );
        assert_eq!(
            merged.pipelines.iter().filter(|p| p.org == "beta").count(),
            2
        );
    }

    #[tokio::test]
    async fn provenance_digest_flows_into_the_summary() {
        let portfolio = audit_org(&MockSourceAdapter::new(), &MockImporter, config())
            .await
            .unwrap();
        // The pinned image digest is recorded for attestation (#30).
        assert_eq!(portfolio.summary.importer_image_digest, "sha256:test");
    }

    #[tokio::test]
    async fn forecast_total_flows_into_portfolio_totals() {
        // MockImporter's forecast fixture reports 23,500 runner-minutes/month.
        let portfolio = audit_org(&MockSourceAdapter::new(), &MockImporter, config())
            .await
            .unwrap();
        assert_eq!(portfolio.summary.totals.forecast_minutes, 23_500);
    }
}
