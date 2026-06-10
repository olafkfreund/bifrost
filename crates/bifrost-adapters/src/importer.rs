//! Wrapper around the official `gh actions-importer`.
//!
//! Provides parsers for the Importer's `audit_summary.md` ([`parse_audit_summary`])
//! and per-pipeline dry-run logs ([`parse_dry_run`]), plus the [`Importer`] trait
//! and a fixture-backed [`MockImporter`]. The Docker subprocess driver that
//! produces these outputs lands behind the same trait later — we wrap the
//! official tool and parse its output; we never reimplement it.

use async_trait::async_trait;
use bifrost_core::{
    AuditCounts, AuditSummary, DryRunResult, Gap, GapKind, ManualTask, ManualTaskKind,
    UnsupportedStep,
};

/// First unsigned integer appearing in `s` (handles bold/percent like `**126 (88%)**`).
fn leading_uint(s: &str) -> Option<u32> {
    let digits: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn apply_count(counts: &mut AuditCounts, label: &str, n: u32) {
    match label {
        "total" => counts.total = n,
        "successful" => counts.successful = n,
        "partially successful" => counts.partially_successful = n,
        "unsupported" => counts.unsupported = n,
        "failed" => counts.failed = n,
        _ => {}
    }
}

/// Extract `NAME` from a `` `${{ secrets.NAME }}` `` manual-task reference.
fn secret_name(item: &str) -> Option<String> {
    let start = item.find("secrets.")? + "secrets.".len();
    let name: String = item[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// Parse a `gh actions-importer audit` summary into a typed [`AuditSummary`].
///
/// Matches the real Importer markdown: bold counts with percentages
/// (`**126 (88%)**`), a plain `Total:` line, and the Build-steps section's inline
/// Known/Unknown/Actions buckets. Unknown lines are ignored so format drift in
/// sections we don't consume can't break it.
pub fn parse_audit_summary(md: &str) -> AuditSummary {
    let mut summary = AuditSummary::default();
    let mut h3 = String::new(); // current `###` section, lowercased
    let mut bucket = String::new(); // Build-steps/Manual-tasks sub-bucket
    let mut pipelines_top = false; // inside `## Pipelines`, before any `###`

    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(t) = line.strip_prefix("### ") {
            h3 = t.trim().to_ascii_lowercase();
            pipelines_top = false;
            bucket.clear();
            continue;
        }
        if let Some(t) = line.strip_prefix("## ") {
            pipelines_top = t.trim().eq_ignore_ascii_case("Pipelines");
            h3.clear();
            bucket.clear();
            continue;
        }
        if line.starts_with('#') {
            continue; // `# Audit summary`, `#### per-pipeline`, etc.
        }

        let bullet = line.strip_prefix("- ").map(str::trim);

        // Plain `Label: **N**` lines set counts and/or select a sub-bucket.
        if bullet.is_none() {
            if let Some((label, rest)) = line.split_once(':') {
                let label = label.trim().to_ascii_lowercase();
                let n = leading_uint(rest);
                if pipelines_top && label == "total" {
                    summary.pipelines.total = n.unwrap_or(0);
                } else if h3 == "build steps" {
                    match label.as_str() {
                        "total" => summary.build_steps.total = n.unwrap_or(0),
                        "known" => {
                            summary.build_steps.successful = n.unwrap_or(0);
                            bucket = "known".into();
                        }
                        "unknown" => {
                            summary.build_steps.unsupported = n.unwrap_or(0);
                            bucket = "unknown".into();
                        }
                        "actions" => bucket = "actions".into(),
                        _ => {}
                    }
                } else if h3 == "manual tasks" && label == "secrets" {
                    bucket = "secrets".into();
                }
            }
            continue;
        }

        let item = bullet.unwrap();
        if pipelines_top {
            if let Some((label, rest)) = item.split_once(':') {
                if let Some(n) = leading_uint(rest) {
                    apply_count(
                        &mut summary.pipelines,
                        &label.trim().to_ascii_lowercase(),
                        n,
                    );
                }
            }
        } else if h3 == "build steps" {
            match bucket.as_str() {
                "unknown" => {
                    if let Some((task, rest)) = item.split_once(':') {
                        if let Some(n) = leading_uint(rest) {
                            summary.unsupported_steps.push(UnsupportedStep {
                                task: task.trim().into(),
                                count: n,
                            });
                        }
                    }
                }
                "actions" => {
                    let action = item.split_once(':').map(|(a, _)| a.trim()).unwrap_or(item);
                    summary.actions.push(action.into());
                }
                _ => {}
            }
        } else if h3 == "manual tasks" && bucket == "secrets" {
            if let Some(name) = secret_name(item) {
                summary.manual_tasks.push(ManualTask {
                    kind: ManualTaskKind::Secret,
                    name,
                });
            }
        }
    }
    summary
}

/// Map a dry-run section header to the gap kind its bullets represent.
fn gap_kind_for(section: &str) -> Option<GapKind> {
    match section {
        "unsupported steps" => Some(GapKind::UnsupportedStep),
        "partial constructs" => Some(GapKind::PartialConstruct),
        "manual tasks" => Some(GapKind::ManualTask),
        _ => None,
    }
}

/// Parse a `gh actions-importer dry-run` log into a typed [`DryRunResult`].
///
/// Extracts the pipeline id, the converted ratio from "Converted N of M steps",
/// and the gaps grouped under their section headers. Tolerant of unknown lines.
pub fn parse_dry_run(log: &str) -> DryRunResult {
    let mut pipeline_id = String::new();
    let mut converted_ratio = 1.0;
    let mut gaps = Vec::new();
    let mut kind: Option<GapKind> = None;

    for raw in log.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("Converting pipeline ") {
            pipeline_id = rest
                .trim_matches(|c| c == '\'' || c == '.' || c == ' ')
                .to_string();
            continue;
        }
        if let Some(rest) = line.strip_prefix("Converted ") {
            // "12 of 14 steps."
            let nums: Vec<f64> = rest
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            if let [n, m, ..] = nums[..] {
                if m > 0.0 {
                    converted_ratio = n / m;
                }
            }
            continue;
        }
        if let Some(header) = line.strip_suffix(':') {
            kind = gap_kind_for(&header.to_ascii_lowercase());
            continue;
        }
        if let (Some(k), Some(item)) = (kind, line.strip_prefix("- ")) {
            let (construct, detail) = item
                .split_once(':')
                .map(|(c, d)| (c.trim(), d.trim()))
                .unwrap_or((item.trim(), ""));
            gaps.push(Gap {
                kind: k,
                construct: construct.into(),
                detail: detail.into(),
            });
        }
    }

    DryRunResult {
        pipeline_id,
        converted_ratio,
        gaps,
    }
}

/// Errors the Importer wrapper can surface.
#[derive(Debug, thiserror::Error)]
pub enum ImporterError {
    #[error("importer subprocess failed: {0}")]
    Subprocess(String),
    #[error("could not parse importer output: {0}")]
    Parse(String),
}

/// The official `gh actions-importer`, wrapped behind a trait so orchestration
/// can be tested without Docker. The real driver shells out to the pinned image.
#[async_trait]
pub trait Importer: Send + Sync {
    /// The Importer version/digest in use (recorded per job for attestation).
    async fn version(&self) -> Result<String, ImporterError>;
    /// Audit the org and return the parsed footprint.
    async fn audit(&self) -> Result<AuditSummary, ImporterError>;
    /// Dry-run a single pipeline and return its converted ratio + gaps.
    async fn dry_run(&self, pipeline_id: &str) -> Result<DryRunResult, ImporterError>;
}

/// A fixture-backed [`Importer`] for tests and offline runs.
#[derive(Debug, Clone, Default)]
pub struct MockImporter;

const FIXTURE_AUDIT: &str = include_str!("../../../fixtures/audit_summary.md");
const FIXTURE_DRY_RUN: &str = include_str!("../../../fixtures/dry_run.log");

#[async_trait]
impl Importer for MockImporter {
    async fn version(&self) -> Result<String, ImporterError> {
        Ok("mock-importer".into())
    }

    async fn audit(&self) -> Result<AuditSummary, ImporterError> {
        Ok(parse_audit_summary(FIXTURE_AUDIT))
    }

    async fn dry_run(&self, pipeline_id: &str) -> Result<DryRunResult, ImporterError> {
        let mut result = parse_dry_run(FIXTURE_DRY_RUN);
        result.pipeline_id = pipeline_id.to_string();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../fixtures/audit_summary.md");

    #[test]
    fn parses_pipeline_and_step_counts_from_bold_percent_format() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(a.pipelines.total, 3);
        assert_eq!(a.pipelines.successful, 1);
        assert_eq!(a.pipelines.partially_successful, 1);
        assert_eq!(a.pipelines.unsupported, 1);
        assert_eq!(a.pipelines.failed, 0);

        // Build steps map Known→successful, Unknown→unsupported.
        assert_eq!(a.build_steps.total, 20);
        assert_eq!(a.build_steps.successful, 17);
        assert_eq!(a.build_steps.unsupported, 3);
    }

    #[test]
    fn parses_unknown_build_steps_with_counts() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(a.unsupported_steps.len(), 2);
        let cache = a
            .unsupported_steps
            .iter()
            .find(|s| s.task == "Cache@2")
            .expect("unknown step present");
        assert_eq!(cache.count, 2);
    }

    #[test]
    fn parses_manual_task_secret_names() {
        let a = parse_audit_summary(FIXTURE);
        let secrets: Vec<_> = a
            .manual_tasks
            .iter()
            .filter(|m| m.kind == ManualTaskKind::Secret)
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(secrets, ["AZURE_CLIENT_SECRET", "REGISTRY_TOKEN"]);
    }

    #[test]
    fn parses_actions_allowlist_without_counts() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(a.actions, ["run", "actions/checkout@v4.1.0"]);
    }

    #[test]
    fn unknown_sections_are_ignored() {
        let md = "## Surprise\n- something: **5**\n## Pipelines\nTotal: **3**\n";
        let a = parse_audit_summary(md);
        assert_eq!(a.pipelines.total, 3);
        assert!(a.actions.is_empty());
    }

    /// Parse a real `audit_summary.md` from a live run when its path is given via
    /// `BIFROST_REAL_AUDIT`. Skipped by default; run with:
    ///   `BIFROST_REAL_AUDIT=/path/to/audit_summary.md cargo test -- --ignored`
    #[test]
    #[ignore = "requires a real audit_summary.md path in BIFROST_REAL_AUDIT"]
    fn parses_a_real_audit_summary() {
        let path = std::env::var("BIFROST_REAL_AUDIT").expect("BIFROST_REAL_AUDIT set");
        let md = std::fs::read_to_string(path).expect("readable audit_summary.md");
        let a = parse_audit_summary(&md);
        eprintln!(
            "real audit: pipelines total={} partial={} | steps total={} known={} unknown={} | unsupported_kinds={} secrets={} actions={}",
            a.pipelines.total,
            a.pipelines.partially_successful,
            a.build_steps.total,
            a.build_steps.successful,
            a.build_steps.unsupported,
            a.unsupported_steps.len(),
            a.manual_tasks.len(),
            a.actions.len(),
        );
        assert!(a.pipelines.total > 0, "parsed at least one pipeline");
        assert!(a.build_steps.total > 0, "parsed build steps");
    }

    const DRY_RUN: &str = include_str!("../../../fixtures/dry_run.log");

    #[test]
    fn dry_run_extracts_id_and_converted_ratio() {
        let r = parse_dry_run(DRY_RUN);
        assert_eq!(r.pipeline_id, "web-portal-release");
        assert!((r.converted_ratio - 12.0 / 14.0).abs() < 1e-9);
    }

    #[test]
    fn dry_run_groups_gaps_by_kind() {
        let r = parse_dry_run(DRY_RUN);
        assert_eq!(r.gaps_of(GapKind::UnsupportedStep).count(), 2);
        assert_eq!(r.gaps_of(GapKind::PartialConstruct).count(), 1);
        assert_eq!(r.gaps_of(GapKind::ManualTask).count(), 3);

        let secret = r
            .gaps_of(GapKind::ManualTask)
            .find(|g| g.construct == "secret")
            .expect("secret manual task");
        assert!(secret.detail.contains("AZURE_CLIENT_SECRET"));
    }

    #[tokio::test]
    async fn mock_importer_audits_and_dry_runs() {
        let imp = MockImporter;
        assert_eq!(imp.version().await.unwrap(), "mock-importer");
        assert_eq!(imp.audit().await.unwrap().pipelines.total, 3);

        let r = imp.dry_run("payments-api-deploy").await.unwrap();
        assert_eq!(r.pipeline_id, "payments-api-deploy"); // id overridden to request
        assert!(!r.gaps.is_empty());
    }
}
