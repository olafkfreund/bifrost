//! Smoke-parity diff (#60).
//!
//! Compares a converted GitHub Actions run against the last successful Azure
//! DevOps run on three **smoke** signals: did it succeed, did it produce the
//! same set of artifacts, and did it declare the same outputs. This is
//! deliberately *not* full equivalence — we do not compare logs, timings, step
//! semantics, or artifact contents. A `Pass` means "nothing the baseline
//! produced went missing", not "the two runs are identical". The verdict is
//! computed here, deterministically; the LLM is never involved.

use serde::{Deserialize, Serialize};

/// The smoke-relevant facts of a single run, normalized across ADO and GitHub so
/// both sides of the diff use the same shape. Names only — never contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunFacts {
    /// Did the run finish successfully (ADO `result == succeeded` / GitHub
    /// `conclusion == success`)?
    pub succeeded: bool,
    /// Names of the artifacts the run published.
    pub artifacts: Vec<String>,
    /// Names of the outputs the run declared.
    pub outputs: Vec<String>,
}

/// A set comparison between baseline and converted: what they share, what the
/// baseline had that the converted run lacks (`missing` — the parity gaps), and
/// what the converted run adds (`extra` — informational, not a failure).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDiff {
    pub common: Vec<String>,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
}

impl SetDiff {
    /// Diff two name lists. Order-insensitive and de-duplicated; results are
    /// sorted so the report is deterministic.
    fn of(baseline: &[String], converted: &[String]) -> Self {
        let mut common: Vec<String> = baseline
            .iter()
            .filter(|b| converted.contains(b))
            .cloned()
            .collect();
        let mut missing: Vec<String> = baseline
            .iter()
            .filter(|b| !converted.contains(b))
            .cloned()
            .collect();
        let mut extra: Vec<String> = converted
            .iter()
            .filter(|c| !baseline.contains(c))
            .cloned()
            .collect();
        for v in [&mut common, &mut missing, &mut extra] {
            v.sort();
            v.dedup();
        }
        Self {
            common,
            missing,
            extra,
        }
    }
}

/// The overall parity outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParityVerdict {
    /// Status matched and nothing the baseline produced is missing.
    Pass,
    /// At least one smoke signal regressed (status mismatch or a missing
    /// artifact/output).
    Gaps,
}

/// The result of a smoke-parity comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityReport {
    /// Both runs succeeded (or both failed — same status either way).
    pub status_match: bool,
    pub baseline_succeeded: bool,
    pub converted_succeeded: bool,
    pub artifacts: SetDiff,
    pub outputs: SetDiff,
    pub verdict: ParityVerdict,
    /// Human-readable gap descriptions plus the explicit smoke-parity caveat.
    pub notes: Vec<String>,
}

/// The fixed caveat appended to every report, so a consumer can never mistake a
/// `Pass` for a proof of equivalence.
pub const SMOKE_PARITY_CAVEAT: &str =
    "Smoke parity only: compares success, artifact names, and declared output \
     names — not logs, step semantics, timings, or artifact contents.";

/// Compare a converted run against the ADO baseline. Deterministic.
pub fn compare(baseline: &RunFacts, converted: &RunFacts) -> ParityReport {
    let status_match = baseline.succeeded == converted.succeeded;
    let artifacts = SetDiff::of(&baseline.artifacts, &converted.artifacts);
    let outputs = SetDiff::of(&baseline.outputs, &converted.outputs);

    let mut notes = Vec::new();
    if !status_match {
        notes.push(format!(
            "status mismatch: baseline succeeded={}, converted succeeded={}",
            baseline.succeeded, converted.succeeded
        ));
    }
    if !artifacts.missing.is_empty() {
        notes.push(format!(
            "missing artifacts the baseline produced: {}",
            artifacts.missing.join(", ")
        ));
    }
    if !outputs.missing.is_empty() {
        notes.push(format!(
            "missing outputs the baseline declared: {}",
            outputs.missing.join(", ")
        ));
    }
    if !artifacts.extra.is_empty() {
        notes.push(format!(
            "converted run adds artifacts (informational): {}",
            artifacts.extra.join(", ")
        ));
    }
    if !outputs.extra.is_empty() {
        notes.push(format!(
            "converted run adds outputs (informational): {}",
            outputs.extra.join(", ")
        ));
    }

    let verdict = if status_match && artifacts.missing.is_empty() && outputs.missing.is_empty() {
        ParityVerdict::Pass
    } else {
        ParityVerdict::Gaps
    };

    notes.push(SMOKE_PARITY_CAVEAT.to_string());

    ParityReport {
        status_match,
        baseline_succeeded: baseline.succeeded,
        converted_succeeded: converted.succeeded,
        artifacts,
        outputs,
        verdict,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(succeeded: bool, artifacts: &[&str], outputs: &[&str]) -> RunFacts {
        RunFacts {
            succeeded,
            artifacts: artifacts.iter().map(|s| s.to_string()).collect(),
            outputs: outputs.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn identical_runs_pass() {
        let b = facts(true, &["app", "coverage"], &["image_tag"]);
        let c = facts(true, &["coverage", "app"], &["image_tag"]);
        let r = compare(&b, &c);
        assert_eq!(r.verdict, ParityVerdict::Pass);
        assert!(r.status_match);
        assert_eq!(r.artifacts.common, vec!["app", "coverage"]);
        assert!(r.artifacts.missing.is_empty());
        // The caveat is always present.
        assert!(r.notes.iter().any(|n| n.starts_with("Smoke parity only")));
    }

    #[test]
    fn missing_artifact_is_a_gap() {
        let b = facts(true, &["app", "coverage"], &[]);
        let c = facts(true, &["app"], &[]);
        let r = compare(&b, &c);
        assert_eq!(r.verdict, ParityVerdict::Gaps);
        assert_eq!(r.artifacts.missing, vec!["coverage"]);
    }

    #[test]
    fn status_mismatch_is_a_gap() {
        let b = facts(true, &[], &[]);
        let c = facts(false, &[], &[]);
        let r = compare(&b, &c);
        assert_eq!(r.verdict, ParityVerdict::Gaps);
        assert!(!r.status_match);
    }

    #[test]
    fn extra_artifacts_are_informational_not_a_gap() {
        let b = facts(true, &["app"], &["image_tag"]);
        let c = facts(true, &["app", "sbom"], &["image_tag", "digest"]);
        let r = compare(&b, &c);
        assert_eq!(r.verdict, ParityVerdict::Pass);
        assert_eq!(r.artifacts.extra, vec!["sbom"]);
        assert_eq!(r.outputs.extra, vec!["digest"]);
    }

    #[test]
    fn missing_output_is_a_gap() {
        let b = facts(true, &[], &["image_tag", "digest"]);
        let c = facts(true, &[], &["image_tag"]);
        let r = compare(&b, &c);
        assert_eq!(r.verdict, ParityVerdict::Gaps);
        assert_eq!(r.outputs.missing, vec!["digest"]);
    }
}
