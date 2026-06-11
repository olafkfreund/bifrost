//! The manual-task **runbook** — the checklist of things the Importer cannot do
//! for you, derived from a pipeline's gaps.
//!
//! The Importer converts steps, but some work is inherently human: provisioning
//! secrets, federating service connections to OIDC, standing up self-hosted
//! runners, recreating approval gates as Environments, and replacing custom or
//! marketplace tasks that have no first-party action. [`Runbook::from_gaps`]
//! turns the typed [`Gap`]s into an actionable, categorised checklist a reviewer
//! works through before a migration is "done" (it pairs with the [`Proposal`]).
//!
//! Categorisation is keyword-based and conservative, mirroring the risk
//! signals' mapping (see [`crate::conversion::signals_from_dry_run`]) so the
//! checklist and the score never disagree about what a gap is. Secret *names*
//! are data; secret values are never present here.
//!
//! [`Proposal`]: crate::proposal::Proposal

use serde::{Deserialize, Serialize};

use crate::gap::{Gap, GapKind};

/// What kind of manual follow-up a checklist item represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistCategory {
    /// Provision a repository/organisation secret (by name).
    Secret,
    /// Federate an Azure DevOps service connection to GitHub via OIDC.
    ServiceConnection,
    /// Recreate an Azure DevOps variable group.
    VariableGroup,
    /// Stand up (or choose) a self-hosted runner / pool.
    SelfHostedRunner,
    /// Recreate an approval gate / environment as a GitHub Environment.
    Environment,
    /// Replace a custom/marketplace task that has no first-party action.
    ReplacementAction,
    /// Anything else the Importer flagged for a human.
    Other,
}

impl ChecklistCategory {
    /// A stable, actionable verb phrase for the category (the item's title).
    fn title(self) -> &'static str {
        match self {
            Self::Secret => "Provision repository secret",
            Self::ServiceConnection => "Federate service connection to GitHub via OIDC",
            Self::VariableGroup => "Recreate variable group",
            Self::SelfHostedRunner => "Provision or select a runner",
            Self::Environment => "Recreate approval gate as a GitHub Environment",
            Self::ReplacementAction => "Find or author a replacement action",
            Self::Other => "Resolve manual task",
        }
    }
}

/// One actionable item in the [`Runbook`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub category: ChecklistCategory,
    /// Actionable summary, e.g. "Provision repository secret".
    pub title: String,
    /// The source construct / name the action applies to (e.g. a secret name).
    pub construct: String,
    /// The Importer's message — the specifics for the reviewer.
    pub detail: String,
    /// Whether this task must be resolved before the migration can be validated.
    #[serde(default = "default_true")]
    pub required: bool,
    /// Whether a human has marked this task done (#57).
    #[serde(default)]
    pub done: bool,
}

fn default_true() -> bool {
    true
}

/// The ordered checklist of manual tasks for a pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Runbook {
    pub items: Vec<ChecklistItem>,
}

