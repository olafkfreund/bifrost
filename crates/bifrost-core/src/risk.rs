//! Deterministic risk model.
//!
//! The score is computed here from explainable, weighted factors (plan §6) and
//! mapped onto a Green/Amber/Red band. The LLM never touches the score, the
//! band, or the factor weights — it only explains them downstream. Given the
//! same [`RiskSignals`], [`assess`] always returns the same result.

use crate::model::{Classification, RiskBand, RiskFactor};

/// Upper bound (exclusive) of the Green band.
pub const AMBER_THRESHOLD: i32 = 34;
/// Lower bound (inclusive) of the Red band.
pub const RED_THRESHOLD: i32 = 67;

/// Per-factor weights, named so they can be tuned and reasoned about.
pub mod weights {
    /// Classic/designer pipelines have no YAML source — the hard tail. Weighted
    /// above the amber threshold so a classic pipeline is never auto-Green.
    pub const CLASSIC: i32 = 35;
    /// Full weight applied to the share of steps the Importer left unconverted.
    pub const UNSUPPORTED_FULL: i32 = 30;
    pub const SERVICE_CONNECTION_EACH: i32 = 9;
    pub const SERVICE_CONNECTION_CAP: i32 = 22;
    pub const PROVISIONING_BASE: i32 = 8;
    pub const PROVISIONING_PER_EXTRA: i32 = 2;
    pub const PROVISIONING_CAP: i32 = 14;
    pub const SELF_HOSTED: i32 = 16;
    pub const APPROVAL_GATE: i32 = 14;
    pub const MATRIX: i32 = 10;
    pub const CUSTOM_TASK_EACH: i32 = 5;
    pub const CUSTOM_TASK_CAP: i32 = 16;
    pub const ARTIFACT_PASSING: i32 = 8;
    pub const COMPLEX_CONDITIONALS: i32 = 8;
}

/// Map a deterministic risk score (0–100) onto a band.
///
/// Mirrors the thresholds used by the portal so server- and client-derived
/// bands never disagree.
pub fn band_for_score(score: i32) -> RiskBand {
    if score >= RED_THRESHOLD {
        RiskBand::Red
    } else if score >= AMBER_THRESHOLD {
        RiskBand::Amber
    } else {
        RiskBand::Green
    }
}

/// Deterministic inputs to the risk model for a single pipeline.
///
/// These come from the Importer dry-run (conversion ratio, unsupported steps)
/// plus the source-adapter inventory (connections, secrets, runners, gates).
/// Build with [`RiskSignals::default`] (a clean YAML pipeline) and set fields.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskSignals {
    pub classification: Classification,
    /// Share of steps the Importer converted automatically (0.0–1.0).
    pub converted_ratio: f64,
    pub secrets: u32,
    pub variable_groups: u32,
    pub service_connections: u32,
    pub self_hosted_pools: u32,
    pub approval_gates: u32,
    pub uses_matrix: bool,
    /// Custom/marketplace tasks with no first-party GHA equivalent.
    pub custom_or_marketplace_tasks: u32,
    pub artifact_passing: bool,
    pub complex_conditionals: bool,
}

impl Default for RiskSignals {
    fn default() -> Self {
        Self {
            classification: Classification::Yaml,
            converted_ratio: 1.0,
            secrets: 0,
            variable_groups: 0,
            service_connections: 0,
            self_hosted_pools: 0,
            approval_gates: 0,
            uses_matrix: false,
            custom_or_marketplace_tasks: 0,
            artifact_passing: false,
            complex_conditionals: false,
        }
    }
}

/// The deterministic outcome: a capped 0–100 score, its band, and the factor
/// breakdown that produced it (the same `RiskFactor` shape the portal renders).
#[derive(Debug, Clone, PartialEq)]
pub struct RiskAssessment {
    pub score: i32,
    pub band: RiskBand,
    pub factors: Vec<RiskFactor>,
}

fn factor(key: &str, label: &str, contribution: i32, detail: String) -> RiskFactor {
    RiskFactor {
        key: key.into(),
        label: label.into(),
        contribution,
        detail,
    }
}

