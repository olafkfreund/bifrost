//! Pre-migration status report (#204, #220).
//!
//! Enterprises want a change-management-grade report **before** any change — an
//! assessment to review with the migration manager and the change advisory board.
//! This module turns an audited [`Portfolio`] into that report (Markdown + a
//! structured [`ReportStats`]), optionally **scoped to a single project** so each
//! project's owner gets their own document.
//!
//! It is **pure and read-only**: generating a report makes no changes. The report
//! states exactly what must change, what must be set up in GitHub (secrets,
//! variables, environments/service connections, the Actions allow-list), and that
//! every change is delivered as a reviewable pull request — never a live edit.

use serde::{Deserialize, Serialize};

use crate::audit::ManualTaskKind;
use crate::model::{Classification, Pipeline, Portfolio, RiskBand};

/// Headline numbers for the status report (over the report's scope).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportStats {
    pub ready: u32,
    pub needs_review: u32,
    pub high_risk: u32,
    pub classic: u32,
    pub total_unsupported_steps: u32,
    pub total_manual_tasks: u32,
    pub forecast_minutes: u32,
    /// Secrets to create in GitHub (distinct names across the scope).
    pub secrets_to_add: u32,
    /// Service connections / integrations to recreate (distinct, in scope).
    pub connections_to_recreate: u32,
}

/// Pipelines in scope: all of them, or just one project's.
fn scoped<'a>(portfolio: &'a Portfolio, project: Option<&str>) -> Vec<&'a Pipeline> {
    portfolio
        .pipelines
        .iter()
        .filter(|p| project.is_none_or(|proj| p.project == proj))
        .collect()
}

