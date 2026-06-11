//! Assembling portfolio entries from deterministic risk signals.
//!
//! This is the seam where the risk engine meets the portfolio view: given a
//! pipeline's display metadata and its [`RiskSignals`], [`build_pipeline`] runs
//! [`crate::risk::assess`] and produces the [`Pipeline`] the API/portal render —
//! so the score, band, and factor breakdown shown are *computed*, never typed in.

use crate::gap::{DryRunResult, GapKind};
use crate::model::{Classification, Pipeline, ProposalStatus};
use crate::risk::{assess, RiskSignals};

/// Display metadata for a pipeline that the risk model does not derive.
#[derive(Debug, Clone)]
pub struct PipelineMeta {
    pub id: String,
    pub name: String,
    pub project: String,
    /// Owning source org (multi-org, #156). Empty for single-org audits.
    pub org: String,
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
        org: meta.org,
        classification: signals.classification,
        converted_ratio: signals.converted_ratio,
        unsupported_steps: meta.unsupported_steps,
        manual_tasks: meta.manual_tasks,
        risk_band: assessment.band,
        risk_score: assessment.score,
        status: meta.status,
        forecast_minutes: meta.forecast_minutes,
        factors: assessment.factors,
        // Review metadata is overlaid from the proposal store at serve time.
        reviewer: None,
        reviewed_at: None,
    }
}

/// Derive deterministic [`RiskSignals`] from a pipeline's dry-run.
///
/// The dry-run's per-pipeline gaps are the authoritative source: manual-task
/// gaps name the secrets / connections / gates / runners to provision, partial
/// constructs flag matrix / artifact / conditional semantics, and namespaced
/// unsupported steps are custom/marketplace tasks. Mapping is keyword-based and
/// intentionally conservative; `classification` comes from the source pipeline.
pub fn signals_from_dry_run(dry_run: &DryRunResult, classification: Classification) -> RiskSignals {
    let mut s = RiskSignals {
        classification,
        converted_ratio: dry_run.converted_ratio,
        ..Default::default()
    };

    for gap in &dry_run.gaps {
        let text = format!("{} {}", gap.construct, gap.detail).to_ascii_lowercase();
        match gap.kind {
            GapKind::ManualTask => {
                if text.contains("secret") {
                    s.secrets += 1;
                } else if text.contains("variable") {
                    s.variable_groups += 1;
                } else if text.contains("connection") {
                    s.service_connections += 1;
                } else if text.contains("environment")
                    || text.contains("approval")
                    || text.contains("gate")
                {
                    s.approval_gates += 1;
                } else if text.contains("self-hosted")
                    || text.contains("self hosted")
                    || text.contains("runner")
                    || text.contains("pool")
                {
                    s.self_hosted_pools += 1;
                }
            }
            GapKind::UnsupportedStep => {
                // A namespaced task id (e.g. "acme-corp.deploy.DeployTask@2") is a
                // custom/marketplace task with no first-party equivalent.
                if gap.construct.contains('.') {
                    s.custom_or_marketplace_tasks += 1;
                }
            }
            GapKind::PartialConstruct => {
                if text.contains("matrix") {
                    s.uses_matrix = true;
                }
                if text.contains("artifact") {
                    s.artifact_passing = true;
                }
                if text.contains("condition")
                    || text.contains("template")
                    || text.contains("expression")
                {
                    s.complex_conditionals = true;
                }
            }
        }
    }

    s
}

/// Build a portfolio [`Pipeline`] straight from a dry-run: derive its signals,
/// assess them, and assemble the entry. This is the full
/// `dry-run → signals → assess → pipeline` path in one call.
pub fn pipeline_from_dry_run(
    meta: PipelineMeta,
    classification: Classification,
    dry_run: &DryRunResult,
) -> Pipeline {
    build_pipeline(meta, &signals_from_dry_run(dry_run, classification))
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
            org: "contoso".into(),
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

    use crate::gap::{DryRunResult, Gap, GapKind};

    fn gap(kind: GapKind, construct: &str, detail: &str) -> Gap {
        Gap {
            kind,
            construct: construct.into(),
            detail: detail.into(),
        }
    }

    fn sample_dry_run() -> DryRunResult {
        DryRunResult {
            pipeline_id: "web-portal-release".into(),
            converted_ratio: 12.0 / 14.0,
            gaps: vec![
                gap(
                    GapKind::UnsupportedStep,
                    "DownloadSecureFile@1",
                    "no equivalent",
                ),
                gap(
                    GapKind::UnsupportedStep,
                    "acme-corp.deploy.DeployTask@2",
                    "custom marketplace task",
                ),
                gap(
                    GapKind::PartialConstruct,
                    "strategy.matrix",
                    "reduced fidelity",
                ),
                gap(
                    GapKind::ManualTask,
                    "secret",
                    "AZURE_CLIENT_SECRET must be configured",
                ),
                gap(
                    GapKind::ManualTask,
                    "service-connection",
                    "azure-prod federated via OIDC",
                ),
                gap(
                    GapKind::ManualTask,
                    "environment",
                    "pre-deploy approval gate",
                ),
            ],
            converted_yaml: String::new(),
            source_yaml: String::new(),
        }
    }

    #[test]
    fn signals_are_derived_from_gaps() {
        let s = signals_from_dry_run(&sample_dry_run(), Classification::Yaml);
        assert_eq!(s.classification, Classification::Yaml);
        assert!((s.converted_ratio - 12.0 / 14.0).abs() < 1e-9);
        assert_eq!(s.secrets, 1);
        assert_eq!(s.service_connections, 1);
        assert_eq!(s.approval_gates, 1);
        assert!(s.uses_matrix);
        assert_eq!(s.custom_or_marketplace_tasks, 1); // only the namespaced task
        assert_eq!(s.self_hosted_pools, 0);
        assert_eq!(s.variable_groups, 0);
    }

    #[test]
    fn dry_run_pipeline_is_amber_for_this_fixture() {
        let p = pipeline_from_dry_run(meta(), Classification::Yaml, &sample_dry_run());
        assert_eq!(p.risk_band, crate::RiskBand::Amber);
        let keys: Vec<_> = p.factors.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"service_conn"));
        assert!(keys.contains(&"environments"));
        assert!(keys.contains(&"matrix"));
        assert!(keys.contains(&"custom"));
    }

    #[test]
    fn fully_converted_dry_run_with_no_gaps_is_green() {
        let clean = DryRunResult {
            pipeline_id: "x".into(),
            converted_ratio: 1.0,
            gaps: vec![],
            converted_yaml: String::new(),
            source_yaml: String::new(),
        };
        let p = pipeline_from_dry_run(meta(), Classification::Yaml, &clean);
        assert_eq!(p.risk_band, crate::RiskBand::Green);
        assert!(p.factors.is_empty());
    }
}
