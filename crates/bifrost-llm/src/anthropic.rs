//! Anthropic (Claude) [`LlmProvider`].
//!
//! A frontier provider for hard semantic reasoning and documentation. It is
//! **not local** ([`is_local`](LlmProvider::is_local) is `false`), so the
//! [`Router`](crate::Router) blocks it in air-gap mode — pipeline data never
//! leaves the box when air-gap is on.
//!
//! There is no official Anthropic Rust SDK, so we call the Messages API over
//! raw HTTP (`reqwest` + rustls). Per the project's hard rules the request is
//! grounded (built by [`build_gap_fill_prompt`](crate::build_gap_fill_prompt))
//! and the response carries no risk score.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default model — overridable via `ANTHROPIC_MODEL`. Config-driven per #38.
const DEFAULT_MODEL: &str = "claude-opus-4-8";
const DEFAULT_MAX_TOKENS: u32 = 4096;

/// Calls the Anthropic Messages API to fill a single gap.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// Construct with an explicit key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: MESSAGES_URL.to_string(),
            max_tokens: DEFAULT_MAX_TOKENS,
            client: reqwest::Client::new(),
        }
    }

    /// Build from env: `ANTHROPIC_API_KEY` (required), `ANTHROPIC_MODEL`
    /// (default `claude-opus-4-8`), `ANTHROPIC_BASE_URL` (default the public
    /// API — set this to route through a gateway).
    pub fn from_env() -> Result<Self, LlmError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| LlmError::Transport("ANTHROPIC_API_KEY not set".into()))?;
        let model = std::env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let base_url =
            std::env::var("ANTHROPIC_BASE_URL").unwrap_or_else(|_| MESSAGES_URL.to_string());
        Ok(Self {
            api_key,
            model,
            base_url,
            max_tokens: DEFAULT_MAX_TOKENS,
            client: reqwest::Client::new(),
        })
    }

    /// Override the response token ceiling (default 4096).
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = MessagesRequest {
            model: &self.model,
            max_tokens: self.max_tokens,
            // Adaptive thinking: this is semantic reasoning, so let Claude
            // decide its own depth. Thinking blocks are ignored — we read only
            // the text blocks below.
            thinking: Thinking { kind: "adaptive" },
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
        };

        let resp = self
            .client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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
            return Err(LlmError::Transport(format!("anthropic {status}: {text}")));
        }

        let parsed: MessagesResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        let answer = parsed.text();
        if answer.is_empty() {
            return Err(LlmError::Parse(format!(
                "no text block in response: {text}"
            )));
        }
        parse_gap_fill(&answer)
    }
}

// --- Anthropic Messages API wire types (only the fields we use) ---

#[derive(Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    thinking: Thinking,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: String,
}

#[derive(Deserialize)]
struct MessagesResponse {
    #[serde(default)]
    content: Vec<ContentBlock>,
}

impl MessagesResponse {
    /// Concatenate every `text` block, skipping `thinking`/other block types.
    fn text(&self) -> String {
        self.content
            .iter()
            .filter(|b| b.kind == "text")
            .filter_map(|b| b.text.as_deref())
            .collect::<Vec<_>>()
            .join("")
    }
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_opus_and_is_not_local() {
        let p = AnthropicProvider::new("sk-test", DEFAULT_MODEL);
        assert_eq!(p.name(), "anthropic");
        assert!(!p.is_local(), "frontier provider must report non-local");
        assert_eq!(p.model, "claude-opus-4-8");
    }

    #[test]
    fn extracts_text_blocks_and_skips_thinking() {
        // Mirror a real envelope: a thinking block (no text) then a text block.
        let envelope = serde_json::json!({
            "content": [
                { "type": "thinking", "thinking": "reasoning…" },
                { "type": "text", "text": "{\"hello\":1}" }
            ]
        })
        .to_string();
        let parsed: MessagesResponse = serde_json::from_str(&envelope).unwrap();
        assert_eq!(parsed.text(), "{\"hello\":1}");
    }

    #[test]
    fn request_body_sends_adaptive_thinking_and_grounded_prompt() {
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
        let body = MessagesRequest {
            model: "claude-opus-4-8",
            max_tokens: 4096,
            thinking: Thinking { kind: "adaptive" },
            messages: vec![Message {
                role: "user",
                content: build_gap_fill_prompt(&req),
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"]["type"], "adaptive");
        assert_eq!(json["model"], "claude-opus-4-8");
        assert!(json["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("DownloadSecureFile@1"));
    }

    /// Live smoke test — needs `ANTHROPIC_API_KEY`. Ignored by default so CI
    /// (no key, air-gap-friendly) stays green. Run with:
    ///   ANTHROPIC_API_KEY=… cargo test -p bifrost-llm -- --ignored live_
    #[tokio::test]
    #[ignore = "requires ANTHROPIC_API_KEY and network"]
    async fn live_fills_a_real_gap() {
        use bifrost_core::{Gap, GapKind};
        let provider = AnthropicProvider::from_env().expect("ANTHROPIC_API_KEY set");
        let req = GapFillRequest {
            gap: Gap {
                kind: GapKind::UnsupportedStep,
                construct: "DownloadSecureFile@1".into(),
                detail: "no GitHub Actions equivalent".into(),
            },
            source_snippet: "- task: DownloadSecureFile@1\n  inputs:\n    secureFile: app.keystore"
                .into(),
            converted_yaml: "steps:\n  - uses: actions/checkout@v4".into(),
            importer_message: "DownloadSecureFile@1 has no GitHub Actions equivalent".into(),
            repo_context: "languages: java; build: gradle".into(),
        };
        let resp = provider.fill_gap(&req).await.expect("live gap fill");
        assert!(!resp.proposed_yaml.is_empty(), "proposed YAML present");
        assert!(
            (0.0..=1.0).contains(&resp.confidence),
            "confidence in range"
        );
    }
}
