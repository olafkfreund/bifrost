//! Program board plan (#265) — the deterministic, **dry-run** spec of the GitHub
//! Projects board Bifrost would stand up for a migration: the dedicated repo, the
//! org Project, its custom fields, one issue per pipeline (with the migration
//! checklist as sub-issues), and the KPIs.
//!
//! This module computes only the *plan*; it performs **no GitHub writes**.
//! Provisioning (the GraphQL mutations) is a separate, approval-gated step
//! (#266). The plan is what a user reviews before approving — nothing is created
//! until then. Deterministic; no LLM.

use serde::{Deserialize, Serialize};

use crate::model::{Classification, Portfolio, ProposalStatus, RiskBand};

/// A custom field to create on the Project (drives its board/roadmap views).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardField {
    pub name: String,
    /// GraphQL field data type: `single-select`, `number`, or `date`.
    pub data_type: String,
    /// Options for a single-select field (empty otherwise).
    pub options: Vec<String>,
}

/// One issue the board would carry — one per pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedIssue {
    pub title: String,
    pub wave: u8,
    pub risk: String,
    pub status: String,
    pub forecast_minutes: u32,
    /// The migration checklist, as sub-issues.
    pub sub_issues: Vec<String>,
}

/// Bifrost-computed KPIs (Projects Insights is UI-only, so we compute our own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardKpis {
    pub total: u32,
    /// Committed or validated.
    pub migrated: u32,
    pub validated: u32,
    /// Draft / in-review / changes-requested.
    pub in_progress: u32,
    pub not_started: u32,
    pub percent_done: u32,
    pub forecast_minutes: u32,
}

/// The full dry-run plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramBoardPlan {
    pub repo: String,
    pub project_title: String,
    pub fields: Vec<BoardField>,
    pub issues: Vec<PlannedIssue>,
    pub kpis: BoardKpis,
    pub notes: Vec<String>,
}

/// The migration checklist each pipeline issue carries (the golden-path steps).
fn checklist() -> Vec<String> {
    vec![
        "Review the converted workflow (three-pane diff)".into(),
        "Create the required GitHub Actions secrets".into(),
        "Federate service connections to GitHub via OIDC".into(),
        "Provision/label any self-hosted runners".into(),
        "Validate the workflow in a sandbox (parity)".into(),
        "Approve and merge the pull request".into(),
    ]
}

