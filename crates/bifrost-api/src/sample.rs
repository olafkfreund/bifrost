//! Sample portfolio data.
//!
//! Stands in for the output of `bifrost audit` until the ADO adapter + Importer
//! wrapper land. The shape is the real API contract; only the source is fake.
//! Mirrors `portal/src/data/portfolio.ts` so the portal renders identically
//! whether it runs against the mock client or this endpoint.

use bifrost_core::{
    band_for_score, Classification, Pipeline, Portfolio, PortfolioSummary, ProposalStatus,
    RiskFactor,
};

use Classification::{Classic, Yaml};
use ProposalStatus::*;

fn rf(key: &str, label: &str, contribution: i32, detail: &str) -> RiskFactor {
    RiskFactor {
        key: key.into(),
        label: label.into(),
        contribution,
        detail: detail.into(),
    }
}

#[allow(clippy::too_many_arguments)]
fn pl(
    id: &str,
    name: &str,
    project: &str,
    classification: Classification,
    converted_ratio: f64,
    unsupported_steps: u32,
    manual_tasks: u32,
    risk_score: i32,
    status: ProposalStatus,
    forecast_minutes: u32,
    factors: Vec<RiskFactor>,
) -> Pipeline {
    Pipeline {
        id: id.into(),
        name: name.into(),
        project: project.into(),
        classification,
        converted_ratio,
        unsupported_steps,
        manual_tasks,
        risk_band: band_for_score(risk_score),
        risk_score,
        status,
        forecast_minutes,
        factors,
    }
}

