//! Portal authentication — Entra ID OIDC SSO (#65).
//!
//! The portal signs the user in against Entra ID (Microsoft Entra / Azure AD)
//! using the standard OIDC authorization-code + PKCE flow in the browser (MSAL),
//! and sends the resulting **bearer token** to this API. Here we *validate* that
//! token: signature against Entra's published JWKS, plus issuer / audience /
//! expiry, then map its role claims to a Bifrost [`Identity`].
//!
//! Authentication is **opt-in**. With `BIFROST_AUTH` unset the API runs open and
//! every request is the local admin (logged loudly at startup) — so the system
//! works out of the box and existing tests are unaffected. With
//! `BIFROST_AUTH=entra` a valid bearer token is required on `/api/*` (except
//! `/api/health`); anything else gets 401. Role-level enforcement and tenant
//! scoping build on this in #66.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bifrost_core::{Identity, Role};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde_json::Value;
use tokio::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing configuration: {0}")]
    Config(String),
    #[error("invalid token: {0}")]
    Token(String),
    #[error("jwks error: {0}")]
    Jwks(String),
}

/// Validates a bearer token and resolves the acting [`Identity`].
#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, bearer: &str) -> Result<Identity, AuthError>;
}

/// Offline authenticator: returns a fixed identity for any token. Used in tests
/// and as the stand-in when auth is disabled.
#[derive(Debug, Clone)]
pub struct MockAuthenticator {
    pub identity: Identity,
}

impl Default for MockAuthenticator {
    fn default() -> Self {
        Self {
            identity: Identity::local_admin(),
        }
    }
}

#[async_trait]
impl Authenticator for MockAuthenticator {
    async fn authenticate(&self, _bearer: &str) -> Result<Identity, AuthError> {
        Ok(self.identity.clone())
    }
}

/// Map validated Entra v2 token claims to a Bifrost [`Identity`]. Pure — the
/// signature/issuer/audience checks happen before this is called.
pub fn identity_from_claims(c: &Value) -> Identity {
    let subject = c["oid"]
        .as_str()
        .or_else(|| c["sub"].as_str())
        .unwrap_or_default()
        .to_string();
    let non_empty = |v: &Value, k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    let name = non_empty(c, "name");
    let email = non_empty(c, "preferred_username")
        .or_else(|| non_empty(c, "email"))
        .or_else(|| non_empty(c, "upn"));
    let roles = c["roles"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter_map(Role::from_claim)
                .collect()
        })
        .unwrap_or_default();
    let tenant = c["tid"].as_str().unwrap_or("default").to_string();
    Identity {
        subject,
        name,
        email,
        tenant,
        roles,
    }
}

/// Validate a JWT's signature + standard claims against `key`, returning its
/// claims. Shared by the live authenticator and tests.
pub fn validate_token(
    token: &str,
    key: &DecodingKey,
    audience: &str,
    issuer: &str,
) -> Result<Value, AuthError> {
    let mut v = Validation::new(Algorithm::RS256);
    v.set_audience(&[audience]);
    v.set_issuer(&[issuer]);
    decode::<Value>(token, key, &v)
        .map(|d| d.claims)
        .map_err(|e| AuthError::Token(e.to_string()))
}

/// Cached JWKS + when it was fetched (refresh hourly — Entra rotates keys).
struct CachedJwks {
    keys: JwkSet,
    fetched_at: u64,
}

/// Entra ID OIDC authenticator: validates bearer tokens against the tenant's
/// published JWKS.
pub struct EntraAuthenticator {
    audience: String,
    issuer: String,
    jwks_uri: String,
    client: reqwest::Client,
    jwks: Mutex<Option<CachedJwks>>,
}

impl EntraAuthenticator {
    /// Build from `BIFROST_ENTRA_TENANT_ID` and `BIFROST_ENTRA_AUDIENCE`
    /// (the API's app/client id). Issuer + JWKS URI are derived from the tenant.
    pub fn from_env() -> Result<Self, AuthError> {
        let tenant = std::env::var("BIFROST_ENTRA_TENANT_ID")
            .map_err(|_| AuthError::Config("BIFROST_ENTRA_TENANT_ID not set".into()))?;
        let audience = std::env::var("BIFROST_ENTRA_AUDIENCE")
            .map_err(|_| AuthError::Config("BIFROST_ENTRA_AUDIENCE not set".into()))?;
        // Allow an explicit override (sovereign clouds, CIAM); else standard v2.
        let issuer = std::env::var("BIFROST_ENTRA_ISSUER")
            .unwrap_or_else(|_| format!("https://login.microsoftonline.com/{tenant}/v2.0"));
        let jwks_uri = std::env::var("BIFROST_ENTRA_JWKS_URI").unwrap_or_else(|_| {
            format!("https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys")
        });
        Ok(Self {
            audience,
            issuer,
            jwks_uri,
            client: reqwest::Client::new(),
            jwks: Mutex::new(None),
        })
    }

