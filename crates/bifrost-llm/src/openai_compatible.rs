//! Generic OpenAI-compatible [`LlmProvider`] (#155).
//!
//! One adapter for **any** server that speaks the OpenAI chat-completions API:
//! a local Gemma-12B on an Azure VM, vLLM, LM Studio, Ollama's `/v1` endpoint,
//! Antigravity, or any hosted OpenAI-compatible gateway. Configure it with a base
//! URL (including the `/v1` path), a model id, an optional bearer key (many local
//! servers need none), and an `is_local` flag.
//!
//! `is_local` is the air-gap contract: set it `true` only for an endpoint that
//! runs on infrastructure you control, so the [`Router`](crate::Router) may use it
//! in air-gap mode. A hosted gateway must be `false`. Grounded by
//! [`build_gap_fill_prompt`](crate::build_gap_fill_prompt); the shared
//! [`parse_gap_fill`](crate::parse_gap_fill) enforces the response contract.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

/// Calls any OpenAI-compatible chat-completions endpoint to fill a single gap.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    /// Base URL including the API version path, e.g. `http://host:8000/v1`.
    base_url: String,
    model: String,
    /// Optional bearer key (omitted for keyless local servers).
    key: Option<String>,
    is_local: bool,
    client: reqwest::Client,
}

impl OpenAiCompatibleProvider {
    /// Construct explicitly. `key` is `None` for keyless local servers.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        key: Option<String>,
        is_local: bool,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            key,
            is_local,
            client: reqwest::Client::new(),
        }
    }

    /// Build from env:
    /// - `BIFROST_OPENAI_BASE_URL` (required, include the `/v1` path)
    /// - `BIFROST_OPENAI_MODEL` (required)
    /// - `BIFROST_OPENAI_KEY` (optional bearer)
    /// - `BIFROST_OPENAI_LOCAL` = `1`|`true` marks the endpoint air-gap-eligible
    pub fn from_env() -> Result<Self, LlmError> {
        let base_url = std::env::var("BIFROST_OPENAI_BASE_URL")
            .map_err(|_| LlmError::Transport("BIFROST_OPENAI_BASE_URL not set".into()))?;
        let model = std::env::var("BIFROST_OPENAI_MODEL")
            .map_err(|_| LlmError::Transport("BIFROST_OPENAI_MODEL not set".into()))?;
        let key = std::env::var("BIFROST_OPENAI_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let is_local = std::env::var("BIFROST_OPENAI_LOCAL")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        Ok(Self::new(base_url, model, key, is_local))
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    fn is_local(&self) -> bool {
        self.is_local
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

        let text = crate::http_text_with_retry("openai-compatible", || {
            let mut request = self.client.post(self.endpoint()).json(&body);
            if let Some(key) = &self.key {
                request = request.bearer_auth(key);
            }
            request
        })
        .await?;

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
    fn local_endpoint_is_air_gap_eligible() {
        let p = OpenAiCompatibleProvider::new(
            "http://gemma.vm.internal:8000/v1/",
            "gemma-2-12b",
            None,
            true,
        );
        assert_eq!(p.name(), "openai-compatible");
        assert!(p.is_local(), "a local endpoint must be air-gap eligible");
        // Trailing slash trimmed; endpoint well-formed.
        assert_eq!(
            p.endpoint(),
            "http://gemma.vm.internal:8000/v1/chat/completions"
        );
    }

    #[test]
    fn hosted_endpoint_is_not_local() {
        let p = OpenAiCompatibleProvider::new(
            "https://api.antigravity.example/v1",
            "ag-large",
            Some("k".into()),
            false,
        );
        assert!(!p.is_local());
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
            model: "gemma-2-12b",
            messages: vec![ChatMessage {
                role: "user",
                content: &prompt,
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "gemma-2-12b");
        assert!(json["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("DownloadSecureFile@1"));
    }

    /// Live smoke test against a local server (e.g. Ollama `/v1`, vLLM). Ignored
    /// by default. Set `BIFROST_OPENAI_BASE_URL`/`_MODEL` and run with `--ignored`.
    #[tokio::test]
    #[ignore = "requires a running OpenAI-compatible server + BIFROST_OPENAI_* env"]
    async fn live_fills_a_real_gap() {
        use bifrost_core::{Gap, GapKind};
        let provider = OpenAiCompatibleProvider::from_env().expect("BIFROST_OPENAI_* set");
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
    }
}
