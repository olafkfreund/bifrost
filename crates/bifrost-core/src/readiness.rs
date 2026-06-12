//! Target (GitHub) readiness checklist (#239) — "is GitHub ready to receive it?"
//!
//! The pre-flight partner to the source assessment. A **deterministic** checklist
//! of what must be true in the target org before migrating. Items Bifrost can
//! derive from the audit (secrets to create, connections to federate, runners to
//! provision, actions to allow) get a concrete count and an `Action` status;
//! operational gates Bifrost cannot verify (SSO, branch rulesets, rollback plan)
//! are `Unverified` with guidance rather than a false green. No LLM.

use serde::{Deserialize, Serialize};

use crate::audit::ManualTaskKind;
use crate::model::Portfolio;

/// Where a readiness item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReadinessStatus {
    /// Satisfied / nothing to do.
    Ready,
    /// Known work, quantified from the audit.
    Action,
    /// Bifrost cannot check this — a human must confirm it in GitHub.
    Unverified,
    /// A hard blocker.
    Blocked,
}

/// One pre-flight checklist item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessItem {
    pub category: String,
    pub status: ReadinessStatus,
    pub detail: String,
    pub action: String,
}

fn item(category: &str, status: ReadinessStatus, detail: String, action: &str) -> ReadinessItem {
    ReadinessItem {
        category: category.into(),
        status,
        detail,
        action: action.into(),
    }
}

/// Build the target readiness checklist for a portfolio. Deterministic.
pub fn readiness(portfolio: &Portfolio) -> Vec<ReadinessItem> {
    let a = &portfolio.audit;
    let secrets = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::Secret)
        .count();
    let runners = a
        .manual_tasks
        .iter()
        .filter(|m| m.kind == ManualTaskKind::SelfHostedRunner)
        .count();
    let connections = a.service_connections.len();
    let variable_groups = a.variable_groups.len();
    let actions = a.actions.len();

    // Action when there's quantified work, else Ready ("nothing to set up").
    let acted = |n: usize| {
        if n > 0 {
            ReadinessStatus::Action
        } else {
            ReadinessStatus::Ready
        }
    };

    vec![
        item(
            "Identity & SSO",
            ReadinessStatus::Unverified,
            "SAML/OIDC SSO and SCIM provisioning must be configured for the target org.".into(),
            "Confirm SSO + SCIM in the GitHub enterprise/org settings.",
        ),
        item(
            "Actions runners",
            if runners > 0 { ReadinessStatus::Action } else { ReadinessStatus::Unverified },
            if runners > 0 {
                format!("{runners} self-hosted runner(s) referenced; size GitHub runners to the forecast peak concurrency.")
            } else {
                "GitHub-hosted runners assumed; confirm concurrency/limits against the forecast.".into()
            },
            "Provision runners/runner-groups and match the labels the workflows expect.",
        ),
        item(
            "Actions policy",
            acted(actions),
            format!("{actions} action(s) the converted workflows use must be allowed."),
            "Add them to the org Actions allow-list (pin to SHAs for supply-chain safety).",
        ),
        item(
            "OIDC federation",
            acted(connections),
            format!("{connections} service connection(s) need OIDC/Entra workload-identity federation."),
            "Configure federated credentials. Note: GitHub's OIDC sub claim format changes for repos created after 2026-07-15.",
        ),
        item(
            "Secret management",
            acted(secrets),
            format!("{secrets} secret(s) to create (names only — values are never read)."),
            "Create them as Actions secrets at the right scope (repo/env/org).",
        ),
        item(
            "Variables",
            acted(variable_groups),
            format!("{variable_groups} variable group(s) to recreate."),
            "Recreate as repo/org/environment variables; ADO stage-scoped variables have no direct equivalent.",
        ),
        item(
            "Branch rulesets",
            ReadinessStatus::Unverified,
            "Branch protection (required reviews, status checks, no force-push) should be defined before import.".into(),
            "Define org/repo rulesets; note rulesets can block a migration if they conflict.",
        ),
        item(
            "Ownership / RACI",
            ReadinessStatus::Unverified,
            format!("Assign an owner per project ({} projects) and a change board.", portfolio.summary.totals.projects),
            "Confirm ownership; owning team per pipeline is not yet collected (see Assessment).",
        ),
        item(
            "Rollback plan",
            ReadinessStatus::Unverified,
            "A documented rollback is required before cutover.".into(),
            "Keep Azure DevOps live ~30 days post-cutover; document how to revert.",
        ),
        item(
            "Egress posture",
            ReadinessStatus::Ready,
            if portfolio.summary.air_gap {
                "Air-gap is ON — only in-network providers are used; no pipeline data leaves the box.".into()
            } else {
                "Air-gap is OFF — frontier providers are permitted; conversion data may leave the network.".into()
            },
            "Confirm the egress posture matches your compliance requirement.",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{ManualTask, UnsupportedStep};
    use crate::ingestion::ServiceConnection;
    use crate::model::{Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals};

    fn portfolio(air_gap: bool, audit: PortfolioAudit) -> Portfolio {
        Portfolio {
            summary: PortfolioSummary {
                org: "o".into(),
                importer_version: "v".into(),
                importer_image_digest: String::new(),
                ado2gh_version: "n/a".into(),
                air_gap,
                generated_at: "t".into(),
                totals: PortfolioTotals {
                    pipelines: 1,
                    orgs: 1,
                    projects: 2,
                    yaml: 1,
                    classic: 0,
                    green: 1,
                    amber: 0,
                    red: 0,
                    forecast_minutes: 0,
                },
            },
            pipelines: vec![],
            audit,
        }
    }

    #[test]
    fn derives_action_items_from_audit_and_marks_gates_unverified() {
        let audit = PortfolioAudit {
            manual_tasks: vec![ManualTask {
                kind: ManualTaskKind::Secret,
                name: "S".into(),
            }],
            unsupported_steps: vec![UnsupportedStep {
                task: "X".into(),
                count: 1,
            }],
            actions: vec!["actions/checkout@v4".into()],
            service_connections: vec![ServiceConnection {
                id: "1".into(),
                name: "c".into(),
                kind: "azurerm".into(),
                project: "P".into(),
            }],
            variable_groups: vec![],
            forecast_capacity: None,
        };
        let rows = readiness(&portfolio(false, audit));
        let by = |c: &str| rows.iter().find(|r| r.category == c).unwrap().clone();

        assert_eq!(by("Secret management").status, ReadinessStatus::Action);
        assert_eq!(by("OIDC federation").status, ReadinessStatus::Action);
        assert_eq!(by("Variables").status, ReadinessStatus::Ready); // none present
        assert_eq!(by("Identity & SSO").status, ReadinessStatus::Unverified);
        assert!(by("OIDC federation").action.contains("2026-07-15"));
    }

    #[test]
    fn egress_reflects_air_gap() {
        let on = readiness(&portfolio(true, PortfolioAudit::default()));
        let off = readiness(&portfolio(false, PortfolioAudit::default()));
        assert!(on
            .iter()
            .any(|r| r.category == "Egress posture" && r.detail.contains("ON")));
        assert!(off
            .iter()
            .any(|r| r.category == "Egress posture" && r.detail.contains("OFF")));
    }
}
