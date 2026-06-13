//! Portal authentication — pluggable SSO behind the [`Authenticator`] seam
//! (#65, #286).
//!
//! The portal signs the user in against an identity provider and sends the
//! resulting **bearer token** to this API, which *validates* it and maps it to a
//! Bifrost [`Identity`]. Several providers plug into the same seam:
//!
//! - **Entra ID** ([`EntraAuthenticator`], `BIFROST_AUTH=entra`) — validates an
//!   Entra (Azure AD) OIDC bearer against the tenant's published JWKS.
//! - **Generic OIDC** ([`GenericOidcAuthenticator`], `BIFROST_AUTH=oidc`) —
//!   validates a bearer against *any* OIDC issuer (issuer / JWKS URI / audience
//!   given by env). This is what makes **Keycloak** work directly, including
//!   Keycloak brokering to Entra.
//! - **GitHub login** ([`GitHubLoginAuthenticator`], `BIFROST_AUTH=github`) —
//!   GitHub's OAuth web flow issues opaque access tokens (not verifiable OIDC
//!   ID tokens), so the bearer is validated by calling the GitHub user API.
//!
//! All providers feed the same provider-agnostic RBAC + tenancy (#66): the
//! signature/issuer/audience checks (or user-API call) happen first, then the
//! claims/profile are mapped to roles and a tenant.
//!
//! Authentication is **opt-in**. With `BIFROST_AUTH` unset the API runs open and
//! every request is the local admin (logged loudly at startup) — so the system
//! works out of the box and existing tests are unaffected. With a provider set a
//! valid bearer token is required on `/api/*` (except `/api/health`); anything
//! else gets 401. Role-level enforcement and tenant scoping build on this in #66.

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