fn slug(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Build the deterministic program-board plan for a portfolio. No I/O.
pub fn program_board_plan(portfolio: &Portfolio) -> ProgramBoardPlan {
    let org = &portfolio.summary.org;
    let repo = format!("{}-migration-program", slug(org));

    let fields = vec![
        BoardField {
            name: "Status".into(),
            data_type: "single-select".into(),
            options: vec![
                "Not started".into(),
                "Draft".into(),
                "In review".into(),
                "Changes requested".into(),
                "Approved".into(),
                "Committed".into(),
                "Validated".into(),
            ],
        },
        BoardField {
            name: "Wave".into(),
            data_type: "single-select".into(),
            options: vec![
                "Pilot".into(),
                "Early majority".into(),
                "Late majority".into(),
            ],
        },
        BoardField {
            name: "Risk".into(),
            data_type: "single-select".into(),
            options: vec!["Green".into(), "Amber".into(), "Red".into()],
        },
        BoardField {
            name: "Forecast minutes".into(),
            data_type: "number".into(),
            options: vec![],
        },
        BoardField {
            name: "Target date".into(),
            data_type: "date".into(),
            options: vec![],
        },
    ];

    // Wave per pipeline, by the same deterministic difficulty rule the program
    // planner uses (classification + risk): pilot the easy ones, the hard tail last.
    let wave_of = |p: &crate::model::Pipeline| -> u8 {
        if p.classification == Classification::Classic || p.risk_band == RiskBand::Red {
            3
        } else if p.risk_band == RiskBand::Green {
            1
        } else {
            2
        }
    };

    let band = |b: RiskBand| match b {
        RiskBand::Green => "Green",
        RiskBand::Amber => "Amber",
        RiskBand::Red => "Red",
    };
    let status_label = |s: ProposalStatus| match s {
        ProposalStatus::NotStarted => "Not started",
        ProposalStatus::Draft => "Draft",
        ProposalStatus::InReview => "In review",
        ProposalStatus::ChangesRequested => "Changes requested",
        ProposalStatus::Approved => "Approved",
        ProposalStatus::Committed => "Committed",
        ProposalStatus::Validated => "Validated",
    };

    let issues: Vec<PlannedIssue> = portfolio
        .pipelines
        .iter()
        .map(|p| PlannedIssue {
            title: format!("Migrate {} · {}", p.project, p.name),
            wave: wave_of(p),
            risk: band(p.risk_band).into(),
            status: status_label(p.status).into(),
            forecast_minutes: p.forecast_minutes,
            sub_issues: checklist(),
        })
        .collect();

    let count = |pred: &dyn Fn(&crate::model::Pipeline) -> bool| {
        portfolio.pipelines.iter().filter(|p| pred(p)).count() as u32
    };
    let migrated = count(&|p| {
        matches!(
            p.status,
            ProposalStatus::Committed | ProposalStatus::Validated
        )
    });
    let total = portfolio.pipelines.len() as u32;
    let kpis = BoardKpis {
        total,
        migrated,
        validated: count(&|p| p.status == ProposalStatus::Validated),
        in_progress: count(&|p| {
            matches!(
                p.status,
                ProposalStatus::Draft | ProposalStatus::InReview | ProposalStatus::ChangesRequested
            )
        }),
        not_started: count(&|p| p.status == ProposalStatus::NotStarted),
        percent_done: (migrated * 100).checked_div(total).unwrap_or(0),
        forecast_minutes: portfolio.summary.totals.forecast_minutes,
    };

    ProgramBoardPlan {
        repo,
        project_title: format!("{org} — ADO to GitHub Actions migration"),
        fields,
        issues,
        kpis,
        notes: vec![
            "This is a dry-run plan — nothing is created on GitHub until you approve provisioning.".into(),
            "Provisioning creates an org-level Project + a dedicated repo for the issues; it is idempotent and appended to the attestation log.".into(),
            "Board/roadmap views and Insights are configured once in the GitHub UI; Bifrost sets the fields that drive them.".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Classification, Pipeline, Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals,
    };

    fn pipe(project: &str, c: Classification, b: RiskBand, status: ProposalStatus) -> Pipeline {
        Pipeline {
            id: format!("{project}-x"),
            name: "CI".into(),
            project: project.into(),
            org: String::new(),
            classification: c,
            converted_ratio: 1.0,
            unsupported_steps: 0,
            manual_tasks: 0,
            risk_band: b,
            risk_score: 0,
            status,
            forecast_minutes: 100,
            factors: vec![],
            reviewer: None,
            reviewed_at: None,
        }
    }

    fn portfolio(org: &str, pipelines: Vec<Pipeline>) -> Portfolio {
        Portfolio {
            summary: PortfolioSummary {
                org: org.into(),
                importer_version: "v".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap: false,
                generated_at: "t".into(),
                totals: PortfolioTotals {
                    pipelines: pipelines.len() as u32,
                    orgs: 1,
                    projects: 1,
                    yaml: 0,
                    classic: 0,
                    green: 0,
                    amber: 0,
                    red: 0,
                    forecast_minutes: 300,
                },
            },
            pipelines,
            audit: PortfolioAudit::default(),
        }
    }

    #[test]
    fn plan_has_fields_one_issue_per_pipeline_and_kpis() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(
            "Contoso 0455",
            vec![
                pipe("A", Yaml, Green, Committed),
                pipe("A", Classic, Red, NotStarted),
                pipe("B", Yaml, Amber, Validated),
            ],
        );
        let plan = program_board_plan(&p);
        assert_eq!(plan.repo, "contoso-0455-migration-program");
        assert!(plan.fields.iter().any(|f| f.name == "Status"));
        assert_eq!(plan.issues.len(), 3);
        assert!(plan.issues[0].title.starts_with("Migrate "));
        assert!(!plan.issues[0].sub_issues.is_empty());
        // KPIs: 2 migrated (Committed + Validated) of 3 = 66%.
        assert_eq!(plan.kpis.total, 3);
        assert_eq!(plan.kpis.migrated, 2);
        assert_eq!(plan.kpis.validated, 1);
        assert_eq!(plan.kpis.not_started, 1);
        assert_eq!(plan.kpis.percent_done, 66);
    }

    #[test]
    fn waves_match_difficulty() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(
            "o",
            vec![
                pipe("A", Yaml, Green, NotStarted),    // pilot (1)
                pipe("A", Classic, Green, NotStarted), // late (3, classic)
            ],
        );
        let plan = program_board_plan(&p);
        assert_eq!(plan.issues[0].wave, 1);
        assert_eq!(plan.issues[1].wave, 3);
    }
}
