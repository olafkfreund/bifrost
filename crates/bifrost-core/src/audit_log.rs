//! The append-only attestation log.
//!
//! Every proposal state transition (and, later, every human action) is recorded
//! here as an immutable [`AuditEvent`]. This is a *feature*, not logging: the
//! log is the evidence trail that makes a migration attestable (plan §1). The
//! type therefore exposes no way to mutate or remove a recorded event — only
//! [`AuditLog::append`] and read accessors.
//!
//! The core stays clock-free and deterministic, so timestamps are supplied by
//! the caller (`at`) rather than read from a system clock here.

use serde::{Deserialize, Serialize};

use crate::model::ProposalStatus;

/// One immutable entry in the [`AuditLog`]: who moved which proposal from which
/// state to which, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub proposal_id: String,
    /// The identity responsible for the transition (user id / service principal).
    pub actor: String,
    pub from: ProposalStatus,
    pub to: ProposalStatus,
    /// Caller-supplied ISO-8601 timestamp (the core does not read a clock).
    pub at: String,
}

/// An append-only log of [`AuditEvent`]s. Events can be added and read, never
/// changed or removed — that immutability is the point.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditLog {
    events: Vec<AuditEvent>,
}

impl AuditLog {
    /// A new, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an event. This is the only way to add to the log.
    pub fn append(&mut self, event: AuditEvent) {
        self.events.push(event);
    }

    /// The recorded events, oldest first.
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Number of recorded events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events affecting a single proposal, oldest first.
    pub fn events_for<'a>(&'a self, proposal_id: &'a str) -> impl Iterator<Item = &'a AuditEvent> {
        self.events
            .iter()
            .filter(move |e| e.proposal_id == proposal_id)
    }
}