/// Map validated generic-OIDC claims to a Bifrost [`Identity`]. Unlike Entra,
/// arbitrary issuers (Keycloak, Auth0, Okta, …) vary in claim shape: roles may
/// arrive under `roles` *or* `groups`, and the tenant under a configurable claim
/// (default `tid`, e.g. `org_id` / `realm`). Subject/name/email follow the
/// standard OIDC claims (`sub`, `name`, `preferred_username`/`email`). Pure — the
/// signature/issuer/audience checks happen before this is called.
pub fn identity_from_oidc_claims(c: &Value, tenant_claim: &str) -> Identity {
    let subject = c["sub"]
        .as_str()
        .or_else(|| c["oid"].as_str())
        .unwrap_or_default()
        .to_string();
    let non_empty = |v: &Value, k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    let name = non_empty(c, "name");
    let email = non_empty(c, "email")
        .or_else(|| non_empty(c, "preferred_username"))
        .or_else(|| non_empty(c, "upn"));
    // Roles can live under `roles` or `groups`; collect from both and dedupe.
    let mut roles: Vec<Role> = Vec::new();
    for key in ["roles", "groups"] {
        if let Some(arr) = c[key].as_array() {
            for r in arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter_map(Role::from_claim)
            {
                if !roles.contains(&r) {
                    roles.push(r);
                }
            }
        }
    }
    let tenant = c[tenant_claim].as_str().unwrap_or("default").to_string();
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

/// Cached JWKS + when it was fetched (refresh hourly — IdPs rotate keys).
struct CachedJwks {
    keys: JwkSet,
    fetched_at: u64,
}

/// A JWKS endpoint with an hourly in-memory cache, shared by every OIDC
/// authenticator (Entra, generic OIDC). Fetches on first use and refreshes when
/// the cache is older than an hour, then resolves a signing key by `kid`.
struct JwksCache {
    jwks_uri: String,
    client: reqwest::Client,
    cached: Mutex<Option<CachedJwks>>,
}

impl JwksCache {
    fn new(jwks_uri: String) -> Self {
        Self {
            jwks_uri,
            client: reqwest::Client::new(),
            cached: Mutex::new(None),
        }
    }

    async fn decoding_key_for(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        let now = unix_now();
        let mut guard = self.cached.lock().await;
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

/// Entra ID OIDC authenticator: validates bearer tokens against the tenant's
/// published JWKS.
pub struct EntraAuthenticator {
    audience: String,
    issuer: String,
    jwks: JwksCache,
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
            jwks: JwksCache::new(jwks_uri),
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
impl Authenticator for EntraAuthenticator {
    async fn authenticate(&self, bearer: &str) -> Result<Identity, AuthError> {
        let header = decode_header(bearer).map_err(|e| AuthError::Token(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| AuthError::Token("token has no kid".into()))?;
        let key = self.jwks.decoding_key_for(&kid).await?;
        let claims = validate_token(bearer, &key, &self.audience, &self.issuer)?;
        Ok(identity_from_claims(&claims))
    }
}

/// Generic OIDC authenticator: validates bearer tokens against **any** OIDC
/// issuer whose issuer, JWKS URI and audience are given by env. This is what
/// makes Keycloak work directly (including Keycloak brokering to Entra), as well
/// as Auth0 / Okta / Ping and other standards-compliant providers.
pub struct GenericOidcAuthenticator {
    audience: String,
    issuer: String,
    tenant_claim: String,
    jwks: JwksCache,
}

impl GenericOidcAuthenticator {
    /// Build from `BIFROST_OIDC_ISSUER`, `BIFROST_OIDC_JWKS_URI` and
    /// `BIFROST_OIDC_AUDIENCE` (all required). `BIFROST_OIDC_TENANT_CLAIM`
    /// selects which claim carries the tenant (default `tid`).
    pub fn from_env() -> Result<Self, AuthError> {
        let issuer = std::env::var("BIFROST_OIDC_ISSUER")
            .map_err(|_| AuthError::Config("BIFROST_OIDC_ISSUER not set".into()))?;
        let jwks_uri = std::env::var("BIFROST_OIDC_JWKS_URI")
            .map_err(|_| AuthError::Config("BIFROST_OIDC_JWKS_URI not set".into()))?;
        let audience = std::env::var("BIFROST_OIDC_AUDIENCE")
            .map_err(|_| AuthError::Config("BIFROST_OIDC_AUDIENCE not set".into()))?;
        let tenant_claim =
            std::env::var("BIFROST_OIDC_TENANT_CLAIM").unwrap_or_else(|_| "tid".into());
        Ok(Self {
            audience,
            issuer,
            tenant_claim,
            jwks: JwksCache::new(jwks_uri),
        })
    }
}

#[async_trait]
impl Authenticator for GenericOidcAuthenticator {
    async fn authenticate(&self, bearer: &str) -> Result<Identity, AuthError> {
        let header = decode_header(bearer).map_err(|e| AuthError::Token(e.to_string()))?;
        let kid = header
            .kid
            .ok_or_else(|| AuthError::Token("token has no kid".into()))?;
        let key = self.jwks.decoding_key_for(&kid).await?;
        let claims = validate_token(bearer, &key, &self.audience, &self.issuer)?;
        Ok(identity_from_oidc_claims(&claims, &self.tenant_claim))
    }
}

/// Map a GitHub user-API profile (`GET /user`) to a Bifrost [`Identity`]. Pure —
/// the token is validated by the API call that produced `user` before this runs.
///
/// GitHub does not carry Bifrost roles, so everyone defaults to `Viewer` unless
/// their login is listed in `admin_logins` (case-insensitive), in which case they
/// get `Admin`. The numeric `id` is the stable subject; tenant is always
/// `default` (GitHub login isn't multi-tenant on its own).
pub fn identity_from_github_user(user: &Value, admin_logins: &[String]) -> Identity {
    let login = user["login"].as_str().unwrap_or_default().to_string();
    let subject = match user["id"].as_i64() {
        Some(id) => id.to_string(),
        None => login.clone(),
    };
    let non_empty = |k: &str| user[k].as_str().filter(|s| !s.is_empty()).map(String::from);
    let name = non_empty("name").or_else(|| (!login.is_empty()).then(|| login.clone()));
    let email = non_empty("email");
    let is_admin = admin_logins
        .iter()
        .any(|l| l.eq_ignore_ascii_case(&login) && !login.is_empty());
    let roles = if is_admin {
        vec![Role::Admin]
    } else {
        vec![Role::Viewer]
    };
    Identity {
        subject,
        name,
        email,
        tenant: "default".into(),
        roles,
    }
}

/// GitHub "sign in with GitHub" authenticator. GitHub's OAuth web flow issues
/// **opaque access tokens** (not verifiable OIDC ID tokens), so we validate the
/// bearer by calling the GitHub user API with it; a 200 proves the token is live
/// and identifies the user. The API base is overridable for GitHub Enterprise
/// Server.
pub struct GitHubLoginAuthenticator {
    api_base: String,
    admin_logins: Vec<String>,
    client: reqwest::Client,
}

impl GitHubLoginAuthenticator {
    /// Build from env. `GITHUB_API_BASE` overrides the API base (default
    /// `https://api.github.com`) for GHES. `BIFROST_GITHUB_ADMIN_LOGINS` is a
    /// comma-separated list of GitHub logins granted `Admin`; everyone else is
    /// `Viewer`.
    pub fn from_env() -> Result<Self, AuthError> {
        let api_base = std::env::var("GITHUB_API_BASE")
            .unwrap_or_else(|_| "https://api.github.com".into())
            .trim_end_matches('/')
            .to_string();
        let admin_logins = std::env::var("BIFROST_GITHUB_ADMIN_LOGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Self {
            api_base,
            admin_logins,
            client: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl Authenticator for GitHubLoginAuthenticator {
    async fn authenticate(&self, bearer: &str) -> Result<Identity, AuthError> {
        let resp = self
            .client
            .get(format!("{}/user", self.api_base))
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "bifrost")
            .send()
            .await
            .map_err(|e| AuthError::Token(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AuthError::Token(format!(
                "github user lookup failed: {}",
                resp.status()
            )));
        }
        let user: Value = resp
            .json()
            .await
            .map_err(|e| AuthError::Token(e.to_string()))?;
        Ok(identity_from_github_user(&user, &self.admin_logins))
    }
}

/// Select the authenticator + whether auth is enforced, from `BIFROST_AUTH`.
/// `entra` → [`EntraAuthenticator`]; `oidc` → [`GenericOidcAuthenticator`];
/// `github` → [`GitHubLoginAuthenticator`] (all enforced). Anything else → open
/// mode: a mock that yields the local admin, and a startup warning. A provider
/// that fails to configure logs an error and falls back to open mode.
pub fn select_authenticator() -> (Arc<dyn Authenticator>, bool) {
    match std::env::var("BIFROST_AUTH").as_deref() {
        Ok("entra") => match EntraAuthenticator::from_env() {
            Ok(a) => (Arc::new(a), true),
            Err(e) => {
                tracing::error!("BIFROST_AUTH=entra but Entra unavailable: {e}; running OPEN");
                (Arc::new(MockAuthenticator::default()), false)
            }
        },
        Ok("oidc") => match GenericOidcAuthenticator::from_env() {
            Ok(a) => (Arc::new(a), true),
            Err(e) => {
                tracing::error!("BIFROST_AUTH=oidc but OIDC unavailable: {e}; running OPEN");
                (Arc::new(MockAuthenticator::default()), false)
            }
        },
        Ok("github") => match GitHubLoginAuthenticator::from_env() {
            Ok(a) => (Arc::new(a), true),
            Err(e) => {
                tracing::error!(
                    "BIFROST_AUTH=github but GitHub login unavailable: {e}; running OPEN"
                );
                (Arc::new(MockAuthenticator::default()), false)
            }
        },
        _ => {
            tracing::warn!(
                "authentication disabled (set BIFROST_AUTH=entra|oidc|github to enable) — \
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

    // --- Generic OIDC (#286) -------------------------------------------------

    const OIDC_ISS: &str = "https://keycloak.example.com/realms/bifrost";

    #[test]
    fn generic_oidc_valid_token_maps_roles_from_roles_claim() {
        let token = sign(&claims(serde_json::json!({
            "iss": OIDC_ISS,
            "sub": "kc-user-1",
            "roles": ["reviewer"],
        })));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        let c = validate_token(&token, &key, AUD, OIDC_ISS).expect("valid");
        let id = identity_from_oidc_claims(&c, "tid");
        assert_eq!(id.subject, "kc-user-1");
        assert!(id.has_role(Role::Reviewer));
        assert!(!id.has_role(Role::Admin));
    }

    #[test]
    fn generic_oidc_maps_roles_from_groups_claim() {
        // Keycloak/Auth0 often expose roles under `groups`.
        let c = serde_json::json!({
            "sub": "kc-user-2",
            "groups": ["Owner", "unrelated-group"],
        });
        let id = identity_from_oidc_claims(&c, "tid");
        assert!(id.has_role(Role::Admin));
    }

    #[test]
    fn generic_oidc_configurable_tenant_claim() {
        let c = serde_json::json!({
            "sub": "u",
            "org_id": "acme",
            "roles": ["viewer"],
        });
        let id = identity_from_oidc_claims(&c, "org_id");
        assert_eq!(id.tenant, "acme");
        // The default `tid` claim is absent → falls back to "default".
        let id_default = identity_from_oidc_claims(&c, "tid");
        assert_eq!(id_default.tenant, "default");
    }

    #[test]
    fn generic_oidc_wrong_audience_rejected() {
        let token = sign(&claims(serde_json::json!({ "iss": OIDC_ISS })));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        assert!(validate_token(&token, &key, "api://other", OIDC_ISS).is_err());
    }

    #[test]
    fn generic_oidc_wrong_issuer_rejected() {
        let token = sign(&claims(serde_json::json!({ "iss": OIDC_ISS })));
        let key = DecodingKey::from_rsa_pem(PUB).unwrap();
        assert!(validate_token(&token, &key, AUD, "https://evil/").is_err());
    }

    #[test]
    fn generic_oidc_no_roles_yields_viewer() {
        let c = serde_json::json!({ "sub": "u", "name": "No Roles" });
        let id = identity_from_oidc_claims(&c, "tid");
        assert_eq!(id.top_role(), Role::Viewer);
        assert!(!id.has_role(Role::Reviewer));
    }

    // --- GitHub login (#286) -------------------------------------------------

    #[test]
    fn github_user_maps_to_viewer_by_default() {
        let user = serde_json::json!({
            "login": "octocat",
            "id": 583231,
            "name": "The Octocat",
            "email": "octo@github.com",
        });
        let id = identity_from_github_user(&user, &[]);
        assert_eq!(id.subject, "583231");
        assert_eq!(id.name.as_deref(), Some("The Octocat"));
        assert_eq!(id.email.as_deref(), Some("octo@github.com"));
        assert_eq!(id.tenant, "default");
        assert_eq!(id.top_role(), Role::Viewer);
        assert!(!id.has_role(Role::Reviewer));
    }

    #[test]
    fn github_admin_login_maps_to_admin_case_insensitive() {
        let user = serde_json::json!({ "login": "OctoCat", "id": 1 });
        let admins = vec!["octocat".to_string(), "someone-else".to_string()];
        let id = identity_from_github_user(&user, &admins);
        assert!(id.has_role(Role::Admin));
        // Falls back to login for the display name when `name` is null.
        assert_eq!(id.name.as_deref(), Some("OctoCat"));
    }

    #[test]
    fn github_null_name_and_email_are_omitted() {
        let user = serde_json::json!({ "login": "ghost", "id": 10, "name": null, "email": null });
        let id = identity_from_github_user(&user, &[]);
        assert_eq!(id.subject, "10");
        assert_eq!(id.email, None);
        assert_eq!(id.name.as_deref(), Some("ghost"));
    }
}
