//! Deterministic cost + capacity forecast for the target GitHub org.
//!
//! GitHub's golden path runs `gh actions-importer forecast` to project Actions
//! usage before migrating. Bifrost makes that projection **deterministic and
//! explainable**: cost is arithmetic on runner-minutes × a configurable rate
//! table — never the LLM (the same rule the risk model follows: the model
//! explains; it does not score, and it does not price).
//!
//! What is **computed** here from data Bifrost already has (per-pipeline
//! `forecast_minutes` from the audit): total minutes, monthly/annual cost, and a
//! per-project breakdown. What is **carried** (not fabricated): the capacity
//! figures — peak concurrency, queue time, job-duration percentiles — because
//! those require run-history timestamps that only the Importer's `forecast`
//! report provides. They are `None` until that report is wired in, and
//! illustrative in the synthetic sample portfolio.

use serde::{Deserialize, Serialize};

use crate::model::Pipeline;

/// The GitHub-hosted runner class assumed for the cost projection, with its
/// per-minute rate. Defaults to Linux x64 2-core. Rates approximate GitHub's
/// 2026 published pricing and **must be verified** against the customer's plan
/// (included free minutes, larger runners, OS multipliers, data-residency).
/// See <https://docs.github.com/en/billing/managing-billing-for-github-actions>.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerRate {
    /// Display name of the runner class (e.g. "ubuntu-latest (2-core)").
    pub class: String,
    /// USD per runner-minute. Self-hosted is `0.0` (infra cost not modelled).
    pub usd_per_minute: f64,
}

impl Default for RunnerRate {
    fn default() -> Self {
        // GitHub-hosted Linux x64 2-core, ~2026 list price. Verify before quoting.
        Self {
            class: "ubuntu-latest (2-core)".into(),
            usd_per_minute: 0.008,
        }
    }
}

/// Per-project slice of the forecast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectForecast {
    pub project: String,
    pub pipelines: u32,
    pub minutes: u32,
    pub cost_usd: f64,
}

/// Capacity figures derived from real run history (the Importer `forecast`
/// report). Carried, not computed here — `None` until that report is wired in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityForecast {
    /// Peak number of jobs running at the same moment — sizes the runner pool.
    pub peak_concurrency: u32,
    /// Median time a job waited for a runner (minutes). High = under-provisioned.
    pub median_queue_minutes: f64,
    /// Job-duration percentiles (minutes).
    pub p50_job_minutes: f64,
    pub p90_job_minutes: f64,
    pub max_job_minutes: f64,
}

/// The projected GitHub Actions cost + capacity for a portfolio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forecast {
    pub runner_class: String,
    pub usd_per_minute: f64,
    pub total_minutes: u32,
    pub monthly_cost_usd: f64,
    pub annual_cost_usd: f64,
    /// Per-project breakdown, sorted by minutes descending.
    pub by_project: Vec<ProjectForecast>,
    /// From the Importer `forecast` report; illustrative in the sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<CapacityForecast>,
    /// Assumptions a reader must know (rate basis, exclusions).
    pub notes: Vec<String>,
}

/// Round a USD amount to whole cents.
fn cents(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Compute a deterministic cost forecast from a portfolio's pipelines.
///
/// Cost = sum of per-pipeline `forecast_minutes` × `rate.usd_per_minute`,
/// assuming every minute runs on `rate.class`. Per-pipeline runner classes are
/// a future refinement (#237); today the projection is single-class with the
/// assumption stated in `notes`.
pub fn forecast(pipelines: &[Pipeline], rate: &RunnerRate) -> Forecast {
    let total_minutes: u32 = pipelines.iter().map(|p| p.forecast_minutes).sum();
    let monthly = cents(total_minutes as f64 * rate.usd_per_minute);

    // Group minutes + counts by project.
    let mut projects: Vec<ProjectForecast> = Vec::new();
    for p in pipelines {
        match projects.iter_mut().find(|pf| pf.project == p.project) {
            Some(pf) => {
                pf.pipelines += 1;
                pf.minutes += p.forecast_minutes;
            }
            None => projects.push(ProjectForecast {
                project: p.project.clone(),
                pipelines: 1,
                minutes: p.forecast_minutes,
                cost_usd: 0.0,
            }),
        }
    }
    for pf in &mut projects {
        pf.cost_usd = cents(pf.minutes as f64 * rate.usd_per_minute);
    }
    // Largest spenders first; ties broken by name for stable output.
    projects.sort_by(|a, b| {
        b.minutes
            .cmp(&a.minutes)
            .then_with(|| a.project.cmp(&b.project))
    });

    let mut notes = vec![
        format!(
            "Assumes all minutes on {} at ${:.3}/min — verify against your GitHub plan.",
            rate.class, rate.usd_per_minute
        ),
        "Excludes any included free minutes and storage; self-hosted runners incur infrastructure cost not shown here.".to_string(),
    ];
    if rate.usd_per_minute == 0.0 {
        notes.push("Self-hosted rate is $0/min: only the compute you provide is billed.".into());
    }

    Forecast {
        runner_class: rate.class.clone(),
        usd_per_minute: rate.usd_per_minute,
        total_minutes,
        monthly_cost_usd: monthly,
        annual_cost_usd: cents(monthly * 12.0),
        by_project: projects,
        capacity: None,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Classification, Pipeline, ProposalStatus, RiskBand};

    fn pipe(project: &str, minutes: u32) -> Pipeline {
        Pipeline {
            id: format!("{project}-{minutes}"),
            name: "p".into(),
            project: project.into(),
            org: String::new(),
            classification: Classification::Yaml,
            converted_ratio: 1.0,
            unsupported_steps: 0,
            manual_tasks: 0,
            risk_band: RiskBand::Green,
            risk_score: 0,
            status: ProposalStatus::Draft,
            forecast_minutes: minutes,
            factors: vec![],
            reviewer: None,
            reviewed_at: None,
        }
    }

    #[test]
    fn totals_and_cost_are_deterministic() {
        let rate = RunnerRate {
            class: "test".into(),
            usd_per_minute: 0.01,
        };
        let f = forecast(&[pipe("A", 1000), pipe("A", 500), pipe("B", 2000)], &rate);
        assert_eq!(f.total_minutes, 3500);
        assert_eq!(f.monthly_cost_usd, 35.0); // 3500 * 0.01
        assert_eq!(f.annual_cost_usd, 420.0);
    }

    #[test]
    fn per_project_is_grouped_and_sorted_by_minutes_desc() {
        let f = forecast(
            &[pipe("A", 1000), pipe("A", 500), pipe("B", 2000)],
            &RunnerRate::default(),
        );
        assert_eq!(f.by_project.len(), 2);
        assert_eq!(f.by_project[0].project, "B"); // 2000 > 1500
        assert_eq!(f.by_project[0].minutes, 2000);
        assert_eq!(f.by_project[1].project, "A");
        assert_eq!(f.by_project[1].pipelines, 2);
        assert_eq!(f.by_project[1].minutes, 1500);
    }

    #[test]
    fn empty_portfolio_is_zero() {
        let f = forecast(&[], &RunnerRate::default());
        assert_eq!(f.total_minutes, 0);
        assert_eq!(f.monthly_cost_usd, 0.0);
        assert!(f.by_project.is_empty());
    }

    #[test]
    fn cost_rounds_to_cents() {
        let rate = RunnerRate {
            class: "t".into(),
            usd_per_minute: 0.008,
        };
        // 1234 * 0.008 = 9.872 -> 9.87
        let f = forecast(&[pipe("A", 1234)], &rate);
        assert_eq!(f.monthly_cost_usd, 9.87);
    }
}