/// Compute the deterministic risk assessment for one pipeline.
///
/// The score is the sum of factor contributions, capped at 100. Only non-zero
/// contributors appear in the breakdown.
pub fn assess(s: &RiskSignals) -> RiskAssessment {
    use weights as w;
    let mut factors = Vec::new();

    if s.classification == Classification::Classic {
        factors.push(factor(
            "classic",
            "Classic/designer pipeline",
            w::CLASSIC,
            "No YAML source — requires manual rebuild".into(),
        ));
    }

    let unconverted = (1.0 - s.converted_ratio.clamp(0.0, 1.0)) * w::UNSUPPORTED_FULL as f64;
    let unconverted = unconverted.round() as i32;
    if unconverted > 0 {
        let pct = ((1.0 - s.converted_ratio.clamp(0.0, 1.0)) * 100.0).round() as i32;
        factors.push(factor(
            "unsupported",
            "Unsupported steps",
            unconverted,
            format!("Importer left {pct}% of steps unconverted"),
        ));
    }

    if s.service_connections > 0 {
        let c = (s.service_connections as i32 * w::SERVICE_CONNECTION_EACH)
            .min(w::SERVICE_CONNECTION_CAP);
        factors.push(factor(
            "service_conn",
            "Service connections",
            c,
            format!(
                "{} connection(s) → OIDC federation to GitHub",
                s.service_connections
            ),
        ));
    }

    let provisioning = s.secrets + s.variable_groups;
    if provisioning > 0 {
        let c = (w::PROVISIONING_BASE + (provisioning as i32 - 1) * w::PROVISIONING_PER_EXTRA)
            .min(w::PROVISIONING_CAP);
        factors.push(factor(
            "secrets",
            "Secrets / variable groups",
            c,
            format!(
                "{} secret(s)/variable group(s) → repo/org secrets to provision",
                provisioning
            ),
        ));
    }

    if s.self_hosted_pools > 0 {
        factors.push(factor(
            "selfhosted",
            "Self-hosted pools",
            w::SELF_HOSTED,
            "Self-hosted agent pool → runner strategy decision".into(),
        ));
    }

    if s.approval_gates > 0 {
        factors.push(factor(
            "environments",
            "Approval gates",
            w::APPROVAL_GATE,
            "Deployment gates → Environments + required reviewers".into(),
        ));
    }

    if s.uses_matrix {
        factors.push(factor(
            "matrix",
            "Matrix semantics",
            w::MATRIX,
            "Multi-config matrix differs from GHA strategy.matrix".into(),
        ));
    }

    if s.custom_or_marketplace_tasks > 0 {
        let c =
            (s.custom_or_marketplace_tasks as i32 * w::CUSTOM_TASK_EACH).min(w::CUSTOM_TASK_CAP);
        factors.push(factor(
            "custom",
            "Custom / marketplace tasks",
            c,
            format!(
                "{} task(s) with no first-party GHA equivalent",
                s.custom_or_marketplace_tasks
            ),
        ));
    }

    if s.artifact_passing {
        factors.push(factor(
            "artifacts",
            "Artifact passing",
            w::ARTIFACT_PASSING,
            "Publish/download semantics → actions/upload|download-artifact".into(),
        ));
    }

    if s.complex_conditionals {
        factors.push(factor(
            "conditionals",
            "Complex conditionals",
            w::COMPLEX_CONDITIONALS,
            "Conditional expressions / template expansion need review".into(),
        ));
    }

    let score = factors.iter().map(|f| f.contribution).sum::<i32>().min(100);
    RiskAssessment {
        score,
        band: band_for_score(score),
        factors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_partition_the_score_range() {
        assert_eq!(band_for_score(0), RiskBand::Green);
        assert_eq!(band_for_score(33), RiskBand::Green);
        assert_eq!(band_for_score(34), RiskBand::Amber);
        assert_eq!(band_for_score(66), RiskBand::Amber);
        assert_eq!(band_for_score(67), RiskBand::Red);
        assert_eq!(band_for_score(100), RiskBand::Red);
    }

    #[test]
    fn clean_yaml_pipeline_is_green_with_no_factors() {
        let a = assess(&RiskSignals::default());
        assert_eq!(a.score, 0);
        assert_eq!(a.band, RiskBand::Green);
        assert!(a.factors.is_empty());
    }

    #[test]
    fn classic_alone_lands_amber() {
        let a = assess(&RiskSignals {
            classification: Classification::Classic,
            ..Default::default()
        });
        assert_eq!(a.score, weights::CLASSIC);
        assert_eq!(a.band, RiskBand::Amber);
        assert_eq!(a.factors.len(), 1);
        assert_eq!(a.factors[0].key, "classic");
    }

    #[test]
    fn classic_release_with_connections_and_gates_is_red() {
        let a = assess(&RiskSignals {
            classification: Classification::Classic,
            converted_ratio: 0.4,
            service_connections: 3,
            approval_gates: 1,
            custom_or_marketplace_tasks: 2,
            ..Default::default()
        });
        assert_eq!(a.band, RiskBand::Red);
        // Has a factor for each driver.
        let keys: Vec<_> = a.factors.iter().map(|f| f.key.as_str()).collect();
        assert!(keys.contains(&"classic"));
        assert!(keys.contains(&"unsupported"));
        assert!(keys.contains(&"service_conn"));
        assert!(keys.contains(&"environments"));
        assert!(keys.contains(&"custom"));
    }

    #[test]
    fn score_is_the_capped_sum_of_contributions() {
        let a = assess(&RiskSignals {
            classification: Classification::Classic,
            converted_ratio: 0.0,
            service_connections: 9,
            secrets: 5,
            self_hosted_pools: 1,
            approval_gates: 1,
            uses_matrix: true,
            custom_or_marketplace_tasks: 9,
            artifact_passing: true,
            complex_conditionals: true,
            variable_groups: 2,
        });
        let raw: i32 = a.factors.iter().map(|f| f.contribution).sum();
        assert!(
            raw > 100,
            "raw sum should exceed the cap in this worst case"
        );
        assert_eq!(a.score, 100);
        assert_eq!(a.band, RiskBand::Red);
    }

    #[test]
    fn adding_a_risk_signal_never_lowers_the_score() {
        let base = assess(&RiskSignals::default()).score;
        let with_matrix = assess(&RiskSignals {
            uses_matrix: true,
            ..Default::default()
        })
        .score;
        assert!(with_matrix >= base);
        assert_eq!(with_matrix, weights::MATRIX);
    }

    #[test]
    fn assessment_is_deterministic() {
        let s = RiskSignals {
            classification: Classification::Classic,
            converted_ratio: 0.6,
            service_connections: 2,
            ..Default::default()
        };
        assert_eq!(assess(&s), assess(&s));
    }

    #[test]
    fn service_connection_contribution_is_capped() {
        let a = assess(&RiskSignals {
            service_connections: 100,
            ..Default::default()
        });
        let sc = a.factors.iter().find(|f| f.key == "service_conn").unwrap();
        assert_eq!(sc.contribution, weights::SERVICE_CONNECTION_CAP);
    }
}
