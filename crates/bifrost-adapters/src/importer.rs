//! Wrapper around the official `gh actions-importer`.
//!
//! For now this provides [`parse_audit_summary`], which turns the Importer's
//! `audit_summary.md` into the typed [`AuditSummary`]. The Docker subprocess
//! driver that produces that file lands behind the same module later — we wrap
//! the official tool and parse its output; we never reimplement it.

use bifrost_core::{AuditCounts, AuditSummary, ManualTask, ManualTaskKind, UnsupportedStep};

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
}
