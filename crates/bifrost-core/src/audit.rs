//! Typed representation of `gh actions-importer audit` output.
//!
//! The Importer writes a human-readable `audit_summary.md`; [`crate`] consumers
//! work with this typed form instead. Parsing lives in `bifrost-adapters`
//! (we wrap the official tool and parse its output — we never reimplement it).

use serde::{Deserialize, Serialize};

/// Counts for one audited category (pipelines or build steps).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditCounts {
    pub total: u32,
    pub successful: u32,
    pub partially_successful: u32,
    pub unsupported: u32,
    pub failed: u32,
}

/// Kind of manual follow-up the Importer flags (it cannot do these for you).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManualTaskKind {
    Secret,
    SelfHostedRunner,
    Other,
}

/// A single manual task. For secrets this is the secret *name* only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualTask {
    pub kind: ManualTaskKind,
    pub name: String,
}

/// An unsupported step/task and how many times it appeared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedStep {
    pub task: String,
    pub count: u32,
}

/// The parsed audit footprint for an org.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSummary {
    pub pipelines: AuditCounts,
    pub build_steps: AuditCounts,
    pub manual_tasks: Vec<ManualTask>,
    pub unsupported_steps: Vec<UnsupportedStep>,
    /// The actions allow-list the converted workflows would require.
    pub actions: Vec<String>,
}
