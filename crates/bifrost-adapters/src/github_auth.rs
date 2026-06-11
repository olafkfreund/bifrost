//! GitHub authentication for the GitHub-calling adapters (#64).
//!
//! Bifrost talks to the GitHub API from three places — the [`crate::Publisher`]
//! (open PRs), the [`crate::SandboxTrigger`] (dispatch workflows), and the
//! [`crate::RunCollector`] (read runs). Each needs a bearer token. This module
//! provides that token two ways, behind the [`GitHubAuth`] trait:
//!
//! - [`StaticTokenAuth`] — a PAT / `GITHUB_TOKEN` (the simple path).
//! - [`GitHubAppAuth`] — a **GitHub App**, the least-privilege production path: a
//!   short-lived RS256 JWT signed with the app's private key is exchanged for an
//!   installation access token scoped to exactly the repos the app is installed
//!   on. The token is cached until shortly before it expires.
//!
//! Both are opt-in: nothing here runs unless the relevant env vars are set, and
//! the App path is preferred over a PAT only when `GITHUB_APP_*` is configured.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing configuration: {0}")]
    Config(String),
    #[error("invalid private key: {0}")]
    Key(String),
    #[error("jwt error: {0}")]
    Jwt(String),
    #[error("github API error: {0}")]
    Api(String),
}

/// Supplies a GitHub bearer token. The token may be long-lived (a PAT) or
/// short-lived and refreshed on demand (a GitHub App installation token).
#[async_trait]
pub trait GitHubAuth: Send + Sync {
    async fn token(&self) -> Result<String, AuthError>;
}

/// A fixed token (PAT / `GITHUB_TOKEN`).
#[derive(Debug, Clone)]
pub struct StaticTokenAuth {
    token: String,
}

impl StaticTokenAuth {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }
}

#[async_trait]
impl GitHubAuth for StaticTokenAuth {
    async fn token(&self) -> Result<String, AuthError> {
        Ok(self.token.clone())
    }
}

/// JWT claims for a GitHub App: issuer is the app id; the token is valid for a
/// short window (GitHub caps app JWTs at 10 minutes).
#[derive(Debug, Serialize)]
struct AppJwtClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// A cached installation token + the unix time it should be refreshed at.
#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    refresh_at: u64,
}

/// GitHub App auth: mints a short-lived JWT, exchanges it for an installation
/// access token, and caches that token until shortly before it expires.
pub struct GitHubAppAuth {
    app_id: String,
    installation_id: String,
    encoding_key: EncodingKey,
    api_base: String,
    client: reqwest::Client,
    cache: Mutex<Option<CachedToken>>,
}

impl GitHubAppAuth {
    /// Build from the app id, the PEM-encoded private key, and the installation id.
    pub fn new(
        app_id: impl Into<String>,
        private_key_pem: &[u8],
        installation_id: impl Into<String>,
    ) -> Result<Self, AuthError> {
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem)
            .map_err(|e| AuthError::Key(e.to_string()))?;
        Ok(Self {
            app_id: app_id.into(),
            installation_id: installation_id.into(),
            encoding_key,
            api_base: "https://api.github.com".to_string(),
            client: reqwest::Client::new(),
            cache: Mutex::new(None),
        })
    }

    /// Build from the environment:
    /// - `GITHUB_APP_ID` (required)
    /// - `GITHUB_APP_INSTALLATION_ID` (required)
    /// - `GITHUB_APP_PRIVATE_KEY` (PEM) or `GITHUB_APP_PRIVATE_KEY_FILE` (path)
    /// - `GITHUB_API_BASE` (optional)
    ///
    /// Returns `Ok(None)` when `GITHUB_APP_ID` is unset (the App path is simply
    /// not configured), so callers can fall back to a PAT without treating it as
    /// an error.
    pub fn from_env() -> Result<Option<Self>, AuthError> {
        let Ok(app_id) = std::env::var("GITHUB_APP_ID") else {
            return Ok(None);
        };
        let installation_id = std::env::var("GITHUB_APP_INSTALLATION_ID")
            .map_err(|_| AuthError::Config("GITHUB_APP_INSTALLATION_ID not set".into()))?;
        let pem = match std::env::var("GITHUB_APP_PRIVATE_KEY") {
            Ok(p) if !p.is_empty() => p.into_bytes(),
            _ => {
                let path = std::env::var("GITHUB_APP_PRIVATE_KEY_FILE").map_err(|_| {
                    AuthError::Config(
                        "set GITHUB_APP_PRIVATE_KEY or GITHUB_APP_PRIVATE_KEY_FILE".into(),
                    )
                })?;
                std::fs::read(&path)
                    .map_err(|e| AuthError::Config(format!("reading {path}: {e}")))?
            }
        };
        let mut auth = Self::new(app_id, &pem, installation_id)?;
        if let Ok(base) = std::env::var("GITHUB_API_BASE") {
            auth.api_base = base;
        }
        Ok(Some(auth))
    }

    /// Sign a fresh app JWT (RS256), valid for `ttl_secs` from `now`. GitHub
    /// rejects JWTs whose `iat` is in the future, so back-date it 60s for clock skew.
    fn app_jwt(&self, now: u64, ttl_secs: u64) -> Result<String, AuthError> {
        let claims = AppJwtClaims {
            iat: now.saturating_sub(60),
            exp: now + ttl_secs,
            iss: self.app_id.clone(),
        };
        encode(&Header::new(Algorithm::RS256), &claims, &self.encoding_key)
            .map_err(|e| AuthError::Jwt(e.to_string()))
    }

    /// Exchange the app JWT for an installation access token.
    async fn fetch_installation_token(&self, now: u64) -> Result<CachedToken, AuthError> {
        let jwt = self.app_jwt(now, 540)?; // 9 min, under GitHub's 10-min cap
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.api_base, self.installation_id
        );
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&jwt)
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "bifrost")
            .send()
            .await
            .map_err(|e| AuthError::Api(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::Api(format!("{status}: {body}")));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AuthError::Api(e.to_string()))?;
        let token = body["token"]
            .as_str()
            .ok_or_else(|| AuthError::Api("no token in response".into()))?
            .to_string();
        // Installation tokens last ~1h; refresh 5 min early.
        Ok(CachedToken {
            token,
            refresh_at: now + 3300,
        })
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl GitHubAuth for GitHubAppAuth {
    async fn token(&self) -> Result<String, AuthError> {
        let now = unix_now();
        let mut cache = self.cache.lock().await;
        if let Some(c) = cache.as_ref() {
            if now < c.refresh_at {
                return Ok(c.token.clone());
            }
        }
        let fresh = self.fetch_installation_token(now).await?;
        let token = fresh.token.clone();
        *cache = Some(fresh);
        Ok(token)
    }
}

