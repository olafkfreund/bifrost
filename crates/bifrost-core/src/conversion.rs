//! Assembling portfolio entries from deterministic risk signals.
//!
//! This is the seam where the risk engine meets the portfolio view: given a
//! pipeline's display metadata and its [`RiskSignals`], [`build_pipeline`] runs
//! [`crate::risk::assess`] and produces the [`Pipeline`] the API/portal render —
//! so the score, band, and factor breakdown shown are *computed*, never typed in.

use crate::model::{Pipeline, ProposalStatus};
use crate::risk::{assess, RiskSignals};

/// Display metadata for a pipeline that the risk model does not derive.
#[derive(Debug, Clone)]
pub struct PipelineMeta {
    pub id: String,
    pub name: String,
    pub project: String,
    pub status: ProposalStatus,
    pub unsupported_steps: u32,
    pub manual_tasks: u32,
    pub forecast_minutes: u32,
}

/// Build a portfolio [`Pipeline`] by assessing `signals` and merging the result
/// with `meta`. Classification and conversion ratio come from `signals` (the
/// risk inputs) so they can never disagree with the score.
pub fn build_pipeline(meta: PipelineMeta, signals: &RiskSignals) -> Pipeline {
    let assessment = assess(signals);
    Pipeline {
        id: meta.id,
        name: meta.name,
        project: meta.project,
        classification: signals.classification,
        converted_ratio: signals.converted_ratio,
        unsupported_steps: meta.unsupported_steps,
        manual_tasks: meta.manual_tasks,
        risk_band: assessment.band,
        risk_score: assessment.score,
        status: meta.status,
        forecast_minutes: meta.forecast_minutes,
        factors: assessment.factors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Classification;
    use crate::risk::assess;

    fn meta() -> PipelineMeta {
        PipelineMeta {
            id: "p".into(),
            name: "p".into(),
            project: "proj".into(),
            status: ProposalStatus::NotStarted,
            unsupported_steps: 2,
            manual_tasks: 1,
            forecast_minutes: 100,
        }
    }

    #[test]
    fn built_pipeline_carries_the_computed_assessment() {
        let signals = RiskSignals {
            classification: Classification::Classic,
            converted_ratio: 0.5,
            service_connections: 2,
            ..Default::default()
        };
        let expected = assess(&signals);
        let p = build_pipeline(meta(), &signals);

        assert_eq!(p.risk_score, expected.score);
        assert_eq!(p.risk_band, expected.band);
        assert_eq!(p.factors, expected.factors);
        // Classification + ratio mirror the signals, not separate inputs.
        assert_eq!(p.classification, Classification::Classic);
        assert_eq!(p.converted_ratio, 0.5);
    }

    #[test]
    fn clean_signals_produce_a_green_pipeline() {
        let p = build_pipeline(meta(), &RiskSignals::default());
        assert_eq!(p.risk_band, crate::RiskBand::Green);
        assert!(p.factors.is_empty());
    }
}
