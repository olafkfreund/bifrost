//! LLM provider layer.
//!
//! The `LlmProvider` trait abstracts every LLM backend; orchestration calls only
//! this trait, never a vendor SDK directly. Two hard rules are encoded here:
//!
//! - **Grounded generation only.** A [`GapFillRequest`] carries the source
//!   snippet + the Importer's converted output + the specific failure; the model
//!   fills *that gap* from the diff — it never converts a pipeline from scratch.
//! - **The LLM explains; it does not score.** [`GapFillResponse`] has no risk
//!   score — risk is computed deterministically in `bifrost-core`. `confidence`
//!   is only the model's certainty in its proposed YAML.
//!
//! Air-gap capability is a first-class concern: see [`LlmProvider::is_local`] and
//! [`Router`].

mod anthropic;
mod copilot;
mod gemini;
mod ollama;
mod openai_compatible;

pub use anthropic::AnthropicProvider;
pub use copilot::CopilotProvider;
pub use gemini::GeminiProvider;
pub use ollama::OllamaProvider;
pub use openai_compatible::OpenAiCompatibleProvider;

use async_trait::async_trait;
use bifrost_core::Gap;
use serde::{Deserialize, Serialize};

/// A grounded request to fill one [`Gap`]. The model works only from this diff.
#[derive(Debug, Clone)]
pub struct GapFillRequest {
    pub gap: Gap,
    /// The source construct (Azure DevOps task/snippet).
    pub source_snippet: String,
    /// The Importer's converted GitHub Actions output so far.
    pub converted_yaml: String,
    /// The specific failure the Importer reported for this construct.
    pub importer_message: String,
    /// Repo context (languages, detected build tools).
    pub repo_context: String,
}

/// Accept `proposed_yaml` as a string, or coerce a non-string (some local models
/// emit the YAML as a nested JSON object) into a string — JSON is valid YAML, so
/// the result is still a reviewable fragment. Richer coercion / prompt-eval is #103.
fn string_or_stringify<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        other => serde_json::to_string_pretty(&other).unwrap_or_default(),
    })
}

/// The model's structured answer. Note: **no risk score** — scoring is deterministic.
///
/// Field names are snake_case to match the `gap-fill.v1` prompt's JSON spec
/// (which is what real models follow). camelCase aliases are accepted too, so a
/// model that returns either casing parses — local models in particular are
/// inconsistent here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapFillResponse {
    #[serde(alias = "proposedYaml", deserialize_with = "string_or_stringify")]
    pub proposed_yaml: String,
    pub rationale: String,
    #[serde(alias = "riskFlags")]
    pub risk_flags: Vec<String>,
    #[serde(alias = "verifySteps")]
    pub verify_steps: Vec<String>,
    /// Model's certainty in `proposed_yaml` (0.0–1.0) — NOT a migration risk score.
    pub confidence: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("provider transport error: {0}")]
    Transport(String),
    #[error("could not parse model output as JSON: {0}")]
    Parse(String),
    #[error("provider '{0}' is disabled in air-gap mode")]
    AirGapBlocked(String),
}

/// An LLM backend. Implementations: Anthropic, Gemini, Copilot/Models, Ollama.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Stable provider id (used in routing and the audit log).
    fn name(&self) -> &str;
    /// Whether the provider runs locally (no pipeline data leaves the network).
    /// Only local providers are permitted in air-gap mode.
    fn is_local(&self) -> bool;
    /// Fill a single gap, grounded in the request's diff.
    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError>;
}

/// The versioned grounded prompt template (referenced by id for auditability).
pub const GAP_FILL_PROMPT_ID: &str = "gap-fill.v1";
const GAP_FILL_TEMPLATE: &str = include_str!("../../../prompts/gap-fill.v1.md");

/// Render the grounded gap-fill prompt for `req` from the versioned template.
pub fn build_gap_fill_prompt(req: &GapFillRequest) -> String {
    // Strip the leading `{# ... #}` template comment.
    let body = match (GAP_FILL_TEMPLATE.find("{#"), GAP_FILL_TEMPLATE.find("#}")) {
        (Some(a), Some(b)) if b > a => {
            let mut s = GAP_FILL_TEMPLATE.to_string();
            s.replace_range(a..b + 2, "");
            s.trim_start().to_string()
        }
        _ => GAP_FILL_TEMPLATE.to_string(),
    };
    body.replace("{{source_snippet}}", &req.source_snippet)
        .replace("{{converted_yaml}}", &req.converted_yaml)
        .replace("{{importer_message}}", &req.importer_message)
        .replace("{{repo_context}}", &req.repo_context)
}

