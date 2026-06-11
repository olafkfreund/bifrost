//! Azure DevOps authentication for the REST client (#20).
//!
//! The ADO REST API accepts two credentials, both as standard HTTP auth on each
//! request, behind the [`AdoAuth`] trait:
//!
//! - [`PatAuth`] — a Personal Access Token via HTTP basic (the simple path).
//! - [`EntraAuth`] — a **Microsoft Entra** access token via the client-credentials
//!   flow (a service principal), the least-privilege production path: a token
//!   scoped to the Azure DevOps resource is minted and cached until shortly before
//!   it expires, then sent as a bearer.
//!
//! Both are opt-in: nothing here runs unless the relevant env vars are set, and
//! the Entra path is preferred over a PAT only when `AZURE_*` is configured.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::source::AdapterError;

/// The fixed Azure DevOps resource id; the Entra scope is `<id>/.default`.
const ADO_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// Applies a credential to an outgoing ADO request.
#[async_trait]
pub trait AdoAuth: Send + Sync {
    async fn apply(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, AdapterError>;
}

/// PAT via HTTP basic (`:` + token), the historical default.
#[derive(Debug, Clone)]
pub struct PatAuth {
    pat: String,
}

impl PatAuth {
    pub fn new(pat: impl Into<String>) -> Self {
        Self { pat: pat.into() }
    }
}

#[async_trait]
impl AdoAuth for PatAuth {
    async fn apply(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, AdapterError> {
        Ok(req.basic_auth("", Some(&self.pat)))
    }
}

/// A cached bearer token + the unix time it should be refreshed at.
#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    refresh_at: u64,
}

/// Entra (Azure AD) client-credentials auth: mints an ADO-scoped access token for
/// a service principal and caches it until shortly before it expires.
pub struct EntraAuth {
    tenant_id: String,
    client_id: String,
    client_secret: String,
    login_base: String,
    client: reqwest::Client,
    cache: Mutex<Option<CachedToken>>,
}

impl EntraAuth {
    pub fn new(
        tenant_id: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            login_base: "https://login.microsoftonline.com".to_string(),
            client: reqwest::Client::new(),
            cache: Mutex::new(None),
        }
    }

    /// Build from `AZURE_TENANT_ID` / `AZURE_CLIENT_ID` / `AZURE_CLIENT_SECRET`.
    /// Returns `Ok(None)` when `AZURE_TENANT_ID` is unset (Entra is simply not
    /// configured), so callers can fall back to a PAT without it being an error.
    pub fn from_env() -> Result<Option<Self>, AdapterError> {
        let Ok(tenant_id) = std::env::var("AZURE_TENANT_ID") else {
            return Ok(None);
        };
        let client_id = std::env::var("AZURE_CLIENT_ID")
            .map_err(|_| AdapterError::Auth("AZURE_CLIENT_ID not set".into()))?;
        let client_secret = std::env::var("AZURE_CLIENT_SECRET")
            .map_err(|_| AdapterError::Auth("AZURE_CLIENT_SECRET not set".into()))?;
        let mut auth = Self::new(tenant_id, client_id, client_secret);
        if let Ok(base) = std::env::var("AZURE_LOGIN_BASE") {
            auth.login_base = base;
        }
        Ok(Some(auth))
    }

    fn token_url(&self) -> String {
        format!("{}/{}/oauth2/v2.0/token", self.login_base, self.tenant_id)
    }

    async fn fetch_token(&self, now: u64) -> Result<CachedToken, AdapterError> {
        let resp = self
            .client
            .post(self.token_url())
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.client_id.as_str()),
                ("client_secret", self.client_secret.as_str()),
                ("scope", &format!("{ADO_RESOURCE}/.default")),
            ])
            .send()
            .await
            .map_err(|e| AdapterError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AdapterError::Auth(format!("entra token {status}: {body}")));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AdapterError::Transport(e.to_string()))?;
        let token = body["access_token"]
            .as_str()
            .ok_or_else(|| AdapterError::Auth("no access_token in response".into()))?
            .to_string();
        // Tokens last ~1h; refresh 5 min early (or sooner if the response says so).
        let ttl = body["expires_in"].as_u64().unwrap_or(3600);
        Ok(CachedToken {
            token,
            refresh_at: now + ttl.saturating_sub(300),
        })
    }

    async fn token(&self) -> Result<String, AdapterError> {
        let now = unix_now();
        let mut cache = self.cache.lock().await;
        if let Some(c) = cache.as_ref() {
            if now < c.refresh_at {
                return Ok(c.token.clone());
            }
        }
        let fresh = self.fetch_token(now).await?;
        let token = fresh.token.clone();
        *cache = Some(fresh);
        Ok(token)
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl AdoAuth for EntraAuth {
    async fn apply(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, AdapterError> {
        Ok(req.bearer_auth(self.token().await?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pat_auth_applies_basic() {
        // A request built with PatAuth carries a Basic Authorization header.
        let client = reqwest::Client::new();
        let req = PatAuth::new("ghp-pat")
            .apply(client.get("https://dev.azure.com/contoso/_apis/projects"))
            .await
            .unwrap();
        let built = req.build().unwrap();
        let auth = built
            .headers()
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(auth.starts_with("Basic "));
    }

    #[test]
    fn entra_token_url_is_the_v2_endpoint() {
        let auth = EntraAuth::new("tenant", "client", "secret");
        assert_eq!(
            auth.token_url(),
            "https://login.microsoftonline.com/tenant/oauth2/v2.0/token"
        );
    }

    /// Live: mint a real ADO-scoped token. Ignored by default (needs a real
    /// service principal in AZURE_TENANT_ID / AZURE_CLIENT_ID / AZURE_CLIENT_SECRET).
    #[tokio::test]
    #[ignore = "calls Entra — needs a real service principal"]
    async fn live_mints_an_ado_token() {
        let auth = EntraAuth::from_env()
            .expect("env ok")
            .expect("AZURE_TENANT_ID set");
        assert!(!auth.token().await.expect("mint").is_empty());
    }
}
