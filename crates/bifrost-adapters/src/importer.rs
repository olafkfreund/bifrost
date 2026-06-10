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

/// Strip a leading markdown bullet (`- ` / `* `) if present.
fn bullet(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("- ")
        .or_else(|| t.strip_prefix("* "))
        .map(str::trim)
}

/// Strip leading `#`s from a heading line and return the trimmed title.
fn heading(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.starts_with('#').then(|| t.trim_start_matches('#').trim())
}

/// Parse a `"Label: 12"` stat line into `(label_lowercased, count)`.
fn stat(item: &str) -> Option<(String, u32)> {
    let (label, value) = item.split_once(':')?;
    let n: u32 = value.split_whitespace().next()?.parse().ok()?;
    Some((label.trim().to_ascii_lowercase(), n))
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

/// Parse a `gh actions-importer audit` summary into a typed [`AuditSummary`].
///
/// Tolerant of heading level (`#` vs `##`): sections are matched by their unique
/// titles. Unknown sections are ignored so new Importer output doesn't break it.
pub fn parse_audit_summary(md: &str) -> AuditSummary {
    let mut summary = AuditSummary::default();
    let mut section = String::new();

    for line in md.lines() {
        if let Some(title) = heading(line) {
            section = title.to_ascii_lowercase();
            continue;
        }
        let Some(item) = bullet(line) else { continue };

        match section.as_str() {
            "pipelines" => {
                if let Some((label, n)) = stat(item) {
                    apply_count(&mut summary.pipelines, &label, n);
                }
            }
            "build steps" => {
                if let Some((label, n)) = stat(item) {
                    apply_count(&mut summary.build_steps, &label, n);
                }
            }
            "unsupported build steps" => {
                // `stat` lowercases the label, so take the count from it but keep
                // the original task casing from `item`.
                if let Some((_, n)) = stat(item) {
                    let task = item.split_once(':').map(|(t, _)| t.trim()).unwrap_or(item);
                    summary.unsupported_steps.push(UnsupportedStep {
                        task: task.into(),
                        count: n,
                    });
                }
            }
            "secrets" => {
                summary.manual_tasks.push(ManualTask {
                    kind: ManualTaskKind::Secret,
                    name: item.into(),
                });
            }
            "self hosted runners" => {
                let name = item.split_once(':').map(|(n, _)| n.trim()).unwrap_or(item);
                summary.manual_tasks.push(ManualTask {
                    kind: ManualTaskKind::SelfHostedRunner,
                    name: name.into(),
                });
            }
            "actions" => {
                let action = item.split_once(':').map(|(a, _)| a.trim()).unwrap_or(item);
                summary.actions.push(action.into());
            }
            _ => {}
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
    fn parses_pipeline_and_step_counts() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(a.pipelines.total, 16);
        assert_eq!(a.pipelines.successful, 9);
        assert_eq!(a.pipelines.partially_successful, 4);
        assert_eq!(a.pipelines.unsupported, 2);
        assert_eq!(a.pipelines.failed, 1);

        assert_eq!(a.build_steps.total, 120);
        assert_eq!(a.build_steps.successful, 100);
        assert_eq!(a.build_steps.unsupported, 15);
    }

    #[test]
    fn parses_unsupported_steps_with_counts() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(a.unsupported_steps.len(), 3);
        let deploy = a
            .unsupported_steps
            .iter()
            .find(|s| s.task == "acme-corp.deploy.DeployTask@2")
            .expect("custom task present");
        assert_eq!(deploy.count, 7);
    }

    #[test]
    fn parses_manual_tasks_secrets_and_runners() {
        let a = parse_audit_summary(FIXTURE);
        let secrets: Vec<_> = a
            .manual_tasks
            .iter()
            .filter(|m| m.kind == ManualTaskKind::Secret)
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(
            secrets,
            ["AZURE_CLIENT_SECRET", "SONAR_TOKEN", "REGISTRY_PASSWORD"]
        );

        let runners: Vec<_> = a
            .manual_tasks
            .iter()
            .filter(|m| m.kind == ManualTaskKind::SelfHostedRunner)
            .map(|m| m.name.as_str())
            .collect();
        assert_eq!(runners, ["linux-pool", "macos-pool"]);
    }

    #[test]
    fn parses_actions_allowlist_without_counts() {
        let a = parse_audit_summary(FIXTURE);
        assert_eq!(
            a.actions,
            [
                "actions/checkout@v4",
                "actions/setup-node@v4",
                "azure/login@v2"
            ]
        );
    }

    #[test]
    fn unknown_sections_are_ignored() {
        let md = "# Surprise\n- something: 5\n# Pipelines\n- Total: 3\n";
        let a = parse_audit_summary(md);
        assert_eq!(a.pipelines.total, 3);
        assert!(a.actions.is_empty());
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
        assert_eq!(imp.audit().await.unwrap().pipelines.total, 16);

        let r = imp.dry_run("payments-api-deploy").await.unwrap();
        assert_eq!(r.pipeline_id, "payments-api-deploy"); // id overridden to request
        assert!(!r.gaps.is_empty());
    }
}
