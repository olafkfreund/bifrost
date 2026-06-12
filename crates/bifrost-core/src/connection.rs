//! Connections — typed, per-tenant links to external systems (#154).
//!
//! A [`Connection`] is how a tenant tells Bifrost about an Azure DevOps org, a
//! GitHub org, or an LLM provider. The cardinal rule for a regulated enterprise:
//! **Bifrost stores references to secrets, not secret values.** The preferred
//! forms are an Azure Key Vault URI, a GitHub App installation, or Entra
//! workload-identity federation; an envelope-**encrypted** inline value is a
//! clearly-labelled fallback for teams without a vault. Plaintext secrets never
//! live in a [`Connection`], and [`Connection::redacted`] guarantees nothing
//! secret (not even the ciphertext) is ever sent to a client.

use serde::{Deserialize, Serialize};

/// How a secret is referenced. Resolution to an actual value happens at use-time,
/// outside the domain layer (see the API's secret resolver).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SecretRef {
    /// An environment variable name (transitional / local dev).
    EnvVar { name: String },
    /// An Azure Key Vault secret URI (preferred).
    KeyVault { uri: String },
    /// A GitHub App installation — Bifrost mints a least-privilege token (#64).
    GitHubApp { installation_id: String },
    /// Microsoft Entra workload-identity federation — no stored secret at all.
    EntraWif {
        tenant_id: String,
        client_id: String,
    },
    /// Envelope-encrypted inline value (the fallback). The ciphertext + nonce are
    /// stored; the plaintext is never present and never serialized to a client.
    EncryptedInline { ciphertext: String, nonce: String },
}

impl SecretRef {
    /// A client-safe view: keeps the *kind* and non-sensitive locators (vault URI,
    /// installation id) but strips any encrypted material entirely.
    pub fn redacted(&self) -> SecretRef {
        match self {
            SecretRef::EncryptedInline { .. } => SecretRef::EncryptedInline {
                ciphertext: String::new(),
                nonce: String::new(),
            },
            other => other.clone(),
        }
    }

    /// Whether this reference holds (encrypted) secret material in the DB, vs
    /// pointing at an external system that holds the secret.
    pub fn is_inline(&self) -> bool {
        matches!(self, SecretRef::EncryptedInline { .. })
    }
}

/// What a connection links to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ConnectionKind {
    #[serde(rename = "azure-devops")]
    AzureDevOps { org_url: String, auth: SecretRef },
    #[serde(rename = "github")]
    GitHub { org: String, auth: SecretRef },
    Llm {
        /// Provider family: `anthropic` | `gemini` | `github-models` |
        /// `openai-compatible` | `ollama`.
        provider: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<SecretRef>,
        /// Runs on infrastructure the tenant controls (air-gap eligible).
        #[serde(default)]
        is_local: bool,
        /// Data-residency label (e.g. `eu`, `on-prem`) for sovereign routing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        residency: Option<String>,
    },
    /// A CI/CD **source** to migrate (#207): Jenkins, GitLab, Bitbucket, CircleCI,
    /// Travis, or Bamboo. Azure DevOps keeps its own dedicated variant above.
    Source {
        /// `jenkins` | `gitlab` | `bitbucket` | `circleci` | `travis` | `bamboo`.
        platform: String,
        /// Host / primary locator: the server URL (Jenkins/GitLab/Bamboo), the
        /// workspace (Bitbucket), or empty for the cloud default (CircleCI/Travis).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base_url: Option<String>,
        /// The API token / app password (resolved at use-time).
        auth: SecretRef,
        /// Username for basic-auth platforms (Jenkins, Bitbucket).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        username: Option<String>,
    },
}

