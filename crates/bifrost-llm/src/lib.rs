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

/// The model's structured answer. Note: **no risk score** — scoring is deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GapFillResponse {
    pub proposed_yaml: String,
    pub rationale: String,
    pub risk_flags: Vec<String>,
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

/// Selects a provider, enforcing air-gap policy.
pub struct Router<'a> {
    providers: Vec<&'a dyn LlmProvider>,
    air_gap: bool,
}

impl<'a> Router<'a> {
    pub fn new(providers: Vec<&'a dyn LlmProvider>, air_gap: bool) -> Self {
        Self { providers, air_gap }
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
        // Round-trips as JSON (the wire contract) and has no `score`/`riskScore` field.
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("score"), "response carries no risk score");
        assert!(json.contains("proposedYaml"));
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
}
