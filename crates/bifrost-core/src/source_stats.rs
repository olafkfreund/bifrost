//! Source (Azure DevOps) assessment statistics (#240) — "status before we start."
//!
//! A **deterministic** aggregation over the audit data: pipeline mix, risk
//! distribution, and the inventory density (how many service connections,
//! variable groups, secrets, runners, custom tasks) a migration program has to
//! account for. Stats Bifrost does not yet collect (run history / dormancy,
//! success-rate baseline, owning team, repo size for GEI) are listed honestly in
//! `uncollected` rather than faked — the same rule as the completeness matrix.

use serde::{Deserialize, Serialize};

use crate::audit::ManualTaskKind;
use crate::model::{Classification, Portfolio, RiskBand};

/// One project's source-side counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStat {
    pub project: String,
    pub pipelines: u32,
    pub yaml: u32,
    pub classic: u32,
    pub red: u32,
}

/// The source assessment for a portfolio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStats {
    pub org: String,
    pub pipelines: u32,
    pub projects: u32,
    pub yaml: u32,
    pub classic: u32,
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    pub forecast_minutes: u32,
    // --- inventory density: what must be recreated/accounted for in GitHub ---
    pub service_connections: u32,
    pub variable_groups: u32,
    pub secrets: u32,
    pub self_hosted_runners: u32,
    /// Distinct unsupported/custom task types needing a first-party Action or rework.
    pub custom_task_types: u32,
    pub actions_allowlist: u32,
    pub by_project: Vec<ProjectStat>,
    /// Assessment data Bifrost does not yet collect — listed so a program knows
    /// what is still unknown before starting.
    pub uncollected: Vec<String>,
}

/// Aggregate the source-side assessment. Deterministic.
pub fn source_stats(portfolio: &Portfolio) -> SourceStats {
    let t = &portfolio.summary.totals;
    let a = &portfolio.audit;

    let secrets = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::Secret)
        .count() as u32;
    let self_hosted_runners = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::SelfHostedRunner)
        .count() as u32;

    // Per-project counts.
    let mut by_project: Vec<ProjectStat> = Vec::new();
    for p in &portfolio.pipelines {
        let entry = match by_project.iter_mut().find(|s| s.project == p.project) {
            Some(s) => s,
            None => {
                by_project.push(ProjectStat {
                    project: p.project.clone(),
                    pipelines: 0,
                    yaml: 0,
                    classic: 0,
                    red: 0,
                });
                by_project.last_mut().unwrap()
            }
        };
        entry.pipelines += 1;
        match p.classification {
            Classification::Yaml => entry.yaml += 1,
            Classification::Classic => entry.classic += 1,
        }
        if p.risk_band == RiskBand::Red {
            entry.red += 1;
        }
    }
    // Most pipelines first; ties by name for stable output.
    by_project.sort_by(|a, b| {
        b.pipelines
            .cmp(&a.pipelines)
            .then_with(|| a.project.cmp(&b.project))
    });

    SourceStats {
        org: portfolio.summary.org.clone(),
        pipelines: t.pipelines,
        projects: t.projects,
        yaml: t.yaml,
        classic: t.classic,
        green: t.green,
        amber: t.amber,
        red: t.red,
        forecast_minutes: t.forecast_minutes,
        service_connections: a.service_connections.len() as u32,
        variable_groups: a.variable_groups.len() as u32,
        secrets,
        self_hosted_runners,
        custom_task_types: a.unsupported_steps.len() as u32,
        actions_allowlist: a.actions.len() as u32,
        by_project,
        uncollected: vec![
            "Last-run date / dormant vs active pipelines".into(),
            "Historical success-rate and build-duration baseline".into(),
            "Owning team per pipeline".into(),
            "Repository size vs GEI limits (40 GiB / 400 MiB)".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{ManualTask, UnsupportedStep};
    use crate::ingestion::ServiceConnection;
    use crate::model::{
        Classification, Pipeline, PortfolioAudit, PortfolioSummary, PortfolioTotals,
        ProposalStatus, RiskBand,
    };

    fn pipe(project: &str, classification: Classification, band: RiskBand) -> Pipeline {
        Pipeline {
            id: format!("{project}-x"),
            name: "p".into(),
            project: project.into(),
            org: String::new(),
            classification,
            converted_ratio: 1.0,
            unsupported_steps: 0,
            manual_tasks: 0,
            risk_band: band,
            risk_score: 0,
            status: ProposalStatus::Draft,
            forecast_minutes: 100,
            factors: vec![],
            reviewer: None,
            reviewed_at: None,
        }
    }

    #[test]
    fn aggregates_mix_density_and_per_project() {
        let pipelines = vec![
            pipe("A", Classification::Yaml, RiskBand::Green),
            pipe("A", Classification::Classic, RiskBand::Red),
            pipe("B", Classification::Yaml, RiskBand::Amber),
        ];
        let audit = PortfolioAudit {
            manual_tasks: vec![
                ManualTask {
                    kind: ManualTaskKind::Secret,
                    name: "S".into(),
                },
                ManualTask {
                    kind: ManualTaskKind::SelfHostedRunner,
                    name: "R".into(),
                },
            ],
            unsupported_steps: vec![UnsupportedStep {
                task: "DownloadSecureFile@1".into(),
                count: 2,
            }],
            actions: vec!["actions/checkout@v4".into()],
            service_connections: vec![ServiceConnection {
                id: "1".into(),
                name: "c".into(),
                kind: "azurerm".into(),
                project: "A".into(),
            }],
            variable_groups: vec![],
            forecast_capacity: None,
        };
        let portfolio = Portfolio {
            summary: PortfolioSummary {
                org: "acme".into(),
                importer_version: "v".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "t".into(),
                totals: PortfolioTotals {
                    pipelines: 3,
                    orgs: 1,
                    projects: 2,
                    yaml: 2,
                    classic: 1,
                    green: 1,
                    amber: 1,
                    red: 1,
                    forecast_minutes: 300,
                },
            },
            pipelines,
            audit,
        };

        let s = source_stats(&portfolio);
        assert_eq!(s.org, "acme");
        assert_eq!(s.secrets, 1);
        assert_eq!(s.self_hosted_runners, 1);
        assert_eq!(s.service_connections, 1);
        assert_eq!(s.custom_task_types, 1);
        assert_eq!(s.actions_allowlist, 1);
        // Project A has 2 pipelines (1 classic, 1 red) and sorts first.
        assert_eq!(s.by_project[0].project, "A");
        assert_eq!(s.by_project[0].pipelines, 2);
        assert_eq!(s.by_project[0].classic, 1);
        assert_eq!(s.by_project[0].red, 1);
        assert!(!s.uncollected.is_empty());
    }
}
