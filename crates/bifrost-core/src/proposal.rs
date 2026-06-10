//! The [`Proposal`] — an augmented workflow awaiting review — and its lifecycle
//! state machine.
//!
//! A proposal is the reviewable artifact the core conversion loop produces: the
//! Importer's baseline merged with LLM gap-fills, plus the rationale, the
//! reviewer-facing risk flags, and the **deterministic** risk band/score. The
//! split is deliberate and encodes a hard rule (plan §1): `risk_band`/
//! `risk_score` come from a [`RiskAssessment`], never from the model;
//! `rationale`/`risk_flags`/`confidence` are the model's explanation.
//!
//! The lifecycle is `Draft → InReview → Approved | ChangesRequested →
//! Committed → Validated`, with `ChangesRequested → InReview` to resubmit.
//! Illegal transitions are rejected, and every accepted transition is appended
//! to the [`AuditLog`] — it is impossible to move a proposal without recording
//! it (see [`Proposal::transition`]).

use serde::{Deserialize, Serialize};

use crate::audit_log::{AuditEvent, AuditLog};
use crate::model::{ProposalStatus, RiskBand};
use crate::risk::RiskAssessment;

/// An augmented workflow plus its rationale, risk, and lifecycle status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: String,
    pub pipeline_id: String,
    /// The proposed GitHub Actions workflow (Importer baseline + gap-fills).
    pub proposed_yaml: String,
    /// The model's explanation of the gap-fills (LLM-authored).
    pub rationale: String,
    /// Things a human reviewer must check (LLM-flagged).
    pub risk_flags: Vec<String>,
    /// How to confirm parity before approving (LLM-suggested).
    pub verify_steps: Vec<String>,
    /// Deterministic risk band — from [`RiskAssessment`], never the LLM.
    pub risk_band: RiskBand,
    /// Deterministic risk score (0–100) — from [`RiskAssessment`], never the LLM.
    pub risk_score: i32,
    /// Id of the versioned prompt that produced the gap-fills (provenance).
    pub prompt_id: String,
    /// The model's certainty in its proposed YAML (0.0–1.0) — NOT a risk score.
    pub confidence: f64,
    /// Where the proposal sits in the review lifecycle.
    pub status: ProposalStatus,
}

impl Proposal {
    /// Assemble a new proposal in [`ProposalStatus::Draft`].
    ///
    /// Risk band/score are taken from `assessment` (the deterministic engine),
    /// keeping the LLM out of scoring even though it authored the prose fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        pipeline_id: impl Into<String>,
        proposed_yaml: impl Into<String>,
        rationale: impl Into<String>,
        risk_flags: Vec<String>,
        verify_steps: Vec<String>,
        prompt_id: impl Into<String>,
        confidence: f64,
        assessment: &RiskAssessment,
    ) -> Self {
        Self {
            id: id.into(),
            pipeline_id: pipeline_id.into(),
            proposed_yaml: proposed_yaml.into(),
            rationale: rationale.into(),
            risk_flags,
            verify_steps,
            risk_band: assessment.band,
            risk_score: assessment.score,
            prompt_id: prompt_id.into(),
            confidence,
            status: ProposalStatus::Draft,
        }
    }

    /// Attempt to move the proposal to `to`, recording the transition.
    ///
    /// On success the status is updated and an [`AuditEvent`] is appended to
    /// `log`; on an illegal edge the status is left unchanged and nothing is
    /// logged. Logging is not optional — there is no way to transition without
    /// it, which is what makes the lifecycle attestable.
    pub fn transition(
        &mut self,
        to: ProposalStatus,
        actor: impl Into<String>,
        at: impl Into<String>,
        log: &mut AuditLog,
    ) -> Result<(), ProposalError> {
        let from = self.status;
        if !is_legal_transition(from, to) {
            return Err(ProposalError::IllegalTransition { from, to });
        }
        self.status = to;
        log.append(AuditEvent {
            proposal_id: self.id.clone(),
            actor: actor.into(),
            from,
            to,
            at: at.into(),
        });
        Ok(())
    }
}

