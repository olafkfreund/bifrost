//! GCP Vertex AI [`LlmProvider`] (Gemini on Vertex).
//!
//! Enterprises standardised on Google Cloud run Gemini through **Vertex AI**, which
//! uses the same `generateContent` schema as the AI-Studio Gemini API but with a
//! Vertex endpoint (`{region}-aiplatform.googleapis.com`, a project + location in
//! the path) and **OAuth bearer** auth instead of an API key. The bearer token is
//! supplied externally — from workload identity, a metadata server, or `gcloud
//! auth print-access-token` — so this provider stays dependency-light (no Google
//! SDK); the operator refreshes the token.
//!
//! Air-gap: set [`VertexProvider::is_local`] (env `VERTEX_PRIVATE=1`) when Vertex
//! is reached over **Private Service Connect / VPC-SC** inside the customer's
//! network, so the [`Router`](crate::Router) may use it in air-gap mode.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

const DEFAULT_LOCATION: &str = "us-central1";
const DEFAULT_MODEL: &str = "gemini-2.5-flash";

/// Calls a Vertex AI Gemini model's `:generateContent` endpoint to fill one gap.
#[derive(Debug, Clone)]
pub struct VertexProvider {
    project: String,
    location: String,
    model: String,
    /// OAuth2 access token (refreshed externally; workload identity / gcloud).
    token: String,
    is_local: bool,
    client: reqwest::Client,
}

impl VertexProvider {
    pub fn new(
        project: impl Into<String>,
        location: impl Into<String>,
        model: impl Into<String>,
        token: impl Into<String>,
        is_local: bool,
    ) -> Self {
        Self {
            project: project.into(),
            location: location.into(),
            model: model.into(),
            token: token.into(),
            is_local,
            client: reqwest::Client::new(),
        }
    }

    /// Build from env:
    /// - `VERTEX_PROJECT` (required, the GCP project id)
    /// - `VERTEX_TOKEN` (required, an OAuth2 access token)
    /// - `VERTEX_LOCATION` (optional, defaults to `us-central1`)
    /// - `VERTEX_MODEL` (optional, defaults to a recent Gemini)
    /// - `VERTEX_PRIVATE` = `1`|`true` marks a PSC/VPC-SC endpoint air-gap-eligible
    pub fn from_env() -> Result<Self, LlmError> {
        let project = std::env::var("VERTEX_PROJECT")
            .map_err(|_| LlmError::Transport("VERTEX_PROJECT not set".into()))?;
        let token = std::env::var("VERTEX_TOKEN")
            .map_err(|_| LlmError::Transport("VERTEX_TOKEN not set".into()))?;
        let location =
            std::env::var("VERTEX_LOCATION").unwrap_or_else(|_| DEFAULT_LOCATION.to_string());
        let model = std::env::var("VERTEX_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        let is_local = std::env::var("VERTEX_PRIVATE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        Ok(Self::new(project, location, model, token, is_local))
    }

    fn endpoint(&self) -> String {
        format!(
            "https://{loc}-aiplatform.googleapis.com/v1/projects/{proj}/locations/{loc}/publishers/google/models/{model}:generateContent",
            loc = self.location,
            proj = self.project,
            model = self.model,
        )
    }
}

#[async_trait]
impl LlmProvider for VertexProvider {
    fn name(&self) -> &str {
        "vertex"
    }

    fn is_local(&self) -> bool {
        self.is_local
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = GenerateRequest {
            contents: vec![Content {
                parts: vec![Part { text: &prompt }],
            }],
        };

        let text = crate::http_text_with_retry("vertex", || {
            self.client
                .post(self.endpoint())
                .bearer_auth(&self.token)
                .json(&body)
        })
        .await?;

        let parsed: GenerateResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        let answer = parsed.text();
        if answer.is_empty() {
            return Err(LlmError::Parse(format!("no text in response: {text}")));
        }
        parse_gap_fill(&answer)
    }

    async fn chat(&self, prompt: &str) -> Result<String, LlmError> {
        let body = GenerateRequest {
            contents: vec![Content {
                parts: vec![Part { text: prompt }],
            }],
        };
        let text = crate::http_text_with_retry("vertex", || {
            self.client
                .post(self.endpoint())
                .bearer_auth(&self.token)
                .json(&body)
        })
        .await?;
        let parsed: GenerateResponse = serde_json::from_str(&text)
            .map_err(|e| LlmError::Parse(format!("response envelope: {e}: {text}")))?;
        let answer = parsed.text();
        if answer.is_empty() {
            return Err(LlmError::Parse(format!("no text in response: {text}")));
        }
        Ok(answer)
    }
}

// --- Vertex generateContent wire types (shared shape with AI-Studio Gemini) ---

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
    fn builds_the_vertex_endpoint_with_project_and_location() {
        let p = VertexProvider::new(
            "acme-prod",
            "europe-west4",
            "gemini-2.5-pro",
            "ya29.token",
            false,
        );
        assert_eq!(p.name(), "vertex");
        assert!(
            !p.is_local(),
            "a public Vertex endpoint is not air-gap eligible"
        );
        assert_eq!(
            p.endpoint(),
            "https://europe-west4-aiplatform.googleapis.com/v1/projects/acme-prod/locations/europe-west4/publishers/google/models/gemini-2.5-pro:generateContent"
        );
    }

    #[test]
    fn private_endpoint_is_air_gap_eligible() {
        let p = VertexProvider::new("p", DEFAULT_LOCATION, DEFAULT_MODEL, "t", true);
        assert!(
            p.is_local(),
            "a PSC/VPC-SC endpoint may run in air-gap mode"
        );
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
}
