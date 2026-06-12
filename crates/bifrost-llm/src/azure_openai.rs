//! Azure OpenAI Service [`LlmProvider`].
//!
//! Enterprises standardised on Azure run their models through **Azure OpenAI**,
//! which speaks the OpenAI chat-completions schema but with Azure's own auth and
//! URL shape: a per-resource endpoint, a **deployment** name in the path, an
//! `api-version` query parameter, and either an `api-key` header or an Entra ID
//! (AAD) bearer token. This provider wraps that so an Azure-hosted GPT deployment
//! plugs into the [`Router`](crate::Router) like any other backend.
//!
//! Air-gap: set [`AzureOpenAiProvider::is_local`] (env `AZURE_OPENAI_PRIVATE=1`)
//! when the resource is reached over a **private endpoint** inside the customer's
//! own network, so the Router may use it in air-gap mode; a public endpoint must
//! stay non-local.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    build_gap_fill_prompt, parse_gap_fill, GapFillRequest, GapFillResponse, LlmError, LlmProvider,
};

/// A sensible recent GA API version; override per deployment if needed.
const DEFAULT_API_VERSION: &str = "2024-10-21";

/// Calls an Azure OpenAI deployment's chat-completions endpoint to fill one gap.
#[derive(Debug, Clone)]
pub struct AzureOpenAiProvider {
    /// Resource endpoint, e.g. `https://my-resource.openai.azure.com`.
    endpoint: String,
    /// Deployment name (Azure routes to the model via the deployment, not `model`).
    deployment: String,
    api_version: String,
    /// `api-key` header value (Azure's key auth).
    api_key: Option<String>,
    /// Entra ID (AAD) bearer token, supplied externally (workload identity / CLI).
    token: Option<String>,
    /// True only for a private-endpoint resource inside the customer's network.
    is_local: bool,
    client: reqwest::Client,
}

impl AzureOpenAiProvider {
    /// Construct explicitly. Provide an `api_key` (key auth) or a `token` (Entra).
    pub fn new(
        endpoint: impl Into<String>,
        deployment: impl Into<String>,
        api_version: Option<String>,
        api_key: Option<String>,
        token: Option<String>,
        is_local: bool,
    ) -> Self {
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            deployment: deployment.into(),
            api_version: api_version.unwrap_or_else(|| DEFAULT_API_VERSION.to_string()),
            api_key,
            token,
            is_local,
            client: reqwest::Client::new(),
        }
    }

    /// Build from env:
    /// - `AZURE_OPENAI_ENDPOINT` (required, e.g. `https://res.openai.azure.com`)
    /// - `AZURE_OPENAI_DEPLOYMENT` (required, the deployment name)
    /// - `AZURE_OPENAI_API_KEY` and/or `AZURE_OPENAI_TOKEN` (one is required)
    /// - `AZURE_OPENAI_API_VERSION` (optional; defaults to a recent GA)
    /// - `AZURE_OPENAI_PRIVATE` = `1`|`true` marks a private endpoint air-gap-eligible
    pub fn from_env() -> Result<Self, LlmError> {
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT")
            .map_err(|_| LlmError::Transport("AZURE_OPENAI_ENDPOINT not set".into()))?;
        let deployment = std::env::var("AZURE_OPENAI_DEPLOYMENT")
            .map_err(|_| LlmError::Transport("AZURE_OPENAI_DEPLOYMENT not set".into()))?;
        let api_key = std::env::var("AZURE_OPENAI_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let token = std::env::var("AZURE_OPENAI_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        if api_key.is_none() && token.is_none() {
            return Err(LlmError::Transport(
                "Azure OpenAI needs AZURE_OPENAI_API_KEY or AZURE_OPENAI_TOKEN".into(),
            ));
        }
        let api_version = std::env::var("AZURE_OPENAI_API_VERSION")
            .ok()
            .filter(|s| !s.is_empty());
        let is_local = std::env::var("AZURE_OPENAI_PRIVATE")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        Ok(Self::new(
            endpoint,
            deployment,
            api_version,
            api_key,
            token,
            is_local,
        ))
    }

    fn endpoint_url(&self) -> String {
        format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, self.deployment, self.api_version
        )
    }
}

#[async_trait]
impl LlmProvider for AzureOpenAiProvider {
    fn name(&self) -> &str {
        "azure-openai"
    }

    fn is_local(&self) -> bool {
        self.is_local
    }

    async fn fill_gap(&self, req: &GapFillRequest) -> Result<GapFillResponse, LlmError> {
        let prompt = build_gap_fill_prompt(req);
        let body = ChatRequest {
            // Azure ignores `model` (the deployment in the URL selects it), but the
            // schema accepts it and some gateways echo it.
            model: &self.deployment,
            messages: vec![ChatMessage {
                role: "user",
                content: &prompt,
            }],
        };

        let text = crate::http_text_with_retry("azure-openai", || {
            let mut request = self.client.post(self.endpoint_url()).json(&body);
            if let Some(key) = &self.api_key {
                request = request.header("api-key", key);
            } else if let Some(token) = &self.token {
                request = request.bearer_auth(token);
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

// --- OpenAI chat-completions wire types (only the fields we use) ---

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
    fn builds_the_azure_deployment_url_with_api_version() {
        let p = AzureOpenAiProvider::new(
            "https://acme.openai.azure.com/",
            "gpt-4o",
            Some("2024-10-21".into()),
            Some("key".into()),
            None,
            false,
        );
        assert_eq!(p.name(), "azure-openai");
        assert!(!p.is_local(), "a public endpoint is not air-gap eligible");
        assert_eq!(
            p.endpoint_url(),
            "https://acme.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn private_endpoint_is_air_gap_eligible_and_defaults_api_version() {
        let p = AzureOpenAiProvider::new(
            "https://private.openai.azure.com",
            "gpt-4o-mini",
            None,
            None,
            Some("aad-token".into()),
            true,
        );
        assert!(p.is_local(), "a private endpoint may run in air-gap mode");
        assert!(p.endpoint_url().contains("api-version=2024-10-21"));
    }
}