impl ConnectionKind {
    fn redacted(&self) -> ConnectionKind {
        match self {
            ConnectionKind::AzureDevOps { org_url, auth } => ConnectionKind::AzureDevOps {
                org_url: org_url.clone(),
                auth: auth.redacted(),
            },
            ConnectionKind::GitHub { org, auth } => ConnectionKind::GitHub {
                org: org.clone(),
                auth: auth.redacted(),
            },
            ConnectionKind::Llm {
                provider,
                base_url,
                model,
                key,
                is_local,
                residency,
            } => ConnectionKind::Llm {
                provider: provider.clone(),
                base_url: base_url.clone(),
                model: model.clone(),
                key: key.as_ref().map(SecretRef::redacted),
                is_local: *is_local,
                residency: residency.clone(),
            },
            ConnectionKind::Source {
                platform,
                base_url,
                auth,
                username,
            } => ConnectionKind::Source {
                platform: platform.clone(),
                base_url: base_url.clone(),
                auth: auth.redacted(),
                username: username.clone(),
            },
        }
    }
}

/// A named connection owned by a tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub tenant: String,
    pub name: String,
    pub kind: ConnectionKind,
    /// Who created/last-edited it (identity actor) and when (ISO-8601).
    pub updated_by: String,
    pub updated_at: String,
}

impl Connection {
    /// A client-safe copy with all secret material stripped — the only form that
    /// should ever leave the server.
    pub fn redacted(&self) -> Connection {
        Connection {
            kind: self.kind.redacted(),
            ..self.clone()
        }
    }

    /// The connection kind as a stable label (no secrets) for the config audit.
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            ConnectionKind::AzureDevOps { .. } => "azure-devops",
            ConnectionKind::GitHub { .. } => "github",
            ConnectionKind::Llm { .. } => "llm",
            ConnectionKind::Source { .. } => "source",
        }
    }
}

/// What happened to a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigAction {
    Upserted,
    Deleted,
}

/// An append-only config-change record (#159): who changed which connection,
/// when, and how — never any secret material. Included in the compliance pack so
/// an auditor sees the full configuration history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEvent {
    pub tenant: String,
    pub action: ConfigAction,
    pub connection_id: String,
    pub connection_name: String,
    pub kind: String,
    pub actor: String,
    /// Caller-supplied ISO-8601 timestamp (the core is clock-free).
    pub at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_strips_encrypted_material_but_keeps_locators() {
        let conn = Connection {
            id: "c1".into(),
            tenant: "acme".into(),
            name: "prod-ado".into(),
            kind: ConnectionKind::AzureDevOps {
                org_url: "https://dev.azure.com/acme".into(),
                auth: SecretRef::EncryptedInline {
                    ciphertext: "SECRET-CIPHERTEXT".into(),
                    nonce: "NONCE".into(),
                },
            },
            updated_by: "admin@acme".into(),
            updated_at: "2026-06-11T00:00:00Z".into(),
        };
        let red = conn.redacted();
        // The org url survives; the ciphertext is gone.
        let json = serde_json::to_string(&red).unwrap();
        assert!(json.contains("dev.azure.com/acme"));
        assert!(!json.contains("SECRET-CIPHERTEXT"));
        assert!(!json.contains("NONCE"));
    }

    #[test]
    fn vault_and_app_refs_survive_redaction_unchanged() {
        let kv = SecretRef::KeyVault {
            uri: "https://kv.vault.azure.net/secrets/ado-pat".into(),
        };
        assert_eq!(kv.redacted(), kv);
        let app = SecretRef::GitHubApp {
            installation_id: "12345".into(),
        };
        assert_eq!(app.redacted(), app);
        assert!(!kv.is_inline());
        assert!(SecretRef::EncryptedInline {
            ciphertext: "x".into(),
            nonce: "y".into()
        }
        .is_inline());
    }

    #[test]
    fn llm_connection_round_trips_with_residency_and_local_flag() {
        let conn = ConnectionKind::Llm {
            provider: "openai-compatible".into(),
            base_url: Some("http://gemma.vm.internal:8000/v1".into()),
            model: "gemma-2-12b".into(),
            key: Some(SecretRef::KeyVault {
                uri: "https://kv/secrets/gemma".into(),
            }),
            is_local: true,
            residency: Some("on-prem".into()),
        };
        let json = serde_json::to_string(&conn).unwrap();
        let back: ConnectionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, conn);
    }
}
