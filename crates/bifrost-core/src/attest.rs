//! Signed, exportable per-migration attestation (#62).
//!
//! Packages everything that makes one pipeline's migration auditable — the
//! proposal's deterministic risk, every recorded decision/approval from the
//! audit log, and the smoke-parity attestation (#61) — into a single
//! in-toto-inspired statement, then signs it so the record can be exported and
//! verified offline.
//!
//! Signing is **HMAC-SHA256** over the canonical JSON of the statement. The
//! choice is deliberate: it is deterministic, dependency-light, and works in an
//! air-gapped deployment with no signing service or network — consistent with
//! Bifrost's hard rules. The signing key is supplied by the caller (from
//! configuration), never read from a clock or the network here. Asymmetric
//! signing (ed25519 / Sigstore) is a future enhancement; the [`Signature`]
//! carries an `algorithm` + `key_id` so a different scheme can be added without
//! breaking the format.

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::audit_log::AuditEvent;
use crate::model::{ProposalStatus, RiskBand};
use crate::proposal::{Attestation, Proposal};

type HmacSha256 = Hmac<Sha256>;

/// The statement type URI for a Bifrost migration attestation (versioned, so the
/// predicate shape can evolve).
pub const MIGRATION_PREDICATE_TYPE: &str = "https://bifrost.dev/attestations/migration/v1";

/// The evidence for one migration: the proposal's deterministic risk, the full
/// decision/approval trail, and the smoke-parity result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPredicate {
    pub status: ProposalStatus,
    pub risk_band: RiskBand,
    pub risk_score: i32,
    /// Id of the versioned prompt that produced the gap-fills (provenance).
    pub prompt_id: String,
    /// The model's certainty (NOT a risk score).
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// Every recorded decision/approval for this proposal, oldest first.
    pub decisions: Vec<AuditEvent>,
    /// The smoke-parity attestation, if one has been recorded (#61).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parity: Option<Attestation>,
}

/// An unsigned migration attestation statement (in-toto-inspired shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationAttestation {
    pub predicate_type: String,
    /// What the attestation is about — the pipeline being migrated.
    pub subject: String,
    pub proposal_id: String,
    pub predicate: MigrationPredicate,
}

impl MigrationAttestation {
    /// Assemble the statement from a proposal and its audit trail.
    pub fn build(proposal: &Proposal, decisions: &[AuditEvent]) -> Self {
        Self {
            predicate_type: MIGRATION_PREDICATE_TYPE.to_string(),
            subject: proposal.pipeline_id.clone(),
            proposal_id: proposal.id.clone(),
            predicate: MigrationPredicate {
                status: proposal.status,
                risk_band: proposal.risk_band,
                risk_score: proposal.risk_score,
                prompt_id: proposal.prompt_id.clone(),
                confidence: proposal.confidence,
                pr_url: proposal.pr_url.clone(),
                decisions: decisions.to_vec(),
                parity: proposal.parity.clone(),
            },
        }
    }

    /// Canonical bytes the signature is computed over. `serde_json` serializes
    /// struct fields in declaration order and we use no maps, so this is stable.
    fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("attestation serializes")
    }

    /// Sign the statement with `key`, tagging the signature with `key_id`.
    pub fn sign(self, key: &[u8], key_id: impl Into<String>) -> SignedMigrationAttestation {
        let value = hmac_sha256_hex(key, &self.canonical_bytes());
        SignedMigrationAttestation {
            attestation: self,
            signature: Signature {
                algorithm: "hmac-sha256".to_string(),
                key_id: key_id.into(),
                value,
            },
        }
    }
}

/// A detached signature over a [`MigrationAttestation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    /// Signing algorithm (`hmac-sha256`).
    pub algorithm: String,
    /// Identifier of the key used, so a verifier knows which key to apply.
    pub key_id: String,
    /// The signature, hex-encoded.
    pub value: String,
}

/// A signed migration attestation — the exportable, verifiable record (#62).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedMigrationAttestation {
    #[serde(flatten)]
    pub attestation: MigrationAttestation,
    pub signature: Signature,
}