/// Parse a model's raw text answer into a [`GapFillResponse`].
///
/// Models often wrap JSON in ```` ```json ```` fences or add a sentence before
/// it, so we extract the outermost `{ … }` object before deserializing. Shared
/// by every provider so the wire contract is enforced in exactly one place.
pub(crate) fn parse_gap_fill(text: &str) -> Result<GapFillResponse, LlmError> {
    let json = extract_json_object(text)
        .ok_or_else(|| LlmError::Parse(format!("no JSON object in model output: {text}")))?;
    serde_json::from_str(json).map_err(|e| LlmError::Parse(format!("{e}: {json}")))
}

/// Slice out the outermost balanced `{ … }` object from `s`, ignoring braces
/// inside strings. Returns `None` if there is no balanced object.
fn extract_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// What kind of work a gap-fill represents — drives provider selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskClass {
    /// High-volume, mechanical fills — prefer a cheap/local provider.
    Bulk,
    /// Hard semantic reasoning — prefer a frontier provider.
    HardReasoning,
    /// Documentation / rationale prose — prefer a frontier provider.
    Documentation,
}

/// Config-driven routing: an ordered provider-name preference per task class.
/// The first preference that is *usable* (present, and local when air-gap) wins.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoutingPolicy {
    /// Preference order for [`TaskClass::Bulk`].
    pub bulk: Vec<String>,
    /// Preference order for [`TaskClass::HardReasoning`].
    pub hard: Vec<String>,
    /// Preference order for [`TaskClass::Documentation`].
    pub docs: Vec<String>,
}

impl Default for RoutingPolicy {
    /// Bulk leans local-first (cheap); reasoning and docs lean frontier-first.
    /// Every list ends with the other provider so a single-provider deployment
    /// (or air-gap, which strips frontier) still resolves.
    fn default() -> Self {
        Self {
            bulk: vec!["ollama".into(), "anthropic".into()],
            hard: vec!["anthropic".into(), "ollama".into()],
            docs: vec!["anthropic".into(), "ollama".into()],
        }
    }
}

impl RoutingPolicy {
    /// Build from env: `BIFROST_ROUTE_BULK`, `BIFROST_ROUTE_HARD`,
    /// `BIFROST_ROUTE_DOCS` as comma-separated provider names; each falls back
    /// to the [`Default`] order when unset or empty.
    pub fn from_env() -> Self {
        let d = Self::default();
        Self {
            bulk: parse_names("BIFROST_ROUTE_BULK").unwrap_or(d.bulk),
            hard: parse_names("BIFROST_ROUTE_HARD").unwrap_or(d.hard),
            docs: parse_names("BIFROST_ROUTE_DOCS").unwrap_or(d.docs),
        }
    }

    fn preferences(&self, class: TaskClass) -> &[String] {
        match class {
            TaskClass::Bulk => &self.bulk,
            TaskClass::HardReasoning => &self.hard,
            TaskClass::Documentation => &self.docs,
        }
    }
}

/// Parse a comma-separated provider-name list from `var`; `None` if unset/empty.
fn parse_names(var: &str) -> Option<Vec<String>> {
    let raw = std::env::var(var).ok()?;
    let names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    (!names.is_empty()).then_some(names)
}

/// Selects a provider, enforcing air-gap policy.
pub struct Router<'a> {
    providers: Vec<&'a dyn LlmProvider>,
    air_gap: bool,
    policy: RoutingPolicy,
}

impl<'a> Router<'a> {
    pub fn new(providers: Vec<&'a dyn LlmProvider>, air_gap: bool) -> Self {
        Self {
            providers,
            air_gap,
            policy: RoutingPolicy::default(),
        }
    }

    /// Override the default routing policy (e.g. one built from env/config).
    pub fn with_policy(mut self, policy: RoutingPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Pick a provider by name, rejecting non-local providers in air-gap mode.
    pub fn select(&self, name: &str) -> Result<&'a dyn LlmProvider, LlmError> {
        let p = self
            .providers
            .iter()
            .copied()
            .find(|p| p.name() == name)
            .ok_or_else(|| LlmError::Transport(format!("no provider '{name}'")))?;
        if self.air_gap && !p.is_local() {
            return Err(LlmError::AirGapBlocked(name.to_string()));
        }
        Ok(p)
    }