/// Resolve a GitHub token from the environment, preferring a **GitHub App**
/// installation token (least privilege) when `GITHUB_APP_*` is configured, then
/// falling back to `GITHUB_TOKEN`. Returns `Ok(None)` when neither is set so the
/// caller can stay on the mock path. Used by the API's live GitHub selectors.
pub async fn github_token_from_env() -> Result<Option<String>, AuthError> {
    if let Some(app) = GitHubAppAuth::from_env()? {
        return app.token().await.map(Some);
    }
    Ok(std::env::var("GITHUB_TOKEN").ok().filter(|t| !t.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    const TEST_KEY: &[u8] = include_bytes!("../tests/fixtures/test_app_key.pem");
    const TEST_PUB_KEY: &[u8] = include_bytes!("../tests/fixtures/test_app_key.pub.pem");

    #[tokio::test]
    async fn static_token_returns_the_token() {
        let auth = StaticTokenAuth::new("ghp_example");
        assert_eq!(auth.token().await.unwrap(), "ghp_example");
    }

    #[test]
    fn app_jwt_is_signed_and_has_correct_claims() {
        let auth = GitHubAppAuth::new("123456", TEST_KEY, "789").expect("valid key");
        let jwt = auth.app_jwt(1_000_000, 540).expect("signs");

        // Verify with the matching public key derived from the same RSA PEM.
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = false;
        let decoded = decode::<serde_json::Value>(
            &jwt,
            &DecodingKey::from_rsa_pem(TEST_PUB_KEY).unwrap(),
            &validation,
        )
        .expect("jwt verifies against the app key");
        assert_eq!(decoded.claims["iss"], "123456");
        // iat is back-dated 60s for skew; exp is iat-window + ttl.
        assert_eq!(decoded.claims["iat"], 999_940);
        assert_eq!(decoded.claims["exp"], 1_000_540);
    }

    #[test]
    fn invalid_private_key_is_rejected() {
        // (GitHubAppAuth holds an EncodingKey/Mutex and isn't Debug, so match the
        // Result rather than unwrap_err.)
        assert!(matches!(
            GitHubAppAuth::new("1", b"not a pem", "2"),
            Err(AuthError::Key(_))
        ));
    }

    /// Live: exchanges a real app JWT for an installation token. Ignored by
    /// default (hits the GitHub API and needs a real app key + installation).
    #[tokio::test]
    #[ignore = "calls the real GitHub App API — needs GITHUB_APP_* configured"]
    async fn live_mints_installation_token() {
        let auth = GitHubAppAuth::from_env()
            .expect("env ok")
            .expect("GITHUB_APP_ID set");
        let token = auth.token().await.expect("mints a token");
        assert!(!token.is_empty());
    }
}
