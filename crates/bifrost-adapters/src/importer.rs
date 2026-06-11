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
        // The log carries gaps, not the converted YAML; callers that have the
        // Importer's output files set `converted_yaml` separately.
        converted_yaml: String::new(),
        // The source definition comes from the SourceAdapter, not the log.
        source_yaml: String::new(),
    }
}

/// Parse the gaps out of a converted GitHub Actions workflow the Importer emits.
///
/// The Importer leaves unsupported steps as commented `# This item has no
/// matching transformer` blocks, maps service connections / secret refs to
/// `${{ secrets.* }}` (a human must still provision the values), and writes
/// `environment:` blocks (a human must create the Environment). We turn each into
/// a typed [`Gap`] so the conversion loop can split them (LLM gap-fill vs. the
/// manual runbook). The converted YAML itself is the Importer's baseline output.
pub fn parse_converted_workflow(workflow: &str) -> Vec<Gap> {
    let lines: Vec<&str> = workflow.lines().collect();
    let mut gaps = Vec::new();

    // 1. Unsupported steps: a "no matching transformer" marker, then the first
    //    following commented `task:` line names the construct.
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("no matching transformer") {
            continue;
        }
        for next in lines.iter().skip(i + 1).take(8) {
            let trimmed = next.trim_start_matches(['#', ' ', '-']);
            if let Some(rest) = trimmed.strip_prefix("task:") {
                gaps.push(Gap {
                    kind: GapKind::UnsupportedStep,
                    construct: rest.trim().to_string(),
                    detail: "no matching GitHub Actions transformer — needs a replacement".into(),
                });
                break;
            }
        }
    }

    // 2. Secrets to provision (mapped from service connections / secret refs).
    let mut secrets = std::collections::BTreeSet::new();
    let mut rest = workflow;
    while let Some(pos) = rest.find("secrets.") {
        if let Some(name) = secret_name(&rest[pos..]) {
            secrets.insert(name);
        }
        rest = &rest[pos + "secrets.".len()..];
    }
    for name in secrets {
        gaps.push(Gap {
            kind: GapKind::ManualTask,
            construct: "secret".into(),
            detail: format!("{name} must be provisioned as a repository/organization secret"),
        });
    }

    // 3. Environments to recreate (`environment:` then a `name:`).
    for (i, line) in lines.iter().enumerate() {
        if line.trim_end().ends_with("environment:") {
            if let Some(name) = lines
                .get(i + 1)
                .and_then(|l| l.trim().strip_prefix("name:"))
            {
                gaps.push(Gap {
                    kind: GapKind::ManualTask,
                    construct: "environment".into(),
                    detail: format!("{} must be recreated as a GitHub Environment", name.trim()),
                });
            }
        }
    }

    gaps
}

/// Share of steps converted, given the converted workflow and its gaps. Counts
/// real (uncommented) step starts against those plus the unsupported steps.
pub fn converted_ratio(workflow: &str, gaps: &[Gap]) -> f64 {
    let converted = workflow
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#')
                && t.starts_with("- ")
                && (t.contains("uses:") || t.contains("run:") || t.contains("name:"))
        })
        .count();
    let unsupported = gaps
        .iter()
        .filter(|g| g.kind == GapKind::UnsupportedStep)
        .count();
    let total = converted + unsupported;
    if total == 0 {
        1.0
    } else {
        converted as f64 / total as f64
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

/// Estimated GitHub Actions runner usage from `gh actions-importer forecast`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forecast {
    /// Estimated total runner-minutes/month across the org.
    pub total_minutes: u32,
    /// Per-pipeline estimates by pipeline name (best-effort from the report).
    pub per_pipeline: Vec<(String, u32)>,
}

/// Parse a `forecast_report.md` into a [`Forecast`]. Tolerant by design: it keys
/// on `## Total` / `### <pipeline>` headers and any line mentioning
/// "runner minutes" with a number, so minor prose changes in the report don't
/// break it. Falls back to summing per-pipeline when no explicit total is found.
pub fn parse_forecast(md: &str) -> Forecast {
    let mut total = 0u32;
    let mut per_pipeline = Vec::new();
    let mut current: Option<String> = None;
    let mut in_total = false;

    for line in md.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("## ") {
            in_total = h.trim().eq_ignore_ascii_case("total");
            current = None;
            continue;
        }
        if let Some(name) = t.strip_prefix("### ") {
            current = Some(name.trim().to_string());
            continue;
        }
        if let Some(n) = runner_minutes(t) {
            match &current {
                Some(name) => per_pipeline.push((name.clone(), n)),
                None if in_total => total = n,
                None => {}
            }
        }
    }

    if total == 0 {
        total = per_pipeline.iter().map(|(_, m)| m).sum();
    }
    Forecast {
        total_minutes: total,
        per_pipeline,
    }
}

/// The runner-minutes figure on a report line (commas stripped), if any.
fn runner_minutes(line: &str) -> Option<u32> {
    if !line.to_ascii_lowercase().contains("runner minutes") {
        return None;
    }
    let digits: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|n| *n > 0)
}