/// Secret variable names to create in GitHub for the scope (deduped).
fn secret_names(portfolio: &Portfolio, project: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = portfolio
        .audit
        .variable_groups
        .iter()
        .filter(|g| project.is_none_or(|proj| g.project == proj))
        .flat_map(|g| g.variables.iter())
        .filter(|v| v.is_secret)
        .map(|v| v.name.clone())
        .chain(
            portfolio
                .audit
                .manual_tasks
                .iter()
                .filter(|t| t.kind == ManualTaskKind::Secret)
                .map(|t| t.name.clone()),
        )
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Non-secret variable names to add to GitHub for the scope (deduped).
fn variable_names(portfolio: &Portfolio, project: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = portfolio
        .audit
        .variable_groups
        .iter()
        .filter(|g| project.is_none_or(|proj| g.project == proj))
        .flat_map(|g| g.variables.iter())
        .filter(|v| !v.is_secret)
        .map(|v| v.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Derive the headline numbers for `portfolio` within `project` scope.
pub fn report_stats(portfolio: &Portfolio, project: Option<&str>) -> ReportStats {
    let pipelines = scoped(portfolio, project);
    let band = |b: RiskBand| pipelines.iter().filter(|p| p.risk_band == b).count() as u32;
    let connections = portfolio
        .audit
        .service_connections
        .iter()
        .filter(|c| project.is_none_or(|proj| c.project == proj))
        .count() as u32;
    ReportStats {
        ready: band(RiskBand::Green),
        needs_review: band(RiskBand::Amber),
        high_risk: band(RiskBand::Red),
        classic: pipelines
            .iter()
            .filter(|p| p.classification == Classification::Classic)
            .count() as u32,
        total_unsupported_steps: pipelines.iter().map(|p| p.unsupported_steps).sum(),
        total_manual_tasks: pipelines.iter().map(|p| p.manual_tasks).sum(),
        forecast_minutes: if project.is_some() {
            pipelines.iter().map(|p| p.forecast_minutes).sum()
        } else {
            portfolio.summary.totals.forecast_minutes
        },
        secrets_to_add: secret_names(portfolio, project).len() as u32,
        connections_to_recreate: connections,
    }
}

fn band_label(b: RiskBand) -> &'static str {
    match b {
        RiskBand::Green => "Green",
        RiskBand::Amber => "Amber",
        RiskBand::Red => "Red",
    }
}

fn classification_label(c: Classification) -> &'static str {
    match c {
        Classification::Yaml => "YAML",
        Classification::Classic => "Classic",
    }
}

/// Render the migration report as Markdown for `portfolio`, optionally scoped to a
/// single `project` (its owner / change board). Read-only — **no changes are made**.
pub fn report_markdown(portfolio: &Portfolio, project: Option<&str>) -> String {
    let s = &portfolio.summary;
    let stats = report_stats(portfolio, project);
    let pipelines = scoped(portfolio, project);
    let scope = project.unwrap_or("(all projects)");
    let mut out = String::new();

    out.push_str("# Migration Status Report\n\n");
    out.push_str(&format!(
        "Organization: **{org}**  \nScope: **{scope}**  \nGenerated: {at}  \nImporter: {imp}\n\n",
        org = if s.org.is_empty() {
            "(portfolio)"
        } else {
            &s.org
        },
        at = s.generated_at,
        imp = s.importer_version,
    ));
    out.push_str(
        "> This is a **pre-migration assessment** for the migration manager and change advisory \
         board. No changes have been made to any pipeline. Every change Bifrost makes is \
         delivered as a **reviewable pull request** to the source repository — never a live edit \
         — and only after a human approves it. Secrets are recorded by **name only**; no secret \
         value is ever read or stored.\n\n",
    );

    // --- Overview ---
    out.push_str("## Overview\n\n");
    out.push_str(&format!(
        "- Pipelines in scope: **{}** ({} YAML, {} classic/designer)\n\
         - Migration readiness: **{} ready** (Green), **{} need review** (Amber), **{} high-risk** (Red)\n\
         - Manual work: **{}** constructs the Importer cannot convert, **{}** manual tasks\n\
         - GitHub setup: **{}** secrets and **{}** integrations/service connections to create\n\
         - Forecast runner-minutes/month: **{}**\n\n",
        pipelines.len(),
        pipelines.iter().filter(|p| p.classification == Classification::Yaml).count(),
        stats.classic,
        stats.ready, stats.needs_review, stats.high_risk,
        stats.total_unsupported_steps, stats.total_manual_tasks,
        stats.secrets_to_add, stats.connections_to_recreate,
        stats.forecast_minutes,
    ));

    // --- Per-pipeline assessment ---
    out.push_str("## Per-pipeline assessment\n\n");
    out.push_str(
        "| Pipeline | Project | Type | Risk | Score | Unsupported | Manual tasks |\n\
         |---|---|---|---|---|---|---|\n",
    );
    let mut rows: Vec<&Pipeline> = pipelines.clone();
    rows.sort_by(|a, b| b.risk_score.cmp(&a.risk_score).then(a.name.cmp(&b.name)));
    for p in &rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            p.name,
            p.project,
            classification_label(p.classification),
            band_label(p.risk_band),
            p.risk_score,
            p.unsupported_steps,
            p.manual_tasks,
        ));
    }
    out.push('\n');

    // --- What needs changing ---
    out.push_str("## What needs changing\n\n");
    if portfolio.audit.unsupported_steps.is_empty() {
        out.push_str("No unsupported constructs were reported across this estate.\n\n");
    } else {
        out.push_str(
            "Constructs the Importer could not convert automatically (manual rework):\n\n",
        );
        let mut us = portfolio.audit.unsupported_steps.clone();
        us.sort_by_key(|u| std::cmp::Reverse(u.count));
        for u in us {
            out.push_str(&format!("- `{}` — used {} time(s)\n", u.task, u.count));
        }
        out.push('\n');
    }
    // Per-pipeline risk drivers (the deterministic factors), highest-risk first.
    out.push_str("Per-pipeline risk drivers:\n\n");
    for p in &rows {
        let drivers: Vec<String> = p
            .factors
            .iter()
            .filter(|f| f.contribution > 0)
            .map(|f| format!("{} ({})", f.label, f.detail))
            .collect();
        if drivers.is_empty() {
            out.push_str(&format!(
                "- **{}** — no significant risk drivers.\n",
                p.name
            ));
        } else {
            out.push_str(&format!("- **{}** — {}\n", p.name, drivers.join("; ")));
        }
    }
    out.push('\n');

    // --- GitHub setup required ---
    out.push_str("## GitHub setup required (before/with the PR)\n\n");
    let secrets = secret_names(portfolio, project);
    let variables = variable_names(portfolio, project);
    let runners: Vec<&str> = portfolio
        .audit
        .manual_tasks
        .iter()
        .filter(|t| t.kind == ManualTaskKind::SelfHostedRunner)
        .map(|t| t.name.as_str())
        .collect();
    let connections: Vec<&crate::ingestion::ServiceConnection> = portfolio
        .audit
        .service_connections
        .iter()
        .filter(|c| project.is_none_or(|proj| c.project == proj))
        .collect();

    out.push_str("**Secrets to create** (GitHub repo/environment secrets — values supplied by the owner, never by Bifrost):\n\n");
    if secrets.is_empty() {
        out.push_str("- None detected.\n");
    } else {
        for name in &secrets {
            out.push_str(&format!("- `{name}`\n"));
        }
    }
    out.push('\n');

    out.push_str("**Variables to add** (non-secret Actions variables):\n\n");
    if variables.is_empty() {
        out.push_str("- None detected.\n");
    } else {
        for name in &variables {
            out.push_str(&format!("- `{name}`\n"));
        }
    }
    out.push('\n');

    out.push_str("**Integrations / service connections to recreate** (as GitHub app installs, OIDC federation, or environment secrets — names + types only):\n\n");
    if connections.is_empty() {
        out.push_str("- None detected.\n");
    } else {
        for c in &connections {
            out.push_str(&format!("- `{}` ({})\n", c.name, c.kind));
        }
    }
    out.push('\n');

    if !runners.is_empty() {
        out.push_str("**Self-hosted runners to provision:**\n\n");
        for r in &runners {
            out.push_str(&format!("- {r}\n"));
        }
        out.push('\n');
    }

    out.push_str("**GitHub Actions allow-list** (the actions the converted workflows require — add to the org/repo policy):\n\n");
    if portfolio.audit.actions.is_empty() {
        out.push_str("- None reported.\n");
    } else {
        for a in &portfolio.audit.actions {
            out.push_str(&format!("- `{a}`\n"));
        }
    }
    out.push('\n');

    // --- Per-project breakdown (only for the whole-estate report) ---
    if project.is_none() {
        out.push_str("## Projects\n\n");
        out.push_str(
            "Each project can be migrated and reviewed independently — generate a per-project \
             report for its owner and change board. Projects in scope:\n\n",
        );
        let mut projects: Vec<&str> = pipelines.iter().map(|p| p.project.as_str()).collect();
        projects.sort_unstable();
        projects.dedup();
        for proj in projects {
            let n = pipelines.iter().filter(|p| p.project == proj).count();
            out.push_str(&format!("- **{proj}** — {n} pipeline(s)\n"));
        }
        out.push('\n');
    }

    // --- Delivery ---
    out.push_str("## How changes are delivered\n\n");
    out.push_str(
        "Bifrost follows a **review-first, Prepare → Act → Reflect → Review** workflow:\n\n\
         1. **Prepare** — this report: assess and agree scope before any change.\n\
         2. **Act** — convert each pipeline (Importer + grounded gap-fill); the result is a \
         *proposal*, not a change.\n\
         3. **Reflect** — a human reviews the three-pane diff, risk, and rationale.\n\
         4. **Review** — an approved proposal is committed to a **new branch and opened as a \
         pull request**. Nothing is ever pushed to the default branch; your existing review \
         and CI gates apply to the PR.\n\n\
         Every state transition is recorded to an immutable audit log; a signed attestation is \
         available after validation.\n",
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{ManualTask, UnsupportedStep};
    use crate::ingestion::{ServiceConnection, VariableGroup, VariableRef};
    use crate::model::{
        Pipeline, PortfolioAudit, PortfolioSummary, PortfolioTotals, ProposalStatus, RiskFactor,
    };

    fn pipe(name: &str, project: &str, c: Classification, band: RiskBand, score: i32) -> Pipeline {
        Pipeline {
            id: name.to_string(),
            name: name.to_string(),
            project: project.into(),
            org: "acme".into(),
            classification: c,
            converted_ratio: 0.8,
            unsupported_steps: 2,
            manual_tasks: 3,
            risk_band: band,
            risk_score: score,
            status: ProposalStatus::NotStarted,
            forecast_minutes: 100,
            factors: vec![RiskFactor {
                key: "secrets".into(),
                label: "Secret variables".into(),
                contribution: 10,
                detail: "2 secrets to migrate".into(),
            }],
            reviewer: None,
            reviewed_at: None,
        }
    }

    fn portfolio() -> Portfolio {
        Portfolio {
            summary: PortfolioSummary {
                org: "acme".into(),
                importer_version: "v1.2.3".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "2026-06-12T00:00:00Z".into(),
                totals: PortfolioTotals {
                    pipelines: 2,
                    orgs: 1,
                    projects: 2,
                    yaml: 1,
                    classic: 1,
                    green: 1,
                    amber: 0,
                    red: 1,
                    forecast_minutes: 300,
                },
            },
            pipelines: vec![
                pipe(
                    "web",
                    "Storefront",
                    Classification::Yaml,
                    RiskBand::Green,
                    10,
                ),
                pipe(
                    "legacy",
                    "Payments",
                    Classification::Classic,
                    RiskBand::Red,
                    80,
                ),
            ],
            audit: PortfolioAudit {
                manual_tasks: vec![ManualTask {
                    kind: ManualTaskKind::Secret,
                    name: "DEPLOY_TOKEN".into(),
                }],
                unsupported_steps: vec![UnsupportedStep {
                    task: "DownloadSecureFile@1".into(),
                    count: 3,
                }],
                actions: vec!["actions/checkout@v4".into()],
                service_connections: vec![ServiceConnection {
                    id: "sc1".into(),
                    name: "azure-prod".into(),
                    kind: "azurerm".into(),
                    project: "Payments".into(),
                }],
                variable_groups: vec![VariableGroup {
                    id: "vg1".into(),
                    name: "shared".into(),
                    project: "Storefront".into(),
                    variables: vec![
                        VariableRef {
                            name: "API_URL".into(),
                            is_secret: false,
                        },
                        VariableRef {
                            name: "API_TOKEN".into(),
                            is_secret: true,
                        },
                    ],
                }],
            },
        }
    }

    #[test]
    fn stats_count_secrets_and_connections_to_add() {
        let s = report_stats(&portfolio(), None);
        assert_eq!(s.ready, 1);
        assert_eq!(s.high_risk, 1);
        assert_eq!(s.classic, 1);
        // DEPLOY_TOKEN (manual task) + API_TOKEN (secret variable).
        assert_eq!(s.secrets_to_add, 2);
        assert_eq!(s.connections_to_recreate, 1);
    }

    #[test]
    fn report_lists_github_setup_and_what_changes() {
        let md = report_markdown(&portfolio(), None);
        assert!(md.contains("No changes have been made"));
        assert!(md.contains("Nothing is ever pushed to the default branch"));
        // GitHub setup: the secret names + connection + actions allow-list.
        assert!(md.contains("DEPLOY_TOKEN"));
        assert!(md.contains("API_TOKEN"));
        assert!(md.contains("API_URL")); // non-secret variable
        assert!(md.contains("azure-prod"));
        assert!(md.contains("actions/checkout@v4"));
        // What-changes: the unsupported construct.
        assert!(md.contains("DownloadSecureFile@1"));
    }

    #[test]
    fn project_scope_filters_pipelines_and_setup() {
        // Scoped to Payments: only the classic pipeline + the Payments connection.
        let md = report_markdown(&portfolio(), Some("Payments"));
        assert!(md.contains("Scope: **Payments**"));
        assert!(md.contains("| legacy |"));
        assert!(!md.contains("| web |"), "Storefront pipeline excluded");
        // The azurerm connection is in Payments; the Storefront secret variable is not.
        assert!(md.contains("azure-prod"));
        assert!(
            !md.contains("API_TOKEN"),
            "Storefront secret excluded from Payments scope"
        );

        let s = report_stats(&portfolio(), Some("Payments"));
        assert_eq!(s.high_risk, 1);
        assert_eq!(s.ready, 0);
        assert_eq!(s.connections_to_recreate, 1);
    }
}
