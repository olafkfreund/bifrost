//! GitHub Models ("Copilot") [`LlmProvider`].
//!
//! GitHub Models exposes an **OpenAI-compatible** chat-completions endpoint, so
//! we call it over raw HTTP (`reqwest` + rustls) with a Bearer token. A frontier
//! provider (`is_local` is `false`), so the [`Router`](crate::Router) blocks it
//! in air-gap mode. Grounded by [`build_gap_fill_prompt`](crate::build_gap_fill_prompt);
//! the shared [`parse_gap_fill`](crate::parse_gap_fill) enforces the response
//! contract (no risk score).
//!
//! Per the M0 spike (#18), GitHub Models is an experimentation surface with its
//! own rate limits and Terms of Service — confirm those before relying on it in
//! production. The token (`GITHUB_MODELS_TOKEN`, or `GITHUB_TOKEN`) needs
//! `models` access and is sent in the `Authorization` header, never the URL.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

/// Base for the OpenAI-compatible inference API; `/chat/completions` is appended.
const DEFAULT_BASE_URL: &str = "https://models.github.ai/inference";
/// Default model — overridable via `GITHUB_MODELS_MODEL`.
const DEFAULT_MODEL: &str = "openai/gpt-4o-mini";

/// Calls the GitHub Models chat-completions API to fill a single gap.
#[derive(Debug, Clone)]
pub struct CopilotProvider {
    token: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl CopilotProvider {
    /// Construct with an explicit token and model.
    pub fn new(token: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Build from env: `GITHUB_MODELS_TOKEN` (falls back to `GITHUB_TOKEN`),
    /// `GITHUB_MODELS_MODEL` (default `openai/gpt-4o-mini`),
    /// `GITHUB_MODELS_BASE_URL`.
    pub fn from_env() -> Result<Self, LlmError> {
        let token = std::env::var("GITHUB_MODELS_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .map_err(|_| LlmError::Transport("GITHUB_MODELS_TOKEN/GITHUB_TOKEN not set".into()))?;
        let model =
            std::env::var("GITHUB_MODELS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let base_url = std::env::var("GITHUB_MODELS_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            token,
            model,
            base_url,
            client: reqwest::Client::new(),
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl LlmProvider for CopilotProvider {
    fn name(&self) -> &str {
        "copilot"
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = ChatRequest {
            model: &self.model,
            messages: vec![ChatMessage {
                role: "user",
                content: &prompt,
            }],
        };

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(LlmError::Transport(format!(
                "github-models {status}: {text}"
            )));
        }

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        let answer = parsed.text();
        if answer.is_empty() {
            return Err(LlmError::Parse(format!("no message content: {text}")));
        }
        parse_gap_fill(&answer)
    }
}

// --- OpenAI-compatible chat-completions wire types (only the fields we use) ---

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

impl ChatResponse {
    /// The first choice's message content.
    fn text(&self) -> String {
        self.choices
            .first()
            .and_then(|c| c.message.content.clone())
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    message: ChoiceMessage,
}

#[derive(Deserialize, Default)]
struct ChoiceMessage {
    #[serde(default)]
    content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_is_not_local() {
        let p = CopilotProvider::new("ghp-test", DEFAULT_MODEL);
        assert_eq!(p.name(), "copilot");
        assert!(!p.is_local(), "frontier provider must report non-local");
        assert_eq!(p.model, "openai/gpt-4o-mini");
        assert!(p.endpoint().ends_with("/chat/completions"));
    }

    #[test]
    fn extracts_first_choice_message() {
        let envelope = serde_json::json!({
            "choices": [
                { "message": { "role": "assistant", "content": "{\"proposed_yaml\":\"x\"}" } }
            ]
        })
        .to_string();
        let parsed: ChatResponse = serde_json::from_str(&envelope).unwrap();
        assert_eq!(parsed.text(), "{\"proposed_yaml\":\"x\"}");
    }

    #[test]
    fn request_body_carries_the_grounded_prompt() {
        use bifrost_core::{Gap, GapKind};
        let req = GapFillRequest {
            gap: Gap {
                kind: GapKind::UnsupportedStep,
                construct: "DownloadSecureFile@1".into(),
                detail: "no equivalent".into(),
            },
            source_snippet: "- task: DownloadSecureFile@1".into(),
            converted_yaml: "steps: []".into(),
            importer_message: "secure file download has no equivalent".into(),
            repo_context: "languages: dotnet".into(),
        };
        let prompt = build_gap_fill_prompt(&req);
        let body = ChatRequest {
            model: "openai/gpt-4o-mini",
            messages: vec![ChatMessage {
                role: "user",
                content: &prompt,
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["messages"][0]["role"], "user");
        assert!(json["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("DownloadSecureFile@1"));
    }

    /// Live smoke test — needs `GITHUB_MODELS_TOKEN`/`GITHUB_TOKEN` with models
    /// access. Ignored by default. Run with:
    ///   GITHUB_TOKEN=… cargo test -p bifrost-llm -- --ignored live_copilot
    #[tokio::test]
    #[ignore = "requires a GitHub token with models access and network"]
    async fn live_copilot_fills_a_real_gap() {
        use bifrost_core::{Gap, GapKind};
        let provider = CopilotProvider::from_env().expect("token set");
        let req = GapFillRequest {
            gap: Gap {
                kind: GapKind::UnsupportedStep,
                construct: "DownloadSecureFile@1".into(),
                detail: "no GitHub Actions equivalent".into(),
            },
            source_snippet: "- task: DownloadSecureFile@1".into(),
            converted_yaml: "steps:\n  - uses: actions/checkout@v4".into(),
            importer_message: "DownloadSecureFile@1 has no GitHub Actions equivalent".into(),
            repo_context: "languages: dotnet".into(),
        };
        let resp = provider.fill_gap(&req).await.expect("live gap fill");
        assert!(!resp.proposed_yaml.is_empty());
        assert!((0.0..=1.0).contains(&resp.confidence));
    }
}
