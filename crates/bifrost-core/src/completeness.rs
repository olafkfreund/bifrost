//! Migration completeness matrix (#238) — "find all the moving parts and make
//! sure they are migrated."
//!
//! A **deterministic** classifier over what the audit already surfaces: every
//! Azure DevOps moving-part category mapped to its GitHub equivalent and a
//! status. Categories Bifrost cannot yet enumerate are shown as
//! `NotInventoried` rather than omitted — surfacing the gap is the whole point
//! of a completeness view, so nothing is silently dropped. No LLM is involved;
//! status is computed from the portfolio, like the risk and cost models.

use serde::{Deserialize, Serialize};

use crate::audit::ManualTaskKind;
use crate::model::Portfolio;

/// What still has to happen for a category to be fully migrated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CategoryStatus {
    /// The Importer converts this automatically.
    Auto,
    /// Converted, but a human must review it (classic pipelines, gap-filled steps).
    Review,
    /// A human must recreate it in GitHub (secrets, connections, runners).
    Manual,
    /// Bifrost does not yet enumerate this category — go look in ADO.
    NotInventoried,
    /// None present in this portfolio.
    NotApplicable,
}

/// One category's row in the matrix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletenessRow {
    pub category: String,
    /// How many were found. Meaningful only when `inventoried` is true.
    pub count: u32,
    /// Whether `count` is a reliable inventory (false = not yet enumerated).
    pub inventoried: bool,
    pub status: CategoryStatus,
    pub github_equivalent: String,
    pub note: String,
}

fn row(
    category: &str,
    count: u32,
    inventoried: bool,
    status: CategoryStatus,
    github_equivalent: &str,
    note: &str,
) -> CompletenessRow {
    CompletenessRow {
        category: category.into(),
        count,
        inventoried,
        status,
        github_equivalent: github_equivalent.into(),
        note: note.into(),
    }
}

/// If a category has zero items, downgrade its status to `NotApplicable` so the
/// matrix does not flag work that isn't there.
fn present(count: u32, status: CategoryStatus) -> CategoryStatus {
    if count == 0 {
        CategoryStatus::NotApplicable
    } else {
        status
    }
}