fn pipelines() -> Vec<Pipeline> {
    vec![
        pl(
            "web-portal-ci",
            "web-portal · CI",
            "Storefront",
            Yaml,
            0.98,
            0,
            1,
            12,
            NotStarted,
            1850,
            vec![
                rf(
                    "secrets",
                    "Variable group",
                    8,
                    "1 non-secret variable group → repo variables",
                ),
                rf(
                    "cache",
                    "Cache step",
                    4,
                    "Cache@2 maps cleanly to actions/cache",
                ),
            ],
        ),
        pl(
            "web-portal-release",
            "web-portal · Release",
            "Storefront",
            Yaml,
            0.86,
            1,
            3,
            44,
            Draft,
            920,
            vec![
                rf(
                    "service_conn",
                    "Service connection",
                    18,
                    "Azure RM connection → OIDC federation required",
                ),
                rf(
                    "environments",
                    "Approval gate",
                    14,
                    "Pre-deploy approval → Environments + required reviewers",
                ),
                rf(
                    "secrets",
                    "Secrets",
                    12,
                    "4 secret variables → org/repo secrets to provision",
                ),
            ],
        ),
        pl(
            "payments-api-ci",
            "payments-api · CI",
            "Payments",
            Yaml,
            0.91,
            1,
            2,
            38,
            InReview,
            2400,
            vec![
                rf(
                    "selfhosted",
                    "Self-hosted pool",
                    16,
                    "Linux self-hosted pool → runner strategy decision",
                ),
                rf(
                    "matrix",
                    "Matrix semantics",
                    12,
                    "Multi-config matrix differs from GHA strategy.matrix",
                ),
                rf(
                    "marketplace",
                    "Marketplace task",
                    10,
                    "SonarQube task → marketplace action mapping",
                ),
            ],
        ),
        pl(
            "payments-api-deploy",
            "payments-api · Deploy (classic)",
            "Payments",
            Classic,
            0.42,
            7,
            6,
            88,
            NotStarted,
            600,
            vec![
                rf(
                    "classic",
                    "Classic/designer pipeline",
                    30,
                    "Designer pipeline — no YAML source; manual rebuild",
                ),
                rf(
                    "service_conn",
                    "Service connections",
                    22,
                    "3 service connections → OIDC federation",
                ),
                rf(
                    "environments",
                    "Stages + gates",
                    20,
                    "Multi-stage with manual gates and post-deploy checks",
                ),
                rf(
                    "custom",
                    "Custom task",
                    16,
                    "In-house deploy extension with no GHA equivalent",
                ),
            ],
        ),
        pl(
            "ledger-batch",
            "ledger-batch · Nightly",
            "Payments",
            Classic,
            0.55,
            4,
            4,
            72,
            NotStarted,
            3100,
            vec![
                rf(
                    "classic",
                    "Classic/designer pipeline",
                    30,
                    "Designer pipeline — manual rebuild",
                ),
                rf(
                    "schedule",
                    "Schedule + triggers",
                    12,
                    "Cron + path filters → on.schedule + paths",
                ),
                rf(
                    "artifacts",
                    "Artifact passing",
                    18,
                    "Cross-stage artifact publish/download semantics",
                ),
            ],
        ),
        pl(
            "mobile-build",
            "mobile-app · Build",
            "Mobile",
            Yaml,
            0.78,
            2,
            3,
            58,
            ChangesRequested,
            4200,
            vec![
                rf(
                    "selfhosted",
                    "macOS self-hosted",
                    22,
                    "macOS pool for iOS build → runner strategy",
                ),
                rf(
                    "secrets",
                    "Signing secrets",
                    16,
                    "Code-signing certs + provisioning → encrypted secrets",
                ),
                rf(
                    "marketplace",
                    "Fastlane task",
                    10,
                    "Fastlane wrapper → run step or marketplace action",
                ),
            ],
        ),
        pl(
            "mobile-release",
            "mobile-app · Store release",
            "Mobile",
            Classic,
            0.38,
            8,
            7,
            91,
            NotStarted,
            480,
            vec![
                rf(
                    "classic",
                    "Classic/designer pipeline",
                    30,
                    "Designer release with gated environments",
                ),
                rf(
                    "environments",
                    "Store approval gates",
                    22,
                    "TestFlight/Play gates → Environments",
                ),
                rf(
                    "custom",
                    "Store-publish extension",
                    20,
                    "Custom publishing task with no GHA equivalent",
                ),
                rf(
                    "secrets",
                    "API keys",
                    14,
                    "App Store Connect / Play keys → secrets",
                ),
            ],
        ),
        pl(
            "data-etl-ci",
            "data-etl · CI",
            "Data Platform",
            Yaml,
            0.95,
            0,
            1,
            18,
            Approved,
            1300,
            vec![
                rf(
                    "container",
                    "Container job",
                    8,
                    "Container resource → jobs.<id>.container",
                ),
                rf(
                    "secrets",
                    "Variable group",
                    6,
                    "1 variable group → repo variables",
                ),
            ],
        ),
        pl(
            "data-etl-deploy",
            "data-etl · Deploy",
            "Data Platform",
            Yaml,
            0.88,
            1,
            2,
            41,
            InReview,
            760,
            vec![
                rf(
                    "service_conn",
                    "Service connection",
                    18,
                    "Databricks connection → OIDC / secret",
                ),
                rf(
                    "environments",
                    "Approval",
                    12,
                    "Single approval gate → environment reviewer",
                ),
            ],
        ),
        pl(
            "infra-terraform",
            "infra · Terraform apply",
            "Platform",
            Yaml,
            0.82,
            1,
            3,
            49,
            NotStarted,
            540,
            vec![
                rf(
                    "service_conn",
                    "Azure RM connection",
                    18,
                    "→ OIDC federation to GitHub",
                ),
                rf(
                    "environments",
                    "Plan/apply gate",
                    16,
                    "Manual apply approval → Environments",
                ),
                rf(
                    "secrets",
                    "Backend secrets",
                    8,
                    "TF state backend creds → secrets",
                ),
            ],
        ),
        pl(
            "docs-site",
            "docs-site · Build & publish",
            "Platform",
            Yaml,
            1.0,
            0,
            0,
            6,
            Validated,
            220,
            vec![rf(
                "clean",
                "Fully mechanical",
                6,
                "All steps converted; no manual tasks",
            )],
        ),
        pl(
            "auth-service-ci",
            "auth-service · CI",
            "Platform",
            Yaml,
            0.93,
            1,
            1,
            24,
            Draft,
            1600,
            vec![
                rf(
                    "marketplace",
                    "Marketplace task",
                    10,
                    "Trivy scan → marketplace action",
                ),
                rf("cache", "Cache", 6, "Maven cache → actions/cache"),
                rf("secrets", "Variable group", 8, "→ repo variables"),
            ],
        ),
        pl(
            "search-index",
            "search-index · Reindex",
            "Data Platform",
            Classic,
            0.6,
            3,
            3,
            66,
            NotStarted,
            2900,
            vec![
                rf(
                    "classic",
                    "Classic/designer pipeline",
                    30,
                    "Designer pipeline — manual rebuild",
                ),
                rf(
                    "selfhosted",
                    "Self-hosted pool",
                    14,
                    "→ runner strategy decision",
                ),
                rf(
                    "artifacts",
                    "Artifact passing",
                    12,
                    "publish/download semantics",
                ),
            ],
        ),
        pl(
            "notifications-ci",
            "notifications · CI",
            "Storefront",
            Yaml,
            0.97,
            0,
            1,
            14,
            Approved,
            700,
            vec![
                rf("secrets", "Variable group", 8, "→ repo variables"),
                rf("cache", "Cache", 4, "npm cache → actions/cache"),
            ],
        ),
        pl(
            "analytics-export",
            "analytics · Export",
            "Data Platform",
            Yaml,
            0.84,
            1,
            2,
            46,
            NotStarted,
            1100,
            vec![
                rf(
                    "service_conn",
                    "Service connection",
                    16,
                    "BigQuery connection → secret/OIDC",
                ),
                rf("matrix", "Matrix", 10, "Region matrix differs from GHA"),
                rf("schedule", "Schedule", 8, "Cron trigger mapping"),
            ],
        ),
        pl(
            "gateway-deploy",
            "api-gateway · Deploy (classic)",
            "Platform",
            Classic,
            0.5,
            5,
            5,
            79,
            NotStarted,
            820,
            vec![
                rf(
                    "classic",
                    "Classic/designer pipeline",
                    30,
                    "Designer multi-stage deploy",
                ),
                rf(
                    "environments",
                    "Ring deploys",
                    22,
                    "Canary rings → Environments + gates",
                ),
                rf(
                    "service_conn",
                    "Service connections",
                    16,
                    "→ OIDC federation",
                ),
            ],
        ),
    ]
}

/// Build the sample portfolio, deriving totals from the pipeline set.
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