/// Whether `from → to` is a legal edge in the proposal lifecycle.
///
/// `Draft → InReview → Approved | ChangesRequested → Committed → Validated`,
/// plus `ChangesRequested → InReview` to resubmit after edits. Every other edge
/// (including no-op self-edges) is illegal.
pub fn is_legal_transition(from: ProposalStatus, to: ProposalStatus) -> bool {
    use ProposalStatus::*;
    matches!(
        (from, to),
        (Draft, InReview)
            | (InReview, Approved)
            | (InReview, ChangesRequested)
            | (ChangesRequested, InReview)
            | (Approved, Committed)
            | (Committed, Validated)
    )
}

/// Errors from an illegal lifecycle move.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProposalError {
    #[error("illegal proposal transition: {from:?} → {to:?}")]
    IllegalTransition {
        from: ProposalStatus,
        to: ProposalStatus,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RiskBand;
    use crate::risk::RiskAssessment;

    fn assessment() -> RiskAssessment {
        RiskAssessment {
            score: 55,
            band: RiskBand::Amber,
            factors: vec![],
        }
    }

    fn proposal() -> Proposal {
        Proposal::new(
            "prop-1",
            "SARC-main",
            "steps:\n  - uses: actions/checkout@v4\n",
            "Filled the secure-file gap with an OIDC download.",
            vec!["verify the keystore secret name".into()],
            vec!["run the workflow in a sandbox".into()],
            "gap-fill.v1",
            0.8,
            &assessment(),
        )
    }

    #[test]
    fn new_proposal_starts_in_draft_with_deterministic_risk() {
        let p = proposal();
        assert_eq!(p.status, ProposalStatus::Draft);
        // Risk comes from the assessment, not the LLM-authored fields.
        assert_eq!(p.risk_band, RiskBand::Amber);
        assert_eq!(p.risk_score, 55);
        assert_eq!(p.prompt_id, "gap-fill.v1");
    }

    #[test]
    fn full_legal_lifecycle_is_recorded_in_order() {
        let mut p = proposal();
        let mut log = AuditLog::new();
        let path = [
            ProposalStatus::InReview,
            ProposalStatus::Approved,
            ProposalStatus::Committed,
            ProposalStatus::Validated,
        ];
        for (i, &to) in path.iter().enumerate() {
            p.transition(
                to,
                "reviewer@example.com",
                format!("2026-06-10T00:0{i}:00Z"),
                &mut log,
            )
            .expect("legal transition");
        }
        assert_eq!(p.status, ProposalStatus::Validated);
        // Every transition left an audit trail, oldest first.
        assert_eq!(log.len(), 4);
        let tos: Vec<_> = log.events().iter().map(|e| e.to).collect();
        assert_eq!(tos, path);
        assert_eq!(log.events()[0].from, ProposalStatus::Draft);
        assert_eq!(log.events()[0].actor, "reviewer@example.com");
        assert_eq!(log.events_for("prop-1").count(), 4);
    }

    #[test]
    fn changes_requested_can_be_resubmitted() {
        let mut p = proposal();
        let mut log = AuditLog::new();
        p.transition(ProposalStatus::InReview, "r", "t1", &mut log)
            .unwrap();
        p.transition(ProposalStatus::ChangesRequested, "r", "t2", &mut log)
            .unwrap();
        p.transition(ProposalStatus::InReview, "author", "t3", &mut log)
            .expect("resubmit after edits is legal");
        assert_eq!(p.status, ProposalStatus::InReview);
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn illegal_transition_is_rejected_and_not_logged() {
        let mut p = proposal();
        let mut log = AuditLog::new();
        // Draft cannot jump straight to Committed.
        let err = p
            .transition(ProposalStatus::Committed, "r", "t", &mut log)
            .unwrap_err();
        assert_eq!(
            err,
            ProposalError::IllegalTransition {
                from: ProposalStatus::Draft,
                to: ProposalStatus::Committed,
            }
        );
        // Status unchanged and nothing recorded — illegal moves leave no trace.
        assert_eq!(p.status, ProposalStatus::Draft);
        assert!(log.is_empty());
    }

    #[test]
    fn self_edges_and_backward_edges_are_illegal() {
        assert!(!is_legal_transition(
            ProposalStatus::Approved,
            ProposalStatus::Approved
        ));
        assert!(!is_legal_transition(
            ProposalStatus::Validated,
            ProposalStatus::Committed
        ));
        assert!(!is_legal_transition(
            ProposalStatus::Approved,
            ProposalStatus::Draft
        ));
    }
}