    /// Route by task class: walk the policy's preference list and return the
    /// first provider that is present and permitted under air-gap.
    ///
    /// In air-gap mode non-local providers are silently skipped, so the returned
    /// provider is **always local** — a frontier never receives pipeline data
    /// and no external call is ever made through the router.
    pub fn route(&self, class: TaskClass) -> Result<&'a dyn LlmProvider, LlmError> {
        for name in self.policy.preferences(class) {
            if let Some(p) = self.providers.iter().copied().find(|p| p.name() == name) {
                if self.air_gap && !p.is_local() {
                    continue; // frontier provider — never used in air-gap mode
                }
                return Ok(p);
            }
        }
        // No policy preference matched (e.g. a provider configured but absent from
        // the list, like a newly-added backend). Fall back to the first usable
        // provider rather than failing — air-gap still excludes frontiers.
        if let Some(p) = self
            .providers
            .iter()
            .copied()
            .find(|p| !self.air_gap || p.is_local())
        {
            return Ok(p);
        }
        if self.air_gap {
            Err(LlmError::AirGapBlocked(format!(
                "no local provider available for {class:?}"
            )))
        } else {
            Err(LlmError::Transport(format!(
                "no provider available for {class:?}"
            )))
        }
    }
}

/// A canned [`LlmProvider`] for tests and offline runs. Marked local so it's
/// usable in air-gap mode.
#[derive(Debug, Clone, Default)]
pub struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }
    fn is_local(&self) -> bool {
        true
    }
    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        Ok(GapFillResponse {
            proposed_yaml: format!("# gap fill for {}\n", req.gap.construct),
            rationale: format!("Mock fill grounded in: {}", req.importer_message),
            risk_flags: vec!["mock — human review required".into()],
            verify_steps: vec!["run the converted workflow in a sandbox".into()],
            confidence: 0.5,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::GapKind;

    fn req() -> GapFillRequest {
        GapFillRequest {
            gap: Gap {
                kind: GapKind::UnsupportedStep,
                construct: "DownloadSecureFile@1".into(),
                detail: "no GitHub Actions equivalent".into(),
            },
            source_snippet: "- task: DownloadSecureFile@1".into(),
            converted_yaml: "steps: []".into(),
            importer_message: "secure file download has no equivalent".into(),
            repo_context: "languages: dotnet".into(),
        }
    }

    #[test]
    fn prompt_is_grounded_in_the_diff() {
        let p = build_gap_fill_prompt(&req());
        assert!(p.contains("- task: DownloadSecureFile@1"), "embeds source");
        assert!(p.contains("steps: []"), "embeds converted output");
        assert!(
            p.contains("secure file download has no equivalent"),
            "embeds failure"
        );
        assert!(p.contains("languages: dotnet"), "embeds repo context");
        assert!(
            !p.contains("{{source_snippet}}"),
            "placeholders substituted"
        );
        assert!(!p.contains("{#"), "template comment stripped");
        // The prompt instructs the model not to emit a risk score.
        assert!(p.to_lowercase().contains("not output a numeric risk score"));
    }

    #[tokio::test]
    async fn mock_provider_returns_structured_response_without_a_score() {
        let r = MockLlmProvider.fill_gap(&req()).await.unwrap();
        assert!(r.proposed_yaml.contains("DownloadSecureFile@1"));
        assert!(!r.risk_flags.is_empty());
        // Serializes with the prompt's snake_case keys and carries no risk score.
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("score"), "response carries no risk score");
        assert!(json.contains("proposed_yaml"));

        // Parsing tolerates either casing real models emit (snake_case primary,
        // camelCase via alias) — see GapFillResponse.
        let snake = r#"{"proposed_yaml":"x","rationale":"r","risk_flags":[],"verify_steps":[],"confidence":0.5}"#;
        let camel = r#"{"proposedYaml":"x","rationale":"r","riskFlags":[],"verifySteps":[],"confidence":0.5}"#;
        assert_eq!(
            parse_gap_fill(snake).unwrap(),
            parse_gap_fill(camel).unwrap()
        );

        // A model that returns proposed_yaml as an object is coerced to a string
        // (JSON is valid YAML) rather than failing the whole conversion.
        let obj = r#"{"proposed_yaml":{"strategy":{"matrix":{}}},"rationale":"r","risk_flags":[],"verify_steps":[],"confidence":0.5}"#;
        let parsed = parse_gap_fill(obj).expect("object proposed_yaml is coerced");
        assert!(parsed.proposed_yaml.contains("strategy"));
    }

    #[test]
    fn parse_gap_fill_unwraps_fenced_json_with_surrounding_prose() {
        let raw = "Here is the fix:\n```json\n{\n  \"proposedYaml\": \"- run: echo hi\",\n  \
                   \"rationale\": \"equivalent step\",\n  \"riskFlags\": [\"check secret\"],\n  \
                   \"verifySteps\": [\"run in sandbox\"],\n  \"confidence\": 0.8\n}\n```\nDone.";
        let r = parse_gap_fill(raw).expect("extracts JSON from fenced prose");
        assert_eq!(r.proposed_yaml, "- run: echo hi");
        assert_eq!(r.confidence, 0.8);
        assert_eq!(r.risk_flags, vec!["check secret".to_string()]);
    }

    #[test]
    fn parse_gap_fill_errors_when_no_json_present() {
        assert!(matches!(
            parse_gap_fill("I cannot help with that."),
            Err(LlmError::Parse(_))
        ));
    }

    #[tokio::test]
    async fn air_gap_blocks_non_local_providers() {
        struct Frontier;
        #[async_trait]
        impl LlmProvider for Frontier {
            fn name(&self) -> &str {
                "frontier"
            }
            fn is_local(&self) -> bool {
                false
            }
            async fn fill_gap(&self, _: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
                unreachable!()
            }
        }
        let local = MockLlmProvider;
        let frontier = Frontier;
        let router = Router::new(vec![&local, &frontier], /* air_gap */ true);

        assert!(router.select("mock").is_ok(), "local allowed in air-gap");
        assert!(
            matches!(router.select("frontier"), Err(LlmError::AirGapBlocked(_))),
            "frontier blocked in air-gap"
        );
    }

    /// A named test double with explicit locality, for routing tests.
    struct Stub {
        id: &'static str,
        local: bool,
    }
    #[async_trait]
    impl LlmProvider for Stub {
        fn name(&self) -> &str {
            self.id
        }
        fn is_local(&self) -> bool {
            self.local
        }
        async fn fill_gap(&self, _: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
            unreachable!("routing tests never call fill_gap")
        }
    }

    #[test]
    fn routes_by_task_class_under_default_policy() {
        let ollama = Stub {
            id: "ollama",
            local: true,
        };
        let anthropic = Stub {
            id: "anthropic",
            local: false,
        };
        let router = Router::new(vec![&ollama, &anthropic], /* air_gap */ false);

        // Bulk leans local-first; reasoning and docs lean frontier-first.
        assert_eq!(router.route(TaskClass::Bulk).unwrap().name(), "ollama");
        assert_eq!(
            router.route(TaskClass::HardReasoning).unwrap().name(),
            "anthropic"
        );
        assert_eq!(
            router.route(TaskClass::Documentation).unwrap().name(),
            "anthropic"
        );
    }

    #[test]
    fn routes_to_a_configured_provider_absent_from_the_policy() {
        // "gemini" is in no default preference list, but it's the only provider
        // configured — the fallback returns it rather than failing.
        let gemini = Stub {
            id: "gemini",
            local: false,
        };
        let router = Router::new(vec![&gemini], /* air_gap */ false);
        assert_eq!(
            router.route(TaskClass::HardReasoning).unwrap().name(),
            "gemini"
        );
        assert_eq!(router.route(TaskClass::Bulk).unwrap().name(), "gemini");
    }

    #[test]
    fn air_gap_routing_never_returns_a_frontier_provider() {
        let ollama = Stub {
            id: "ollama",
            local: true,
        };
        let anthropic = Stub {
            id: "anthropic",
            local: false,
        };
        let router = Router::new(vec![&ollama, &anthropic], /* air_gap */ true);

        // HardReasoning prefers the frontier, but air-gap skips it and falls
        // back to the local provider — so no external call is ever made.
        for class in [
            TaskClass::Bulk,
            TaskClass::HardReasoning,
            TaskClass::Documentation,
        ] {
            let p = router.route(class).expect("a local provider resolves");
            assert!(
                p.is_local(),
                "{class:?} must route to a local provider in air-gap"
            );
            assert_eq!(p.name(), "ollama");
        }
    }

    #[test]
    fn air_gap_with_only_a_frontier_provider_errors() {
        let anthropic = Stub {
            id: "anthropic",
            local: false,
        };
        let router = Router::new(vec![&anthropic], /* air_gap */ true);
        assert!(
            matches!(
                router.route(TaskClass::HardReasoning),
                Err(LlmError::AirGapBlocked(_))
            ),
            "no local provider → AirGapBlocked, never a silent frontier call"
        );
    }

    #[test]
    fn custom_policy_overrides_preference_order() {
        let ollama = Stub {
            id: "ollama",
            local: true,
        };
        let anthropic = Stub {
            id: "anthropic",
            local: false,
        };
        let policy = RoutingPolicy {
            bulk: vec!["anthropic".into(), "ollama".into()],
            ..RoutingPolicy::default()
        };
        let router =
            Router::new(vec![&ollama, &anthropic], /* air_gap */ false).with_policy(policy);
        assert_eq!(
            router.route(TaskClass::Bulk).unwrap().name(),
            "anthropic",
            "custom policy sends Bulk to the frontier"
        );
    }
}
