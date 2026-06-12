//! Pre-migration status report (#204).
//!
//! Enterprises want a status report **before** any change — an assessment of the
//! estate to review and discuss with stakeholders before migrating. This module
//! turns an audited [`Portfolio`] into that report, as shareable Markdown plus a
//! structured [`ReportStats`] for the portal/automation.
//!
//! It is **pure and read-only**: generating a report makes no changes. It is the
//! "Prepare" step — distinct from the post-migration attestation/audit-pack. The
//! report states explicitly that every change Bifrost makes is delivered as a
//! reviewable pull request, never a live edit.

use serde::{Deserialize, Serialize};

use crate::model::{Classification, Pipeline, Portfolio, RiskBand};

/// Headline numbers derived from a portfolio for the status report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportStats {
    /// Pipelines that convert cleanly (Green): low-risk, ready to review + ship.
    pub ready: u32,
    /// Pipelines needing closer review (Amber): partial constructs / some gaps.
    pub needs_review: u32,
    /// High-risk pipelines (Red), including the classic/designer hard tail.
    pub high_risk: u32,
    /// Classic/designer pipelines — no YAML source, must be re-authored by hand.
    pub classic: u32,
    /// Total constructs the Importer could not convert across the estate.
    pub total_unsupported_steps: u32,
    /// Total manual tasks (secrets, service connections, environments, …).
    pub total_manual_tasks: u32,
    /// Forecast GitHub Actions runner-minutes/month across the estate.
    pub forecast_minutes: u32,
}

