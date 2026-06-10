//! The workflow assembler — merges the Importer's baseline workflow with the
//! LLM's gap-fills into a single proposed workflow, recording provenance.
//!
//! Provenance is the point (plan §1: everything is attestable): a reviewer must
//! be able to see, block by block, which YAML came from the official Importer
//! and which was proposed by a model. This v1 keeps the Importer baseline intact
//! and appends a clearly-delimited gap-fill section, each block tagged with its
//! source construct and the versioned prompt id that produced it. In-place
//! splicing of fills at their exact step positions is future work; appending is
//! honest and never silently rewrites the Importer's output.

use serde::{Deserialize, Serialize};

/// One LLM-proposed fill for a single gap, ready to assemble.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapFill {
    /// The source construct the fill addresses (e.g. `DownloadSecureFile@1`).
    pub construct: String,
    /// The versioned prompt that produced the fill (provenance).
    pub prompt_id: String,
    /// The proposed GitHub Actions YAML fragment.
    pub yaml: String,
}

const BANNER: &str = "\
# ──────────────────────────────────────────────────────────────────────
# bifrost: gap-fills below — REVIEW BEFORE USE
# Everything above this banner is GitHub Actions Importer output.
# Everything below is LLM-proposed YAML for constructs the Importer could
# not convert. Each block is tagged with its source construct + prompt id.
# ──────────────────────────────────────────────────────────────────────";

/// Merge the Importer `baseline` workflow with `fills` into one proposed
/// workflow string. With no fills the baseline is returned unchanged; otherwise
/// the baseline is preserved and a provenance-tagged gap-fill section is
/// appended.
pub fn assemble_workflow(baseline: &str, fills: &[GapFill]) -> String {
    if fills.is_empty() {
        return baseline.to_string();
    }

    let mut out = String::new();
    let base = baseline.trim_end();
    if !base.is_empty() {
        out.push_str(base);
        out.push_str("\n\n");
    }
    out.push_str(BANNER);
    out.push('\n');

    for fill in fills {
        out.push_str(&format!(
            "\n# bifrost-gap-fill: {} (prompt: {})\n",
            fill.construct, fill.prompt_id
        ));
        out.push_str(fill.yaml.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(construct: &str, yaml: &str) -> GapFill {
        GapFill {
            construct: construct.into(),
            prompt_id: "gap-fill.v1".into(),
            yaml: yaml.into(),
        }
    }

    #[test]
    fn no_fills_returns_the_baseline_unchanged() {
        let baseline = "steps:\n  - uses: actions/checkout@v4\n";
        assert_eq!(assemble_workflow(baseline, &[]), baseline);
    }

    #[test]
    fn fills_are_appended_with_provenance_after_the_baseline() {
        let baseline = "steps:\n  - uses: actions/checkout@v4";
        let fills = vec![
            fill(
                "DownloadSecureFile@1",
                "- run: az keyvault secret download ...",
            ),
            fill(
                "strategy.matrix",
                "strategy:\n  matrix:\n    os: [ubuntu-latest]",
            ),
        ];
        let out = assemble_workflow(baseline, &fills);

        // Baseline preserved, banner separates Importer output from LLM fills.
        assert!(out.starts_with("steps:\n  - uses: actions/checkout@v4"));
        assert!(out.contains("REVIEW BEFORE USE"));
        // Each fill is tagged with its construct + prompt id (provenance).
        assert!(out.contains("# bifrost-gap-fill: DownloadSecureFile@1 (prompt: gap-fill.v1)"));
        assert!(out.contains("# bifrost-gap-fill: strategy.matrix (prompt: gap-fill.v1)"));
        assert!(out.contains("az keyvault secret download"));
        // Importer block appears before any LLM block.
        assert!(out.find("checkout@v4").unwrap() < out.find("REVIEW BEFORE USE").unwrap());
    }

    #[test]
    fn empty_baseline_still_emits_the_gap_fill_section() {
        let out = assemble_workflow("", &[fill("X@1", "- run: echo hi")]);
        assert!(out.contains("REVIEW BEFORE USE"));
        assert!(out.contains("# bifrost-gap-fill: X@1"));
        assert!(out.contains("echo hi"));
        // No stray leading blank lines when there is no baseline.
        assert!(!out.starts_with('\n'));
    }
}
