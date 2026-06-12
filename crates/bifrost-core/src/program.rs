//! Wave / cohort planning + program dashboard (#242).
//!
//! GitHub's migration program guidance is phased: pilot a low-risk cohort, prove
//! the process, then roll out in waves, saving the hard tail for last. Bifrost
//! makes that **deterministic** — every pipeline is assigned to a wave by its
//! difficulty (classification + risk band), with per-wave risk mix, forecast
//! minutes, and lifecycle progress. No LLM; just the audit's facts.
//!
//! Waves:
//! - **1 Pilot** — green YAML pipelines: easy, prove the process.
//! - **2 Early majority** — amber YAML: standard conversions.
//! - **3 Late majority** — classic/designer or red: the hard tail, most review.

use serde::{Deserialize, Serialize};

use crate::model::{Classification, Pipeline, Portfolio, ProposalStatus, RiskBand};

/// One migration wave with its cohort's facts and progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WavePlan {
    pub wave: u8,
    pub name: String,
    pub rationale: String,
    pub pipelines: u32,
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    pub yaml: u32,
    pub classic: u32,
    pub forecast_minutes: u32,
    /// Lifecycle progress across the cohort.
    pub not_started: u32,
    pub in_progress: u32,
    pub done: u32,
    /// Distinct projects in this wave, sorted.
    pub projects: Vec<String>,
}

/// Which wave a pipeline belongs to. The hard tail (classic or red) goes last;
/// green YAML pilots first; amber YAML is the middle.
fn wave_of(p: &Pipeline) -> u8 {
    if p.classification == Classification::Classic || p.risk_band == RiskBand::Red {
        3
    } else if p.risk_band == RiskBand::Green {
        1
    } else {
        2
    }
}

fn meta(wave: u8) -> (&'static str, &'static str) {
    match wave {
        1 => (
            "Pilot",
            "Low-risk YAML pipelines — migrate these first to prove the process.",
        ),
        2 => (
            "Early majority",
            "Amber YAML pipelines — standard conversions once the pilot succeeds.",
        ),
        _ => (
            "Late majority",
            "Classic/designer and high-risk pipelines — the hard tail; needs the most review.",
        ),
    }
}

/// Build the three-wave program plan for a portfolio. Deterministic.
pub fn program(portfolio: &Portfolio) -> Vec<WavePlan> {
    [1u8, 2, 3]
        .into_iter()
        .map(|wave| {
            let (name, rationale) = meta(wave);
            let members: Vec<&Pipeline> = portfolio
                .pipelines
                .iter()
                .filter(|p| wave_of(p) == wave)
                .collect();

            let count = |pred: &dyn Fn(&Pipeline) -> bool| {
                members.iter().filter(|p| pred(p)).count() as u32
            };
            let mut projects: Vec<String> = members
                .iter()
                .map(|p| p.project.clone())
                .filter(|s| !s.is_empty())
                .collect();
            projects.sort();
            projects.dedup();

            let in_progress = count(&|p| {
                matches!(
                    p.status,
                    ProposalStatus::Draft
                        | ProposalStatus::InReview
                        | ProposalStatus::ChangesRequested
                )
            });
            let done = count(&|p| {
                matches!(
                    p.status,
                    ProposalStatus::Approved
                        | ProposalStatus::Committed
                        | ProposalStatus::Validated
                )
            });

            WavePlan {
                wave,
                name: name.to_string(),
                rationale: rationale.to_string(),
                pipelines: members.len() as u32,
                green: count(&|p| p.risk_band == RiskBand::Green),
                amber: count(&|p| p.risk_band == RiskBand::Amber),
                red: count(&|p| p.risk_band == RiskBand::Red),
                yaml: count(&|p| p.classification == Classification::Yaml),
                classic: count(&|p| p.classification == Classification::Classic),
                forecast_minutes: members.iter().map(|p| p.forecast_minutes).sum(),
                not_started: count(&|p| p.status == ProposalStatus::NotStarted),
                in_progress,
                done,
                projects,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        Classification, Pipeline, Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals,
        RiskBand,
    };

    fn pipe(project: &str, c: Classification, b: RiskBand, status: ProposalStatus) -> Pipeline {
        Pipeline {
            id: format!("{project}-{:?}-{:?}", c, b),
            name: "p".into(),
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

    fn portfolio(pipelines: Vec<Pipeline>) -> Portfolio {
        Portfolio {
            summary: PortfolioSummary {
                org: "o".into(),
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
                    forecast_minutes: 0,
                },
            },
            pipelines,
            audit: PortfolioAudit::default(),
        }
    }

    #[test]
    fn cohorts_pipelines_into_three_waves_by_difficulty() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(vec![
            pipe("A", Yaml, Green, NotStarted),  // pilot
            pipe("A", Yaml, Amber, Draft),       // early
            pipe("B", Yaml, Red, NotStarted),    // late (red)
            pipe("B", Classic, Green, Approved), // late (classic)
        ]);
        let waves = program(&p);
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].wave, 1);
        assert_eq!(waves[0].pipelines, 1); // green yaml
        assert_eq!(waves[1].pipelines, 1); // amber yaml
        assert_eq!(waves[2].pipelines, 2); // red + classic
                                           // Late wave forecast = 2 * 100.
        assert_eq!(waves[2].forecast_minutes, 200);
        assert_eq!(waves[2].projects, vec!["B".to_string()]);
    }

    #[test]
    fn progress_tally_buckets_statuses() {
        use Classification::*;
        use ProposalStatus::*;
        use RiskBand::*;
        let p = portfolio(vec![
            pipe("A", Yaml, Green, NotStarted),
            pipe("A", Yaml, Green, InReview),
            pipe("A", Yaml, Green, Committed),
        ]);
        let pilot = &program(&p)[0];
        assert_eq!(pilot.pipelines, 3);
        assert_eq!(pilot.not_started, 1);
        assert_eq!(pilot.in_progress, 1);
        assert_eq!(pilot.done, 1);
    }
}