/// Derive the headline status numbers from `portfolio`.
pub fn report_stats(portfolio: &Portfolio) -> ReportStats {
    let band = |b: RiskBand| {
        portfolio
            .pipelines
            .iter()
            .filter(|p| p.risk_band == b)
            .count() as u32
    };
    ReportStats {
        ready: band(RiskBand::Green),
        needs_review: band(RiskBand::Amber),
        high_risk: band(RiskBand::Red),
        classic: portfolio
            .pipelines
            .iter()
            .filter(|p| p.classification == Classification::Classic)
            .count() as u32,
        total_unsupported_steps: portfolio
            .pipelines
            .iter()
            .map(|p| p.unsupported_steps)
            .sum(),
        total_manual_tasks: portfolio.pipelines.iter().map(|p| p.manual_tasks).sum(),
        forecast_minutes: portfolio.summary.totals.forecast_minutes,
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

/// Render the pre-migration status report as Markdown for `portfolio`.
///
/// Read-only: this reflects the audited estate; **no changes are made**. Suitable
/// to share with stakeholders before a migration begins.
pub fn report_markdown(portfolio: &Portfolio) -> String {
    let s = &portfolio.summary;
    let t = &s.totals;
    let stats = report_stats(portfolio);
    let mut out = String::new();

    out.push_str("# Migration Status Report\n\n");
    out.push_str(&format!(
        "Organization: **{org}**  \nGenerated: {at}  \nImporter: {imp}\n\n",
        org = if s.org.is_empty() {
            "(portfolio)"
        } else {
            &s.org
        },
        at = s.generated_at,
        imp = s.importer_version,
    ));

    out.push_str(
        "> This is a **pre-migration assessment**. No changes have been made to any pipeline. \
         Bifrost recommends and explains; every change is delivered as a **reviewable pull \
         request** to the source repository — never a live edit — and only after a human \
         approves it.\n\n",
    );

    out.push_str("## Portfolio overview\n\n");
    out.push_str(&format!(
        "- Source orgs: **{}**\n- Projects: **{}**\n- Pipelines: **{}** ({} YAML, {} classic/designer)\n- Forecast runner-minutes/month: **{}**\n\n",
        t.orgs.max(1),
        t.projects,
        t.pipelines,
        t.yaml,
        t.classic,
        stats.forecast_minutes,
    ));

    out.push_str("## Migration readiness\n\n");
    out.push_str(&format!(
        "| Band | Pipelines | Meaning |\n|---|---|---|\n\
         | Green | {} | Converts cleanly — review and ship. |\n\
         | Amber | {} | Partial constructs or gaps — needs closer review. |\n\
         | Red | {} | High risk, incl. the classic/designer hard tail. |\n\n",
        stats.ready, stats.needs_review, stats.high_risk,
    ));
    out.push_str(&format!(
        "Across the estate: **{}** constructs the Importer cannot convert, and **{}** manual \
         tasks (secrets, service connections, environments). **{}** classic/designer pipelines \
         have no YAML source and must be re-authored by hand.\n\n",
        stats.total_unsupported_steps, stats.total_manual_tasks, stats.classic,
    ));

    out.push_str("## Per-pipeline assessment\n\n");
    out.push_str(
        "| Pipeline | Project | Type | Risk | Score | Unsupported | Manual tasks |\n\
         |---|---|---|---|---|---|---|\n",
    );
    let mut pipelines: Vec<&Pipeline> = portfolio.pipelines.iter().collect();
    // Highest-risk first so reviewers see the hard cases up top.
    pipelines.sort_by(|a, b| b.risk_score.cmp(&a.risk_score).then(a.name.cmp(&b.name)));
    for p in pipelines {
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

    out.push_str("## How changes are delivered\n\n");
    out.push_str(
        "Bifrost follows a **review-first, Prepare → Act → Reflect → Review** workflow:\n\n\
         1. **Prepare** — this report: assess the estate and agree scope before any change.\n\
         2. **Act** — convert each pipeline (the Importer plus grounded gap-fill); the result \
         is a *proposal*, not a change.\n\
         3. **Reflect** — a human reviews the three-pane diff, risk, and rationale, and \
         approves or requests changes.\n\
         4. **Review** — an approved proposal is committed to a **new branch and opened as a \
         pull request** in the source repository. Nothing is ever pushed to the default \
         branch; your existing review and CI gates apply to the PR.\n\n\
         No production CI is rewritten silently. Every state transition is recorded to an \
         immutable audit log, and a signed attestation is available after validation.\n",
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Pipeline, PortfolioSummary, PortfolioTotals, ProposalStatus};

    fn pipe(
        name: &str,
        c: Classification,
        band: RiskBand,
        score: i32,
        unsup: u32,
        manual: u32,
    ) -> Pipeline {
        Pipeline {
            id: name.to_string(),
            name: name.to_string(),
            project: "Storefront".into(),
            org: "acme".into(),
            classification: c,
            converted_ratio: 0.8,
            unsupported_steps: unsup,
            manual_tasks: manual,
            risk_band: band,
            risk_score: score,
            status: ProposalStatus::NotStarted,
            forecast_minutes: 100,
            factors: vec![],
            reviewer: None,
            reviewed_at: None,
        }
    }

    fn portfolio() -> Portfolio {
        let pipelines = vec![
            pipe("web", Classification::Yaml, RiskBand::Green, 10, 0, 1),
            pipe("payments", Classification::Yaml, RiskBand::Amber, 45, 2, 3),
            pipe("legacy", Classification::Classic, RiskBand::Red, 80, 5, 4),
        ];
        Portfolio {
            summary: PortfolioSummary {
                org: "acme".into(),
                importer_version: "v1.2.3".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "2026-06-12T00:00:00Z".into(),
                totals: PortfolioTotals {
                    pipelines: 3,
                    orgs: 1,
                    projects: 1,
                    yaml: 2,
                    classic: 1,
                    green: 1,
                    amber: 1,
                    red: 1,
                    forecast_minutes: 300,
                },
            },
            pipelines,
        }
    }

    #[test]
    fn stats_aggregate_the_estate() {
        let s = report_stats(&portfolio());
        assert_eq!(s.ready, 1);
        assert_eq!(s.needs_review, 1);
        assert_eq!(s.high_risk, 1);
        assert_eq!(s.classic, 1);
        assert_eq!(s.total_unsupported_steps, 7);
        assert_eq!(s.total_manual_tasks, 8);
        assert_eq!(s.forecast_minutes, 300);
    }

    #[test]
    fn markdown_states_no_change_and_pr_delivery() {
        let md = report_markdown(&portfolio());
        // The pre-change disclaimer and PR-only delivery must be explicit.
        assert!(md.contains("No changes have been made"));
        assert!(md.contains("reviewable pull request"));
        assert!(md.contains("Nothing is ever pushed to the default branch"));
        assert!(md.contains("Prepare → Act → Reflect → Review"));
    }

    #[test]
    fn markdown_lists_pipelines_highest_risk_first() {
        let md = report_markdown(&portfolio());
        // Every pipeline appears.
        for name in ["web", "payments", "legacy"] {
            assert!(md.contains(name), "missing {name}");
        }
        // The Red pipeline (highest score) sorts above the Green one.
        let legacy = md.find("legacy").unwrap();
        let web = md.find("| web |").unwrap();
        assert!(legacy < web, "highest-risk pipeline should come first");
    }
}
