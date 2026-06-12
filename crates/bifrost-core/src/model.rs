//! Domain model for the portfolio view.
//!
//! The serde representation is the JSON contract the portal consumes (see
//! `portal/src/types.ts`). Field names are camelCase and enum variants are
//! snake_case to match the TypeScript types exactly.

use serde::{Deserialize, Serialize};

/// Deterministic risk band. Computed from factors by [`crate::risk`], never by the LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskBand {
    Green,
    Amber,
    Red,
}

/// ADO pipeline kind. Classic/designer pipelines are the hard tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    Yaml,
    Classic,
}

/// Where a converted pipeline sits in the review lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    NotStarted,
    Draft,
    InReview,
    ChangesRequested,
    Approved,
    Committed,
    Validated,
}

/// A single migration-risk factor and its weighted contribution to the score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskFactor {
    pub key: String,
    pub label: String,
    /// Weighted contribution to the deterministic score (0–100 scale).
    pub contribution: i32,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub project: String,
    /// Owning source org (multi-org, #156). Empty for single-org audits.
    #[serde(default)]
    pub org: String,
    pub classification: Classification,
    /// Share of steps the Importer converted automatically (0–1).
    pub converted_ratio: f64,
    pub unsupported_steps: u32,
    pub manual_tasks: u32,
    pub risk_band: RiskBand,
    pub risk_score: i32,
    pub status: ProposalStatus,
    /// Forecast Actions runner-minutes/month for this pipeline.
    pub forecast_minutes: u32,
    pub factors: Vec<RiskFactor>,
    /// Who last acted on the proposal (from the latest audit event), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,
    /// When the proposal was last acted on (ISO-8601), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioTotals {
    pub pipelines: u32,
    /// Distinct source orgs across the portfolio (multi-org, #156).
    #[serde(default)]
    pub orgs: u32,
    pub projects: u32,
    pub yaml: u32,
    pub classic: u32,
    pub green: u32,
    pub amber: u32,
    pub red: u32,
    pub forecast_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSummary {
    pub org: String,
    /// Pinned tool provenance, recorded per audit run for attestation.
    pub importer_version: String,
    /// Immutable Importer image digest (`repo@sha256:…`) used for this run, so the
    /// conversion is reproducible even if the image tag later moves (#30).
    #[serde(default)]
    pub importer_image_digest: String,
    pub ado2gh_version: String,
    pub air_gap: bool,
    pub generated_at: String,
    pub totals: PortfolioTotals,
}

/// The audit detail a change-management report needs (#220): the names of the
/// secrets/variables/connections to set up in GitHub, the constructs that need
/// manual rework, and the Actions allow-list — beyond the headline counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAudit {
    /// Manual tasks (secret names, self-hosted runners) the Importer flagged.
    pub manual_tasks: Vec<crate::audit::ManualTask>,
    /// Unsupported constructs the Importer could not convert (need rework).
    pub unsupported_steps: Vec<crate::audit::UnsupportedStep>,
    /// The GitHub Actions allow-list the converted workflows require.
    pub actions: Vec<String>,
    /// Service connections / integrations to recreate in GitHub (names + types,
    /// per project — never secret values).
    pub service_connections: Vec<crate::ingestion::ServiceConnection>,
    /// Variable groups (names + secret flag, per project) to add to GitHub.
    pub variable_groups: Vec<crate::ingestion::VariableGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub summary: PortfolioSummary,
    pub pipelines: Vec<Pipeline>,
    /// Audit detail for the change-management report (#220). Empty for the
    /// sample/offline portfolio; populated by a live audit.
    #[serde(default)]
    pub audit: PortfolioAudit,
}

impl Portfolio {
    /// Build a [`PortfolioTotals`] by aggregating the given pipelines.
    pub fn totals_from(pipelines: &[Pipeline]) -> PortfolioTotals {
        let count =
            |pred: &dyn Fn(&Pipeline) -> bool| pipelines.iter().filter(|p| pred(p)).count() as u32;
        let distinct = |key: &dyn Fn(&Pipeline) -> &str| {
            let mut v: Vec<&str> = pipelines
                .iter()
                .map(key)
                .filter(|s| !s.is_empty())
                .collect();
            v.sort_unstable();
            v.dedup();
            v.len() as u32
        };
        PortfolioTotals {
            pipelines: pipelines.len() as u32,
            orgs: distinct(&|p| p.org.as_str()),
            projects: distinct(&|p| p.project.as_str()),
            yaml: count(&|p| p.classification == Classification::Yaml),
            classic: count(&|p| p.classification == Classification::Classic),
            green: count(&|p| p.risk_band == RiskBand::Green),
            amber: count(&|p| p.risk_band == RiskBand::Amber),
            red: count(&|p| p.risk_band == RiskBand::Red),
            forecast_minutes: pipelines.iter().map(|p| p.forecast_minutes).sum(),
        }
    }
}
