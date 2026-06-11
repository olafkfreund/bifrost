//! Google Gemini [`LlmProvider`].
//!
//! A frontier provider (`is_local` is `false`), so the [`Router`](crate::Router)
//! blocks it in air-gap mode. Calls the Generative Language API's
//! `generateContent` over raw HTTP (`reqwest` + rustls); the request is grounded
//! by [`build_gap_fill_prompt`](crate::build_gap_fill_prompt) and the response
//! carries no risk score.
//!
//! The API key goes in the `x-goog-api-key` header — never the URL — so secrets
//! never land in query strings or logs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

/// Base for the model endpoints; the model + `:generateContent` are appended.
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
/// Default model — overridable via `GEMINI_MODEL`.
const DEFAULT_MODEL: &str = "gemini-2.5-flash";

/// Calls the Gemini `generateContent` API to fill a single gap.
#[derive(Debug, Clone)]
pub struct GeminiProvider {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl GeminiProvider {
    /// Construct with an explicit key and model.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Build from env: `GEMINI_API_KEY` (required), `GEMINI_MODEL`
    /// (default `gemini-2.0-flash`), `GEMINI_BASE_URL` (default the public API).
    pub fn from_env() -> Result<Self, LlmError> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .map_err(|_| LlmError::Transport("GEMINI_API_KEY not set".into()))?;
        let model = std::env::var("GEMINI_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let base_url =
            std::env::var("GEMINI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            api_key,
            model,
            base_url,
            client: reqwest::Client::new(),
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/{}:generateContent", self.base_url, self.model)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn name(&self) -> &str {
        "gemini"
    }

    fn is_local(&self) -> bool {
        false
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = GenerateRequest {
            contents: vec![Content {
                parts: vec![Part { text: &prompt }],
            }],
        };

        let resp = self
            .client
            .post(self.endpoint())
            .header("x-goog-api-key", &self.api_key)
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
            return Err(LlmError::Transport(format!("gemini {status}: {text}")));
        }

        let parsed: GenerateResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        let answer = parsed.text();
        if answer.is_empty() {
            return Err(LlmError::Parse(format!("no text in response: {text}")));
        }
        parse_gap_fill(&answer)
    }
}

// --- Generative Language API wire types (only the fields we use) ---

#[derive(Serialize)]
struct GenerateRequest<'a> {
    contents: Vec<Content<'a>>,
}

#[derive(Serialize)]
struct Content<'a> {
    parts: Vec<Part<'a>>,
}

#[derive(Serialize)]
struct Part<'a> {
    text: &'a str,
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

impl GenerateResponse {
    /// Concatenate the text parts of the first candidate.
    fn text(&self) -> String {
        self.candidates
            .first()
            .map(|c| {
                c.content
                    .parts
                    .iter()
                    .filter_map(|p| p.text.as_deref())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct Candidate {
    #[serde(default)]
    content: RespContent,
}

#[derive(Deserialize, Default)]
struct RespContent {
    #[serde(default)]
    parts: Vec<RespPart>,
}

#[derive(Deserialize)]
struct RespPart {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_flash_and_is_not_local() {
        let p = GeminiProvider::new("key-test", DEFAULT_MODEL);
        assert_eq!(p.name(), "gemini");
        assert!(!p.is_local(), "frontier provider must report non-local");
        assert_eq!(p.model, "gemini-2.5-flash");
        assert!(p.endpoint().ends_with("/gemini-2.5-flash:generateContent"));
    }

    #[test]
    fn extracts_text_from_first_candidate_parts() {
        let envelope = serde_json::json!({
            "candidates": [
                { "content": { "parts": [{ "text": "{\"proposed_yaml\":" }, { "text": "\"x\"}" }] } }
            ]
        })
        .to_string();
        let parsed: GenerateResponse = serde_json::from_str(&envelope).unwrap();
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
        let body = GenerateRequest {
            contents: vec![Content {
                parts: vec![Part { text: &prompt }],
            }],
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json["contents"][0]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("DownloadSecureFile@1"));
    }

    /// Live smoke test — needs `GEMINI_API_KEY`. Ignored by default. Run with:
    ///   GEMINI_API_KEY=… cargo test -p bifrost-llm -- --ignored live_gemini
    #[tokio::test]
    #[ignore = "requires GEMINI_API_KEY and network"]
    async fn live_gemini_fills_a_real_gap() {
        use bifrost_core::{Gap, GapKind};
        let provider = GeminiProvider::from_env().expect("GEMINI_API_KEY set");
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
