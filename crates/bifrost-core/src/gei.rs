//! Repo + pipeline coordination (#245).
//!
//! GitHub's migration is two independent tools run as one program: repositories
//! move via GEI (`ado2gh`), pipelines via the Importer. Bifrost tracks them
//! together, **per project**, so a program manager sees both halves at once.
//!
//! The pipeline half is real (from the portfolio). The repo half needs a GEI
//! inventory (`ado2gh inventory-report`: repo list, sizes vs the 40 GiB / 400 MiB
//! limits, PR counts) which Bifrost does not yet collect — so it is reported as
//! `PendingInventory` rather than faked, the same honesty rule as the Coverage
//! matrix. Deterministic; no LLM.

use serde::{Deserialize, Serialize};

use crate::model::{Portfolio, ProposalStatus};

/// Where a project's repository migration (GEI) stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepoStatus {
    /// Bifrost does not yet inventory repos — run `ado2gh inventory-report`.
    PendingInventory,
}

/// One project's combined repo + pipeline migration state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCoordination {
    pub project: String,
    pub pipelines: u32,
    pub pipelines_done: u32,
    pub repo_status: RepoStatus,
}

/// Build the per-project repo+pipeline coordination. Deterministic.
pub fn gei_coordination(portfolio: &Portfolio) -> Vec<ProjectCoordination> {
    let mut rows: Vec<ProjectCoordination> = Vec::new();
    for p in &portfolio.pipelines {
        let done = matches!(
            p.status,
            ProposalStatus::Approved | ProposalStatus::Committed | ProposalStatus::Validated
        );
        match rows.iter_mut().find(|r| r.project == p.project) {
            Some(r) => {
                r.pipelines += 1;
                if done {
                    r.pipelines_done += 1;
                }
            }
            None => rows.push(ProjectCoordination {
                project: p.project.clone(),
                pipelines: 1,
                pipelines_done: u32::from(done),
                repo_status: RepoStatus::PendingInventory,
            }),
        }
    }
    rows.sort_by(|a, b| {
        b.pipelines
            .cmp(&a.pipelines)
            .then_with(|| a.project.cmp(&b.project))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Classification, Pipeline, Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals,
        RiskBand,
    };

    fn pipe(project: &str, status: ProposalStatus) -> Pipeline {
        Pipeline {
            id: format!("{project}-{status:?}"),
            name: "p".into(),
            project: project.into(),
            org: String::new(),
            classification: Classification::Yaml,
            converted_ratio: 1.0,
            unsupported_steps: 0,
            manual_tasks: 0,
            risk_band: RiskBand::Green,
            risk_score: 0,
            status,
            forecast_minutes: 0,
            factors: vec![],
            reviewer: None,
            reviewed_at: None,
        }
    }

    #[test]
    fn coordinates_pipeline_progress_per_project_with_repos_pending() {
        use ProposalStatus::*;
        let portfolio = Portfolio {
            summary: PortfolioSummary {
                org: "o".into(),
                importer_version: "v".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "t".into(),
                totals: PortfolioTotals {
                    pipelines: 3,
                    orgs: 1,
                    projects: 2,
                    yaml: 3,
                    classic: 0,
                    green: 3,
                    amber: 0,
                    red: 0,
                    forecast_minutes: 0,
                },
            },
            pipelines: vec![
                pipe("A", Committed),
                pipe("A", NotStarted),
                pipe("B", Validated),
            ],
            audit: PortfolioAudit::default(),
        };
        let rows = gei_coordination(&portfolio);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].project, "A"); // 2 pipelines, sorts first
        assert_eq!(rows[0].pipelines, 2);
        assert_eq!(rows[0].pipelines_done, 1);
        assert_eq!(rows[0].repo_status, RepoStatus::PendingInventory);
        assert_eq!(rows[1].pipelines_done, 1); // B: Validated
    }
}
