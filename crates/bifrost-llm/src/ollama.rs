//! Ollama (local) [`LlmProvider`].
//!
//! The air-gap workhorse: a model served by a local Ollama (or llama.cpp with
//! the Ollama-compatible API) on the same network. [`is_local`] is `true`, so
//! the [`Router`](crate::Router) permits it in air-gap mode — a customer can
//! run the entire pipeline with **zero external calls** (project hard rule).
//!
//! Uses Ollama's native chat endpoint (`POST /api/chat`) with `format: "json"`
//! so the model is constrained to emit a JSON object, and `stream: false` so we
//! get one envelope back. Grounded gap-fill only; the response carries no risk
//! score.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

const DEFAULT_BASE_URL: &str = "http://localhost:11434";
/// Default local model — overridable via `OLLAMA_MODEL`. A capable code model
/// (e.g. `qwen2.5-coder`, `llama3.1`) is recommended for usable gap-fill.
const DEFAULT_MODEL: &str = "qwen2.5-coder";

/// Calls a local Ollama server's chat API to fill a single gap.
#[derive(Debug, Clone)]
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    /// Construct with an explicit base URL and model.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Build from env: `OLLAMA_BASE_URL` (default `http://localhost:11434`),
    /// `OLLAMA_MODEL` (default `qwen2.5-coder`). Never fails — a local server
    /// is assumed reachable at request time, not construction time.
    pub fn from_env() -> Self {
        let base_url =
            std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        Self::new(base_url, model)
    }
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn is_local(&self) -> bool {
        true
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = ChatRequest {
            model: &self.model,
            stream: false,
            // Constrain output to a JSON object — Ollama enforces this server-side.
            format: "json",
            messages: vec![ChatMessage {
                role: "user",
                content: prompt,
            }],
        };

        let url = format!("{}/api/chat", self.base_url);
        let text =
            crate::http_text_with_retry("ollama", || self.client.post(&url).json(&body)).await?;

        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        parse_gap_fill(&parsed.message.content)
    }

    async fn chat(&self, prompt: &str) -> Result<String, LlmError> {
        // Freeform (no `format: json`) — the assistant returns prose.
        let body = ChatRequestPlain {
            model: &self.model,
            stream: false,
            messages: vec![ChatMessage {
                role: "user",
                content: prompt.to_string(),
            }],
        };
        let url = format!("{}/api/chat", self.base_url);
        let text =
            crate::http_text_with_retry("ollama", || self.client.post(&url).json(&body)).await?;
        let parsed: ChatResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        Ok(parsed.message.content)
    }
}

// --- Ollama chat API wire types (only the fields we use) ---

/// A chat request without the JSON-format constraint, for freeform replies.
#[derive(Serialize)]
struct ChatRequestPlain<'a> {
    model: &'a str,
    stream: bool,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    stream: bool,
    format: &'a str,
    messages: Vec<ChatMessage<'a>>,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    #[serde(default)]
    content: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_local_and_strips_trailing_slash() {
        let p = OllamaProvider::new("http://localhost:11434/", DEFAULT_MODEL);
        assert_eq!(p.name(), "ollama");
        assert!(p.is_local(), "Ollama must be usable in air-gap mode");
        assert_eq!(
            p.base_url, "http://localhost:11434",
            "trailing slash trimmed"
        );
    }

    #[test]
    fn request_constrains_json_and_disables_streaming() {
        let body = ChatRequest {
            model: "qwen2.5-coder",
            stream: false,
            format: "json",
            messages: vec![ChatMessage {
                role: "user",
                content: "hi".into(),
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["format"], "json");
        assert_eq!(json["stream"], false);
        assert_eq!(json["messages"][0]["role"], "user");
    }

    #[test]
    fn parses_ollama_envelope_into_gap_fill() {
        // Ollama returns the JSON object as a string in message.content.
        let inner = r#"{"proposedYaml":"- run: echo hi","rationale":"ok","riskFlags":[],"verifySteps":["sandbox"],"confidence":0.6}"#;
        let envelope = serde_json::json!({
            "model": "qwen2.5-coder",
            "message": { "role": "assistant", "content": inner },
            "done": true
        })
        .to_string();
        let parsed: ChatResponse = serde_json::from_str(&envelope).unwrap();
        let resp = parse_gap_fill(&parsed.message.content).expect("parses gap fill");
        assert_eq!(resp.proposed_yaml, "- run: echo hi");
        assert_eq!(resp.confidence, 0.6);
    }

    /// Live smoke test — needs a local Ollama serving `OLLAMA_MODEL`. Ignored by
    /// default. Run with:
    ///   OLLAMA_MODEL=qwen2.5-coder cargo test -p bifrost-llm -- --ignored live_
    #[tokio::test]
    #[ignore = "requires a running local Ollama server"]
    async fn live_fills_a_real_gap() {
        use bifrost_core::{Gap, GapKind};
        let provider = OllamaProvider::from_env();
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
        assert!(
            (0.0..=1.0).contains(&resp.confidence),
            "confidence in range"
        );
    }
}