impl Runbook {
    /// Build the checklist from a pipeline's gaps.
    ///
    /// Manual-task gaps are categorised by keyword; unsupported *namespaced*
    /// tasks (custom/marketplace, e.g. `acme.deploy.Task@2`) become
    /// replacement-action items. Partial constructs and first-party unsupported
    /// steps are left to the proposal's gap-fills, not the human checklist.
    pub fn from_gaps(gaps: &[Gap]) -> Self {
        let items = gaps.iter().filter_map(item_for_gap).collect();
        Self { items }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Items of a single category, in checklist order.
    pub fn of(&self, category: ChecklistCategory) -> impl Iterator<Item = &ChecklistItem> {
        self.items.iter().filter(move |i| i.category == category)
    }

    /// Number of required tasks not yet marked done. The migration is not
    /// "done" (cannot be validated) until this is zero (#57).
    pub fn required_remaining(&self) -> usize {
        self.items.iter().filter(|i| i.required && !i.done).count()
    }
}

/// Whether a gap is human/manual work (claimed by the [`Runbook`]) rather than
/// LLM gap-fill work. This is the single source of truth for the conversion
/// loop's split: manual gaps become checklist items; everything else
/// (first-party unsupported steps, partial constructs) is routed to the LLM.
///
/// Manual = any [`GapKind::ManualTask`], plus *namespaced* unsupported tasks
/// (e.g. `acme.deploy.Task@2`) — custom/marketplace tasks a human must replace.
pub fn gap_is_manual(gap: &Gap) -> bool {
    matches!(gap.kind, GapKind::ManualTask)
        || (gap.kind == GapKind::UnsupportedStep && gap.construct.contains('.'))
}

/// Map a single gap to a checklist item, or `None` if it is not manual work.
fn item_for_gap(gap: &Gap) -> Option<ChecklistItem> {
    if !gap_is_manual(gap) {
        // First-party unsupported steps and partial constructs are the LLM's job
        // (gap-fill), not a manual checklist item.
        return None;
    }
    let category = match gap.kind {
        GapKind::ManualTask => categorize_manual(&gap.construct, &gap.detail),
        // A namespaced task id is a custom/marketplace task — a human must find
        // or author an equivalent action (guaranteed namespaced by gap_is_manual).
        _ => ChecklistCategory::ReplacementAction,
    };
    Some(ChecklistItem {
        category,
        title: category.title().to_string(),
        construct: gap.construct.clone(),
        detail: gap.detail.clone(),
        required: true,
        done: false,
    })
}

/// Categorise a manual-task gap by keyword across its construct + detail.
fn categorize_manual(construct: &str, detail: &str) -> ChecklistCategory {
    let text = format!("{construct} {detail}").to_ascii_lowercase();
    if text.contains("secret") {
        ChecklistCategory::Secret
    } else if text.contains("connection") {
        ChecklistCategory::ServiceConnection
    } else if text.contains("variable") {
        ChecklistCategory::VariableGroup
    } else if text.contains("self-hosted")
        || text.contains("self hosted")
        || text.contains("runner")
        || text.contains("pool")
    {
        ChecklistCategory::SelfHostedRunner
    } else if text.contains("environment") || text.contains("approval") || text.contains("gate") {
        ChecklistCategory::Environment
    } else {
        ChecklistCategory::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(kind: GapKind, construct: &str, detail: &str) -> Gap {
        Gap {
            kind,
            construct: construct.into(),
            detail: detail.into(),
        }
    }

    #[test]
    fn categorises_manual_tasks_from_real_sarc_shape() {
        // Mirrors the manual tasks in fixtures/dry_run.log.
        let gaps = vec![
            gap(
                GapKind::ManualTask,
                "secret",
                "AZURE_CLIENT_SECRET must be configured as a repository secret",
            ),
            gap(
                GapKind::ManualTask,
                "service-connection",
                "azure-prod must be federated to GitHub via OIDC",
            ),
            gap(
                GapKind::ManualTask,
                "environment",
                "pre-deploy approval gate must be recreated as an Environment",
            ),
        ];
        let rb = Runbook::from_gaps(&gaps);
        assert_eq!(rb.len(), 3);
        assert_eq!(rb.of(ChecklistCategory::Secret).count(), 1);
        assert_eq!(rb.of(ChecklistCategory::ServiceConnection).count(), 1);
        assert_eq!(rb.of(ChecklistCategory::Environment).count(), 1);
        // Title is actionable; the secret name is carried as data (not the value).
        let secret = rb.of(ChecklistCategory::Secret).next().unwrap();
        assert_eq!(secret.title, "Provision repository secret");
        assert!(secret.detail.contains("AZURE_CLIENT_SECRET"));
    }

    #[test]
    fn namespaced_unsupported_task_becomes_a_replacement_action() {
        let gaps = vec![gap(
            GapKind::UnsupportedStep,
            "acme-corp.deploy.DeployTask@2",
            "custom marketplace task, no first-party action",
        )];
        let rb = Runbook::from_gaps(&gaps);
        assert_eq!(rb.len(), 1);
        assert_eq!(rb.items[0].category, ChecklistCategory::ReplacementAction);
        assert_eq!(rb.items[0].construct, "acme-corp.deploy.DeployTask@2");
    }

    #[test]
    fn first_party_steps_and_partial_constructs_are_not_manual() {
        // These are the LLM's job (gap-fill), not the human checklist.
        let gaps = vec![
            gap(
                GapKind::UnsupportedStep,
                "DownloadSecureFile@1",
                "no equivalent",
            ),
            gap(
                GapKind::PartialConstruct,
                "strategy.matrix",
                "reduced fidelity",
            ),
        ];
        assert!(Runbook::from_gaps(&gaps).is_empty());
    }

    #[test]
    fn self_hosted_runner_and_variable_group_categorised() {
        let gaps = vec![
            gap(
                GapKind::ManualTask,
                "pool",
                "self-hosted agent pool 'linux-build' must be provisioned",
            ),
            gap(
                GapKind::ManualTask,
                "variable-group",
                "variable group 'shared-config' must be recreated",
            ),
        ];
        let rb = Runbook::from_gaps(&gaps);
        assert_eq!(rb.of(ChecklistCategory::SelfHostedRunner).count(), 1);
        assert_eq!(rb.of(ChecklistCategory::VariableGroup).count(), 1);
    }
}