/// The official `gh actions-importer`, wrapped behind a trait so orchestration
/// can be tested without Docker. The real driver shells out to the pinned image.
#[async_trait]
pub trait Importer: Send + Sync {
    /// The Importer version/digest in use (recorded per job for attestation).
    async fn version(&self) -> Result<String, ImporterError>;
    /// Audit the org and return the parsed footprint.
    async fn audit(&self) -> Result<AuditSummary, ImporterError>;
    /// Forecast estimated GitHub Actions runner usage for the org.
    async fn forecast(&self) -> Result<Forecast, ImporterError>;
    /// Dry-run a single pipeline and return its converted ratio + gaps.
    async fn dry_run(&self, pipeline_id: &str) -> Result<DryRunResult, ImporterError>;
}

/// A fixture-backed [`Importer`] for tests and offline runs.
#[derive(Debug, Clone, Default)]
pub struct MockImporter;

const FIXTURE_AUDIT: &str = include_str!("../../../fixtures/audit_summary.md");
const FIXTURE_DRY_RUN: &str = include_str!("../../../fixtures/dry_run.log");
const FIXTURE_DRY_RUN_YAML: &str = include_str!("../../../fixtures/dry_run_converted.yml");
const FIXTURE_SOURCE_YAML: &str = include_str!("../../../fixtures/source_pipeline.yml");
const FIXTURE_FORECAST: &str = include_str!("../../../fixtures/forecast_report.md");

#[async_trait]
impl Importer for MockImporter {
    async fn version(&self) -> Result<String, ImporterError> {
        Ok("mock-importer".into())
    }

    async fn audit(&self) -> Result<AuditSummary, ImporterError> {
        Ok(parse_audit_summary(FIXTURE_AUDIT))
    }

    async fn forecast(&self) -> Result<Forecast, ImporterError> {
        Ok(parse_forecast(FIXTURE_FORECAST))
    }

    async fn dry_run(&self, pipeline_id: &str) -> Result<DryRunResult, ImporterError> {
        let mut result = parse_dry_run(FIXTURE_DRY_RUN);
        result.pipeline_id = pipeline_id.to_string();
        // The log fixture carries only gaps; pair it with the Importer's baseline
        // workflow so the conversion loop has something to assemble against, and
        // with the source ADO pipeline so the review diff has a left-hand side.
        result.converted_yaml = FIXTURE_DRY_RUN_YAML.to_string();
        result.source_yaml = FIXTURE_SOURCE_YAML.to_string();
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../fixtures/audit_summary.md");
    const FORECAST: &str = include_str!("../../../fixtures/forecast_report.md");

    #[test]
    fn parse_forecast_extracts_total_and_per_pipeline() {
        let f = parse_forecast(FORECAST);
        // Total from the "## Total" section (commas stripped).
        assert_eq!(f.total_minutes, 23_500);
        assert_eq!(f.per_pipeline.len(), 3);
        assert!(f
            .per_pipeline
            .iter()
            .any(|(n, m)| n == "payments-api-ci" && *m == 6_800));
    }

    #[test]
    fn parse_forecast_falls_back_to_sum_without_an_explicit_total() {
        let md = "## Pipeline details\n\n### a\n- Estimated runner minutes per month: 100\n\
                  \n### b\n- Estimated runner minutes per month: 250\n";
        let f = parse_forecast(md);
        assert_eq!(f.total_minutes, 350);
    }

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

    // A real converted workflow captured from `gh actions-importer` (Contoso-CI).
    const CONVERTED: &str = include_str!("../../../fixtures/importer_converted_workflow.yml");

    #[test]
    fn parses_unsupported_steps_from_a_real_converted_workflow() {
        let gaps = parse_converted_workflow(CONVERTED);
        let unsupported: Vec<_> = gaps
            .iter()
            .filter(|g| g.kind == GapKind::UnsupportedStep)
            .map(|g| g.construct.as_str())
            .collect();
        // The Importer commented these out as "no matching transformer".
        assert!(
            unsupported.contains(&"DownloadSecureFile@1"),
            "got {unsupported:?}"
        );
        assert!(
            unsupported.contains(&"SonarQubePrepare@5"),
            "got {unsupported:?}"
        );
        // Converted ratio is between 0 and 1 (some steps converted, some not).
        let ratio = converted_ratio(CONVERTED, &gaps);
        assert!(ratio > 0.0 && ratio < 1.0, "ratio = {ratio}");
    }

    #[test]
    fn parses_secret_and_environment_manual_tasks() {
        let wf = "jobs:\n  deploy:\n    environment:\n      name: contoso-prod\n    steps:\n      - uses: azure/login@v1\n        with:\n          creds: \"${{ secrets.AZURE_CREDENTIALS }}\"\n";
        let gaps = parse_converted_workflow(wf);
        assert!(gaps
            .iter()
            .any(|g| g.construct == "secret" && g.detail.contains("AZURE_CREDENTIALS")));
        assert!(gaps
            .iter()
            .any(|g| g.construct == "environment" && g.detail.contains("contoso-prod")));
    }
}
