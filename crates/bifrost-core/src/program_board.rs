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

/// The human-readable wave name (Pilot / Early majority / Late majority).
fn wave_name(wave: u8) -> &'static str {
    match wave {
        1 => "Pilot",
        2 => "Early majority",
        _ => "Late majority",
    }
}

/// Humanize a runner-minutes total into a short, management-friendly string
/// (minutes → hours → days, ~8h working days). Deterministic, no rounding drift.
fn humanize_minutes(minutes: u32) -> String {
    if minutes == 0 {
        return "0 min".to_string();
    }
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = minutes as f64 / 60.0;
    if hours < 8.0 {
        return format!("{hours:.1} h");
    }
    let days = hours / 8.0;
    format!("{days:.1} working days ({hours:.0} h)")
}

/// One wave's roll-up for the management roadmap: count, forecast, and a status
/// breakdown so the timeline reads as a management snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaveRollup {
    pub wave: u8,
    /// Pilot / Early majority / Late majority.
    pub name: String,
    pub count: u32,
    pub forecast_minutes: u32,
    pub migrated: u32,
    pub in_progress: u32,
    pub not_started: u32,
}

/// The management KPI + roadmap snapshot (#269) — a deterministic, read-only
/// export built **on top of** [`program_board_plan`]. It pairs with the
/// pre-migration PDF status report: same provenance + tone, but a management-ready
/// KPI roll-up and a roadmap-by-wave timeline rather than a per-pipeline assessment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramBoardExport {
    pub org: String,
    pub generated_at: String,
    pub importer_version: String,
    pub project_title: String,
    pub kpis: BoardKpis,
    /// Roadmap by wave, ordered Pilot → Early → Late.
    pub waves: Vec<WaveRollup>,
    pub notes: Vec<String>,
}

/// Build the deterministic management KPI + roadmap snapshot for a portfolio.
/// Built on top of [`program_board_plan`] — it never recomputes KPIs differently.
/// No I/O, no LLM.
pub fn program_board_export(portfolio: &Portfolio) -> ProgramBoardExport {
    let plan = program_board_plan(portfolio);

    // Roll the planned issues up by wave (1=Pilot, 2=Early, 3=Late). "Migrated"
    // and "in progress" mirror the same status buckets the KPI roll-up uses, so
    // the per-wave breakdown reconciles with the headline KPIs.
    let migrated_status = |s: &str| matches!(s, "Committed" | "Validated");
    let in_progress_status = |s: &str| matches!(s, "Draft" | "In review" | "Changes requested");

    let waves: Vec<WaveRollup> = [1u8, 2, 3]
        .iter()
        .map(|&wave| {
            let members: Vec<&PlannedIssue> =
                plan.issues.iter().filter(|i| i.wave == wave).collect();
            WaveRollup {
                wave,
                name: wave_name(wave).to_string(),
                count: members.len() as u32,
                forecast_minutes: members.iter().map(|i| i.forecast_minutes).sum(),
                migrated: members
                    .iter()
                    .filter(|i| migrated_status(&i.status))
                    .count() as u32,
                in_progress: members
                    .iter()
                    .filter(|i| in_progress_status(&i.status))
                    .count() as u32,
                not_started: members.iter().filter(|i| i.status == "Not started").count() as u32,
            }
        })
        .collect();

    ProgramBoardExport {
        org: portfolio.summary.org.clone(),
        generated_at: portfolio.summary.generated_at.clone(),
        importer_version: portfolio.summary.importer_version.clone(),
        project_title: plan.project_title,
        kpis: plan.kpis,
        waves,
        notes: plan.notes,
    }
}