/// Build the completeness matrix for a portfolio. Deterministic.
pub fn completeness(portfolio: &Portfolio) -> Vec<CompletenessRow> {
    let t = &portfolio.summary.totals;
    let a = &portfolio.audit;

    let secrets = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::Secret)
        .count() as u32;
    let runners = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::SelfHostedRunner)
        .count() as u32;
    let unsupported: u32 = a.unsupported_steps.iter().map(|u| u.count).sum();
    let service_connections = a.service_connections.len() as u32;
    let variable_groups = a.variable_groups.len() as u32;
    let actions = a.actions.len() as u32;

    vec![
        // --- Converted by the Importer ---
        row(
            "YAML pipelines",
            t.yaml,
            true,
            present(t.yaml, CategoryStatus::Auto),
            "GitHub Actions workflows",
            "Converted automatically; inspect before production use.",
        ),
        row(
            "Triggers (CI / PR / schedule / path)",
            0,
            false,
            CategoryStatus::Auto,
            "on: push / pull_request / schedule",
            "Converted automatically by the Importer; pipeline-completion triggers map to workflow_run.",
        ),
        row(
            "Actions allow-list",
            actions,
            true,
            present(actions, CategoryStatus::Manual),
            "Org/repo Actions policy",
            "Add these actions to the GitHub Actions allow-list.",
        ),
        // --- Converted but needs human review ---
        row(
            "Classic / designer pipelines",
            t.classic,
            true,
            present(t.classic, CategoryStatus::Review),
            "Workflows (reverse-engineered)",
            "The hard tail — UI-defined logic; defaults Amber/Red and needs the most review.",
        ),
        row(
            "Unsupported / partial steps",
            unsupported,
            true,
            present(unsupported, CategoryStatus::Review),
            "Gap-filled workflow steps",
            "The model fills each gap from the diff; a human approves.",
        ),
        // --- Human must recreate in GitHub (Bifrost has names, never values) ---
        row(
            "Secrets",
            secrets,
            true,
            present(secrets, CategoryStatus::Manual),
            "Actions secrets",
            "Names only — values are never read; re-enter them in GitHub.",
        ),
        row(
            "Service connections",
            service_connections,
            true,
            present(service_connections, CategoryStatus::Manual),
            "OIDC federation / secrets",
            "Recreate; Azure connections become Entra workload-identity federation.",
        ),
        row(
            "Variable groups",
            variable_groups,
            true,
            present(variable_groups, CategoryStatus::Manual),
            "Repo/org/environment variables",
            "Recreate; ADO stage-scoped variables have no direct GitHub equivalent.",
        ),
        row(
            "Self-hosted runners",
            runners,
            true,
            present(runners, CategoryStatus::Manual),
            "Self-hosted runners / runner groups",
            "Provision runners and match the labels the workflows expect.",
        ),
        // --- Not yet inventoried by Bifrost (the honest gap — go look in ADO) ---
        row("Secure files", 0, false, CategoryStatus::NotInventoried, "Actions secrets / external vault", "Certificates and signing keys — not yet enumerated; check the ADO library."),
        row("Task groups", 0, false, CategoryStatus::NotInventoried, "Composite actions / reusable workflows", "Classic task groups — not yet enumerated."),
        row("Agent pools", 0, false, CategoryStatus::NotInventoried, "Runner labels / runner groups", "Pool usage beyond named self-hosted runners — not yet enumerated."),
        row("Deployment groups", 0, false, CategoryStatus::NotInventoried, "Self-hosted runner labels", "Release-pipeline deployment targets — not yet enumerated."),
        row("Environments + approvals / gates", 0, false, CategoryStatus::NotInventoried, "GitHub Environments + protection rules", "Pre/post-deployment approvals and gates — not yet enumerated; commonly missed."),
        row("Azure Artifacts feeds", 0, false, CategoryStatus::NotInventoried, "GitHub Packages", "Package feeds and upstream sources — not yet enumerated."),
        row("Retention policies", 0, false, CategoryStatus::NotInventoried, "Artifact retention / Releases", "Run/artifact retention — not yet enumerated."),
        row("Pipeline permissions", 0, false, CategoryStatus::NotInventoried, "Workflow permissions / GITHUB_TOKEN", "Pipeline RBAC — not yet enumerated."),
        // --- Separate tool (GEI) ---
        row("Repositories (history, branches, PRs)", 0, false, CategoryStatus::NotInventoried, "GitHub Enterprise Importer (GEI)", "Out of pipeline scope; migrated and tracked separately via GEI."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{ManualTask, UnsupportedStep};
    use crate::ingestion::ServiceConnection;
    use crate::model::{Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals};

    fn portfolio(yaml: u32, classic: u32, audit: PortfolioAudit) -> Portfolio {
        Portfolio {
            summary: PortfolioSummary {
                org: "o".into(),
                importer_version: "v".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "t".into(),
                totals: PortfolioTotals {
                    pipelines: yaml + classic,
                    orgs: 1,
                    projects: 1,
                    yaml,
                    classic,
                    green: 0,
                    amber: 0,
                    red: 0,
                    forecast_minutes: 0,
                },
            },
            pipelines: vec![],
            audit,
        }
    }

    #[test]
    fn classifies_known_categories_from_audit_data() {
        let audit = PortfolioAudit {
            manual_tasks: vec![
                ManualTask {
                    kind: ManualTaskKind::Secret,
                    name: "NUGET_API_KEY".into(),
                },
                ManualTask {
                    kind: ManualTaskKind::SelfHostedRunner,
                    name: "linux-pool".into(),
                },
            ],
            unsupported_steps: vec![UnsupportedStep {
                task: "DownloadSecureFile@1".into(),
                count: 3,
            }],
            actions: vec!["actions/checkout@v4".into()],
            service_connections: vec![ServiceConnection {
                id: "1".into(),
                name: "azure-prod".into(),
                kind: "azurerm".into(),
                project: "P".into(),
            }],
            variable_groups: vec![],
        };
        let rows = completeness(&portfolio(10, 2, audit));
        let by = |c: &str| rows.iter().find(|r| r.category == c).unwrap().clone();

        assert_eq!(by("YAML pipelines").status, CategoryStatus::Auto);
        assert_eq!(by("YAML pipelines").count, 10);
        assert_eq!(
            by("Classic / designer pipelines").status,
            CategoryStatus::Review
        );
        assert_eq!(by("Unsupported / partial steps").count, 3);
        assert_eq!(by("Secrets").status, CategoryStatus::Manual);
        assert_eq!(by("Secrets").count, 1);
        assert_eq!(by("Self-hosted runners").count, 1);
        assert_eq!(by("Service connections").count, 1);
        // Empty category downgrades to NotApplicable.
        assert_eq!(by("Variable groups").status, CategoryStatus::NotApplicable);
        // Un-enumerated category is honestly flagged.
        let sf = by("Secure files");
        assert_eq!(sf.status, CategoryStatus::NotInventoried);
        assert!(!sf.inventoried);
    }

    #[test]
    fn every_row_has_a_github_equivalent() {
        let rows = completeness(&portfolio(0, 0, PortfolioAudit::default()));
        assert!(rows.iter().all(|r| !r.github_equivalent.is_empty()));
        assert!(rows.len() >= 15);
    }
}
