//! Sample portfolio data.
//!
//! Stands in for the output of `bifrost audit` until the ADO adapter + Importer
//! wrapper land. Each pipeline is described by its **risk signals** plus display
//! metadata; the score, band, and factor breakdown are *computed* by the
//! deterministic risk model (`bifrost_core::build_pipeline` → `risk::assess`),
//! never hand-written. Only the source of the signals is fake.

use bifrost_core::{
    build_pipeline, Classification, Pipeline, PipelineMeta, Portfolio, PortfolioSummary,
    ProposalStatus, RiskSignals,
};

use Classification::{Classic, Yaml};
use ProposalStatus::*;

#[allow(clippy::too_many_arguments)]
fn entry(
    id: &str,
    name: &str,
    project: &str,
    status: ProposalStatus,
    unsupported_steps: u32,
    manual_tasks: u32,
    forecast_minutes: u32,
    signals: RiskSignals,
) -> Pipeline {
    build_pipeline(
        PipelineMeta {
            id: id.into(),
            name: name.into(),
            project: project.into(),
            status,
            unsupported_steps,
            manual_tasks,
            forecast_minutes,
        },
        &signals,
    )
}

fn sig(classification: Classification, converted_ratio: f64) -> RiskSignals {
    RiskSignals {
        classification,
        converted_ratio,
        ..Default::default()
    }
}

fn pipelines() -> Vec<Pipeline> {
    vec![
        entry(
            "web-portal-ci",
            "web-portal · CI",
            "Storefront",
            NotStarted,
            0,
            1,
            1850,
            RiskSignals {
                variable_groups: 1,
                ..sig(Yaml, 0.98)
            },
        ),
        entry(
            "web-portal-release",
            "web-portal · Release",
            "Storefront",
            Draft,
            1,
            3,
            920,
            RiskSignals {
                service_connections: 1,
                approval_gates: 1,
                secrets: 4,
                ..sig(Yaml, 0.86)
            },
        ),
        entry(
            "payments-api-ci",
            "payments-api · CI",
            "Payments",
            InReview,
            1,
            2,
            2400,
            RiskSignals {
                self_hosted_pools: 1,
                uses_matrix: true,
                custom_or_marketplace_tasks: 1,
                ..sig(Yaml, 0.91)
            },
        ),
        entry(
            "payments-api-deploy",
            "payments-api · Deploy (classic)",
            "Payments",
            NotStarted,
            7,
            6,
            600,
            RiskSignals {
                service_connections: 3,
                approval_gates: 1,
                custom_or_marketplace_tasks: 1,
                ..sig(Classic, 0.42)
            },
        ),
        entry(
            "ledger-batch",
            "ledger-batch · Nightly",
            "Payments",
            NotStarted,
            4,
            4,
            3100,
            RiskSignals {
                artifact_passing: true,
                complex_conditionals: true,
                custom_or_marketplace_tasks: 1,
                ..sig(Classic, 0.55)
            },
        ),
        entry(
            "mobile-build",
            "mobile-app · Build",
            "Mobile",
            ChangesRequested,
            2,
            3,
            4200,
            RiskSignals {
                self_hosted_pools: 1,
                secrets: 2,
                custom_or_marketplace_tasks: 1,
                ..sig(Yaml, 0.78)
            },
        ),
        entry(
            "mobile-release",
            "mobile-app · Store release",
            "Mobile",
            NotStarted,
            8,
            7,
            480,
            RiskSignals {
                approval_gates: 1,
                custom_or_marketplace_tasks: 1,
                secrets: 2,
                ..sig(Classic, 0.38)
            },
        ),
        entry(
            "data-etl-ci",
            "data-etl · CI",
            "Data Platform",
            Approved,
            0,
            1,
            1300,
            RiskSignals {
                variable_groups: 1,
                ..sig(Yaml, 0.95)
            },
        ),
        entry(
            "data-etl-deploy",
            "data-etl · Deploy",
            "Data Platform",
            InReview,
            1,
            2,
            760,
            RiskSignals {
                service_connections: 1,
                approval_gates: 1,
                secrets: 1,
                ..sig(Yaml, 0.88)
            },
        ),
        entry(
            "infra-terraform",
            "infra · Terraform apply",
            "Platform",
            NotStarted,
            1,
            3,
            540,
            RiskSignals {
                service_connections: 1,
                approval_gates: 1,
                secrets: 1,
                ..sig(Yaml, 0.82)
            },
        ),
        entry(
            "docs-site",
            "docs-site · Build & publish",
            "Platform",
            Validated,
            0,
            0,
            220,
            sig(Yaml, 1.0),
        ),
        entry(
            "auth-service-ci",
            "auth-service · CI",
            "Platform",
            Draft,
            1,
            1,
            1600,
            RiskSignals {
                custom_or_marketplace_tasks: 1,
                variable_groups: 1,
                ..sig(Yaml, 0.93)
            },
        ),
        entry(
            "search-index",
            "search-index · Reindex",
            "Data Platform",
            NotStarted,
            3,
            3,
            2900,
            RiskSignals {
                self_hosted_pools: 1,
                artifact_passing: true,
                ..sig(Classic, 0.6)
            },
        ),
        entry(
            "notifications-ci",
            "notifications · CI",
            "Storefront",
            Approved,
            0,
            1,
            700,
            RiskSignals {
                variable_groups: 1,
                ..sig(Yaml, 0.97)
            },
        ),
        entry(
            "analytics-export",
            "analytics · Export",
            "Data Platform",
            NotStarted,
            1,
            2,
            1100,
            RiskSignals {
                service_connections: 1,
                uses_matrix: true,
                variable_groups: 2,
                ..sig(Yaml, 0.84)
            },
        ),
        entry(
            "gateway-deploy",
            "api-gateway · Deploy (classic)",
            "Platform",
            NotStarted,
            5,
            5,
            820,
            RiskSignals {
                approval_gates: 1,
                service_connections: 1,
                ..sig(Classic, 0.5)
            },
        ),
    ]
}

/// Build the sample portfolio, deriving totals from the (computed) pipeline set.
pub fn portfolio() -> Portfolio {
    let pipelines = pipelines();
    let totals = Portfolio::totals_from(&pipelines);
    Portfolio {
        summary: PortfolioSummary {
            org: "contoso".into(),
            importer_version: "1.3.21847".into(),
            ado2gh_version: "1.10.0".into(),
            air_gap: true,
            generated_at: "2026-06-10T09:14:00Z".into(),
            totals,
        },
        pipelines,
    }
}