impl SignedMigrationAttestation {
    /// Verify the signature against `key`. Returns `false` on any tampering with
    /// the attestation body or the signature, an unknown algorithm, or a bad key.
    pub fn verify(&self, key: &[u8]) -> bool {
        if self.signature.algorithm != "hmac-sha256" {
            return false;
        }
        let Some(expected) = hex_decode(&self.signature.value) else {
            return false;
        };
        let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
            return false;
        };
        mac.update(&self.attestation.canonical_bytes());
        mac.verify_slice(&expected).is_ok()
    }
}

/// Deterministic roll-up of an [`AuditPack`]: how many migrations, how many
/// validated, and the spread of parity verdicts. Fixed fields (no maps) so the
/// pack signs deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPackSummary {
    pub total: usize,
    pub validated: usize,
    pub parity_pass: usize,
    pub parity_gaps: usize,
    pub parity_unattested: usize,
}

/// A per-org compliance audit pack (#63): every migration's signed attestation
/// (who/what/why/when + parity) bundled into one artifact for auditors, with a
/// summary roll-up. The pack itself is signed, so the whole set is tamper-evident
/// — not just the individual attestations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditPack {
    /// Caller-supplied ISO-8601 timestamp (the core does not read a clock).
    pub generated_at: String,
    pub summary: AuditPackSummary,
    pub attestations: Vec<SignedMigrationAttestation>,
}

impl AuditPack {
    /// Assemble a pack from per-migration signed attestations, computing the
    /// summary. The attestations are sorted by `proposalId` so the pack — and its
    /// signature — is deterministic regardless of store iteration order.
    pub fn build(
        generated_at: impl Into<String>,
        mut signed: Vec<SignedMigrationAttestation>,
    ) -> Self {
        signed.sort_by(|a, b| a.attestation.proposal_id.cmp(&b.attestation.proposal_id));
        let total = signed.len();
        let validated = signed
            .iter()
            .filter(|s| s.attestation.predicate.status == ProposalStatus::Validated)
            .count();
        let mut parity_pass = 0;
        let mut parity_gaps = 0;
        let mut parity_unattested = 0;
        for s in &signed {
            match &s.attestation.predicate.parity {
                Some(a) if a.verdict == crate::parity::ParityVerdict::Pass => parity_pass += 1,
                Some(_) => parity_gaps += 1,
                None => parity_unattested += 1,
            }
        }
        Self {
            generated_at: generated_at.into(),
            summary: AuditPackSummary {
                total,
                validated,
                parity_pass,
                parity_gaps,
                parity_unattested,
            },
            attestations: signed,
        }
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("audit pack serializes")
    }

    /// Sign the pack with `key`, tagging the signature with `key_id`.
    pub fn sign(self, key: &[u8], key_id: impl Into<String>) -> SignedAuditPack {
        let value = hmac_sha256_hex(key, &self.canonical_bytes());
        SignedAuditPack {
            pack: self,
            signature: Signature {
                algorithm: "hmac-sha256".to_string(),
                key_id: key_id.into(),
                value,
            },
        }
    }
}

/// A signed [`AuditPack`] — the exportable, verifiable compliance artifact (#63).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedAuditPack {
    #[serde(flatten)]
    pub pack: AuditPack,
    pub signature: Signature,
}

impl SignedAuditPack {
    /// Verify the pack-level signature against `key`. (Each attestation inside can
    /// also be verified individually via [`SignedMigrationAttestation::verify`].)
    pub fn verify(&self, key: &[u8]) -> bool {
        if self.signature.algorithm != "hmac-sha256" {
            return false;
        }
        let Some(expected) = hex_decode(&self.signature.value) else {
            return false;
        };
        let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
            return false;
        };
        mac.update(&self.pack.canonical_bytes());
        mac.verify_slice(&expected).is_ok()
    }
}

