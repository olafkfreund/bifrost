//! Gaps — the constructs the Importer could not fully convert.
//!
//! A [`Gap`] is what the dry-run leaves behind: an unsupported step, a partial
//! construct, or a manual task. Gaps are the grounded input the LLM layer fills
//! later (source snippet + Importer message), and they feed the deterministic
//! risk signals. They never carry secret values — only names and messages.

use serde::{Deserialize, Serialize};

/// What kind of gap the Importer reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapKind {
    /// A build step with no GitHub Actions equivalent.
    UnsupportedStep,
    /// A construct the Importer converted only partially.
    PartialConstruct,
    /// Something a human must do (provision a secret, federate a connection,
    /// recreate an approval gate, choose a runner).
    ManualTask,
}

/// A single gap from a pipeline's dry-run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub kind: GapKind,
    /// The task/construct identifier, e.g. "DownloadSecureFile@1" or "secret".
    pub construct: String,
    /// The Importer's message — grounding for the LLM, shown to reviewers.
    pub detail: String,
}

/// The parsed result of a single pipeline dry-run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunResult {
    pub pipeline_id: String,
    /// Share of steps the Importer converted automatically (0.0–1.0).
    pub converted_ratio: f64,
    pub gaps: Vec<Gap>,
}

impl DryRunResult {
    /// Gaps of a given kind.
    pub fn gaps_of(&self, kind: GapKind) -> impl Iterator<Item = &Gap> {
        self.gaps.iter().filter(move |g| g.kind == kind)
    }
}