    async fn decoding_key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        let now = unix_now();
        let mut guard = self.jwks.lock().await;
        let stale = guard
            .as_ref()
            .map(|c| now.saturating_sub(c.fetched_at) > 3600)
            .unwrap_or(true);
        if stale {
            let keys: JwkSet = self
                .client
                .get(&self.jwks_uri)
                .header("User-Agent", "bifrost")
                .send()
                .await
                .map_err(|e| AuthError::Jwks(e.to_string()))?
                .json()
                .await
                .map_err(|e| AuthError::Jwks(e.to_string()))?;
            *guard = Some(CachedJwks {
                keys,
                fetched_at: now,
            });
        }
        let jwks = guard.as_ref().expect("jwks populated");
        let jwk = jwks
            .keys
            .find(kid)
            .ok_or_else(|| AuthError::Jwks(format!("no key for kid {kid}")))?;
        DecodingKey::from_jwk(jwk).map_err(|e| AuthError::Jwks(e.to_string()))
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[async_trait]
impl Authenticator for EntraAuthenticator {
    async fn authenticate(&self, bearer: &str) -> Result<Identity, AuthError> {
        let header = decode_header(bearer).map_err(|e| AuthError::Token(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| AuthError::Token("token has no kid".into()))?;
        let key = self.decoding_key_for(&kid).await?;
        let claims = validate_token(bearer, &key, &self.audience, &self.issuer)?;
        Ok(identity_from_claims(&claims))
    }
}

/// Select the authenticator + whether auth is enforced, from `BIFROST_AUTH`.
/// `entra` → [`EntraAuthenticator`] (enforced). Anything else → open mode: a
/// mock that yields the local admin, and a startup warning.
pub fn select_authenticator() -> (Arc<dyn Authenticator>, bool) {
    match std::env::var("BIFROST_AUTH").as_deref() {
        Ok("entra") => match EntraAuthenticator::from_env() {
            Ok(a) => (Arc::new(a), true),
            Err(e) => {
                tracing::error!("BIFROST_AUTH=entra but Entra unavailable: {e}; running OPEN");
                (Arc::new(MockAuthenticator::default()), false)
            }
        },
        _ => {
            tracing::warn!(
                "authentication disabled (set BIFROST_AUTH=entra to enable) — \
                 all requests run as the local admin"
            );
            (Arc::new(MockAuthenticator::default()), false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const PRIV: &[u8] = include_bytes!("../tests/fixtures/test_oidc_key.pem");
    const PUB: &[u8] = include_bytes!("../tests/fixtures/test_oidc_key.pub.pem");

    const AUD: &str = "api://bifrost";
    const ISS: &str = "https://login.microsoftonline.com/test-tenant/v2.0";

    fn sign(claims: &Value) -> String {
        let key = EncodingKey::from_rsa_pem(PRIV).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-kid".into());
        encode(&header, claims, &key).unwrap()
    }

    fn claims(extra: Value) -> Value {
        let mut base = serde_json::json!({
            "aud": AUD,
            "iss": ISS,
            "exp": unix_now() + 600,
            "oid": "00000000-user",
            "name": "Ada Reviewer",
            "preferred_username": "ada@contoso.com",
            "tid": "test-tenant",
        });
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        base
    }

    #[test]
    fn valid_token_maps_to_identity_with_roles() {
        let token = sign(&claims(serde_json::json!({ "roles": ["Reviewer"] })));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        let c = validate_token(&token, &key, AUD, ISS).expect("valid");
        let id = identity_from_claims(&c);
        assert_eq!(id.subject, "00000000-user");
        assert_eq!(id.email.as_deref(), Some("ada@contoso.com"));
        assert_eq!(id.tenant, "test-tenant");
        assert!(id.has_role(Role::Reviewer));
        assert!(!id.has_role(Role::Admin));
    }

    #[test]
    fn wrong_audience_is_rejected() {
        let token = sign(&claims(serde_json::json!({})));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        assert!(validate_token(&token, &key, "api://other", ISS).is_err());
    }

    #[test]
    fn wrong_issuer_is_rejected() {
        let token = sign(&claims(serde_json::json!({})));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        assert!(validate_token(&token, &key, AUD, "https://evil/").is_err());
    }

    #[test]
    fn no_roles_claim_yields_viewer_only() {
        let token = sign(&claims(serde_json::json!({})));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        let id = identity_from_claims(&validate_token(&token, &key, AUD, ISS).unwrap());
        assert_eq!(id.top_role(), Role::Viewer);
    }

    #[tokio::test]
    async fn mock_authenticator_yields_local_admin() {
        let id = MockAuthenticator::default()
            .authenticate("anything")
            .await
            .unwrap();
        assert!(id.has_role(Role::Admin));
    }
}