/// HMAC-SHA256 of `bytes` under `key`, hex-encoded. HMAC accepts any key length.
fn hmac_sha256_hex(key: &[u8], bytes: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(bytes);
    hex_encode(&mac.finalize().into_bytes())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi << 4 | lo) as u8);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit_log::AuditLog;
    use crate::risk::RiskAssessment;

    fn proposal() -> Proposal {
        let assessment = RiskAssessment {
            score: 42,
            band: RiskBand::Amber,
            factors: vec![],
        };
        Proposal::new(
            "prop-1",
            "SARC-main",
            "src",
            "out",
            "why",
            vec![],
            vec![],
            "gap-fill.v1",
            0.8,
            &assessment,
        )
    }

    fn committed_with_trail() -> (Proposal, AuditLog) {
        let mut p = proposal();
        let mut log = AuditLog::new();
        for to in [
            ProposalStatus::InReview,
            ProposalStatus::Approved,
            ProposalStatus::Committed,
        ] {
            p.transition(to, "reviewer@example.com", "2026-06-11T00:00:00Z", &mut log)
                .unwrap();
        }
        (p, log)
    }

    #[test]
    fn build_captures_decisions_and_risk() {
        let (p, log) = committed_with_trail();
        let att = MigrationAttestation::build(&p, log.events());
        assert_eq!(att.subject, "SARC-main");
        assert_eq!(att.predicate.risk_score, 42);
        assert_eq!(att.predicate.decisions.len(), 3);
        assert_eq!(att.predicate_type, MIGRATION_PREDICATE_TYPE);
    }

    #[test]
    fn sign_then_verify_roundtrips() {
        let (p, log) = committed_with_trail();
        let key = b"super-secret-signing-key";
        let signed = MigrationAttestation::build(&p, log.events()).sign(key, "test-key");
        assert_eq!(signed.signature.algorithm, "hmac-sha256");
        assert_eq!(signed.signature.key_id, "test-key");
        assert!(signed.verify(key));
    }

    #[test]
    fn verify_fails_on_tamper() {
        let (p, log) = committed_with_trail();
        let key = b"super-secret-signing-key";
        let mut signed = MigrationAttestation::build(&p, log.events()).sign(key, "k");
        // Tamper with the body — the risk score is now a lie.
        signed.attestation.predicate.risk_score = 0;
        assert!(!signed.verify(key));
    }

    #[test]
    fn verify_fails_with_wrong_key() {
        let (p, log) = committed_with_trail();
        let signed = MigrationAttestation::build(&p, log.events()).sign(b"key-a", "k");
        assert!(!signed.verify(b"key-b"));
    }

    #[test]
    fn signing_is_deterministic() {
        let (p, log) = committed_with_trail();
        let key = b"k";
        let a = MigrationAttestation::build(&p, log.events()).sign(key, "k");
        let b = MigrationAttestation::build(&p, log.events()).sign(key, "k");
        assert_eq!(a.signature.value, b.signature.value);
    }

    #[test]
    fn audit_pack_summarizes_signs_and_verifies() {
        let key = b"pack-key";
        let (p, log) = committed_with_trail();
        let a = MigrationAttestation::build(&p, log.events()).sign(key, "k");

        // A second migration, still a draft (no decisions, no parity).
        let mut p2 = proposal();
        p2.id = "prop-2".into();
        p2.pipeline_id = "OTHER-main".into();
        let b = MigrationAttestation::build(&p2, &[]).sign(key, "k");

        // Build out of order — the pack sorts by proposalId for determinism.
        let pack = AuditPack::build("2026-06-11T00:00:00Z", vec![b, a]);
        assert_eq!(pack.summary.total, 2);
        assert_eq!(pack.summary.parity_unattested, 2);
        assert_eq!(pack.attestations[0].attestation.proposal_id, "prop-1");

        let signed = pack.sign(key, "pack");
        assert!(signed.verify(key));
        assert!(!signed.verify(b"wrong"));
    }

    #[test]
    fn audit_pack_build_is_order_independent() {
        let key = b"k";
        let (p, log) = committed_with_trail();
        let a = MigrationAttestation::build(&p, log.events()).sign(key, "k");
        let mut p2 = proposal();
        p2.id = "prop-2".into();
        let b = MigrationAttestation::build(&p2, &[]).sign(key, "k");
        let sig1 = AuditPack::build("t", vec![a.clone(), b.clone()]).sign(key, "p");
        let sig2 = AuditPack::build("t", vec![b, a]).sign(key, "p");
        assert_eq!(sig1.signature.value, sig2.signature.value);
    }

    #[test]
    fn hex_roundtrips() {
        let bytes = [0x00u8, 0x0f, 0xa5, 0xff];
        assert_eq!(hex_encode(&bytes), "000fa5ff");
        assert_eq!(hex_decode("000fa5ff").unwrap(), bytes);
        assert!(hex_decode("xyz").is_none());
        assert!(hex_decode("abc").is_none()); // odd length
    }
}