/// Render the management KPI + roadmap snapshot as Markdown — a management-ready
/// sibling of the pre-migration status report (same provenance block + tone).
/// Deterministic, read-only.
pub fn program_board_export_markdown(portfolio: &Portfolio) -> String {
    let e = program_board_export(portfolio);
    let k = &e.kpis;
    let mut out = String::new();

    out.push_str("# Migration Program KPI & Roadmap Snapshot\n\n");
    out.push_str(&format!(
        "Organization: **{org}**  \nProject: **{project}**  \nGenerated: {at}  \nImporter: {imp}\n\n",
        org = if e.org.is_empty() {
            "(portfolio)"
        } else {
            &e.org
        },
        project = e.project_title,
        at = e.generated_at,
        imp = e.importer_version,
    ));
    out.push_str(
        "> This is a **management snapshot** for the program/steering board — a KPI roll-up and a \
         wave roadmap. It pairs with the pre-migration status report. KPIs are computed \
         deterministically by Bifrost (GitHub Projects Insights is UI-only); nothing has been \
         created on GitHub, and every change Bifrost makes is delivered as a **reviewable pull \
         request**, never a live edit.\n\n",
    );

    // --- KPI roll-up ---
    out.push_str("## KPIs\n\n");
    out.push_str("| Metric | Value |\n|---|---|\n");
    out.push_str(&format!("| Pipelines (total) | {} |\n", k.total));
    out.push_str(&format!("| Migrated | {} |\n", k.migrated));
    out.push_str(&format!("| Validated | {} |\n", k.validated));
    out.push_str(&format!("| In progress | {} |\n", k.in_progress));
    out.push_str(&format!("| Not started | {} |\n", k.not_started));
    out.push_str(&format!("| Percent done | {}% |\n", k.percent_done));
    out.push_str(&format!(
        "| Forecast runner-minutes/month | {} ({}) |\n\n",
        k.forecast_minutes,
        humanize_minutes(k.forecast_minutes),
    ));

    // --- Roadmap by wave ---
    out.push_str("## Roadmap by wave\n\n");
    out.push_str(
        "Pipelines are sequenced into waves by difficulty — pilot the easy ones, the hard tail \
         (classic/designer and high-risk) last.\n\n",
    );
    out.push_str(
        "| Wave | Cohort | Pipelines | Forecast | Migrated | In progress | Not started |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for w in &e.waves {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            w.wave,
            w.name,
            w.count,
            humanize_minutes(w.forecast_minutes),
            w.migrated,
            w.in_progress,
            w.not_started,
        ));
    }
    out.push('\n');

    // --- Notes (mirror the plan's honest guardrails) ---
    out.push_str("## Notes\n\n");
    for n in &e.notes {
        out.push_str(&format!("- {n}\n"));
    }
    out.push('\n');

    out
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

    #[test]
    fn export_kpis_match_the_plan_and_waves_group_correctly() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(
            "Contoso 0455",
            vec![
                pipe("A", Yaml, Green, Committed),   // pilot (1), migrated
                pipe("A", Classic, Red, NotStarted), // late (3), not started
                pipe("B", Yaml, Amber, Validated),   // early (2), migrated+validated
                pipe("B", Yaml, Green, InReview),    // pilot (1), in progress
            ],
        );
        let plan = program_board_plan(&p);
        let export = program_board_export(&p);

        // KPIs are taken verbatim from the plan — never recomputed differently.
        assert_eq!(export.kpis, plan.kpis);
        assert_eq!(export.org, "Contoso 0455");
        assert_eq!(export.project_title, plan.project_title);

        // Three waves, ordered Pilot → Early → Late.
        assert_eq!(export.waves.len(), 3);
        assert_eq!(export.waves[0].wave, 1);
        assert_eq!(export.waves[0].name, "Pilot");
        assert_eq!(export.waves[2].name, "Late majority");

        // Pilot wave: the Green/Committed + the Green/InReview pipeline.
        assert_eq!(export.waves[0].count, 2);
        assert_eq!(export.waves[0].migrated, 1);
        assert_eq!(export.waves[0].in_progress, 1);
        // Early wave: the Amber/Validated pipeline.
        assert_eq!(export.waves[1].count, 1);
        assert_eq!(export.waves[1].migrated, 1);
        // Late wave: the Classic/Red/NotStarted pipeline.
        assert_eq!(export.waves[2].count, 1);
        assert_eq!(export.waves[2].not_started, 1);

        // Per-wave counts reconcile with the total.
        let summed: u32 = export.waves.iter().map(|w| w.count).sum();
        assert_eq!(summed, export.kpis.total);
    }

    #[test]
    fn export_markdown_has_management_sections() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(
            "Contoso 0455",
            vec![
                pipe("A", Yaml, Green, Committed),
                pipe("A", Classic, Red, NotStarted),
            ],
        );
        let md = program_board_export_markdown(&p);
        assert!(md.contains("# Migration Program KPI & Roadmap Snapshot"));
        assert!(md.contains("Organization: **Contoso 0455**"));
        assert!(md.contains("## KPIs"));
        assert!(md.contains("Percent done"));
        assert!(md.contains("## Roadmap by wave"));
        assert!(md.contains("Pilot"));
        assert!(md.contains("Late majority"));
        assert!(md.contains("## Notes"));
        // Honest guardrail mirrored from plan.notes.
        assert!(md.contains("nothing is created on GitHub") || md.contains("dry-run plan"));
    }

    #[test]
    fn humanize_minutes_steps_through_units() {
        assert_eq!(humanize_minutes(0), "0 min");
        assert_eq!(humanize_minutes(45), "45 min");
        assert_eq!(humanize_minutes(90), "1.5 h");
        // 16 hours = 2 working days.
        assert!(humanize_minutes(960).contains("working days"));
    }
}
