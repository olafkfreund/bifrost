//! Secret encryption + resolution (#154).
//!
//! Connections store [`SecretRef`]s, never plaintext. This module:
//!  - **encrypts/decrypts** the inline fallback ([`SecretRef::EncryptedInline`])
//!    with AES-256-GCM under a master key derived from `BIFROST_SECRET_KEY`, so a
//!    DB leak alone never yields a usable secret; and
//!  - **resolves** a [`SecretRef`] to a value at use-time via the [`SecretResolver`]
//!    trait — env vars and the encrypted inline value today; Azure Key Vault and
//!    GitHub App / Entra are wired in later M7 phases.
//!
//! Master key handling: `BIFROST_SECRET_KEY` is hashed (SHA-256) to a 32-byte key,
//! so any passphrase length works. With the key unset, inline encryption is
//! refused (fail closed) — the vault-reference path is unaffected.

// The resolver + decryption path is the *use-time* secret layer: it is exercised
// by the tests here and consumed when connections drive audits (#156). It is not
// called from a request handler in this phase, so allow the not-yet-wired items.
#![allow(dead_code)]

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, KeyInit, Nonce};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use bifrost_core::SecretRef;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("inline secret encryption is not configured (set BIFROST_SECRET_KEY)")]
    NoMasterKey,
    #[error("decryption failed (wrong key or corrupted data)")]
    Decrypt,
    #[error("malformed encrypted value: {0}")]
    Malformed(String),
    #[error("cannot resolve {0} (not supported yet in this build)")]
    Unsupported(String),
    #[error("secret not found: {0}")]
    NotFound(String),
}

/// Derive a 32-byte AES key from any passphrase (SHA-256).
fn derive_key(passphrase: &str) -> Key<Aes256Gcm> {
    *Key::<Aes256Gcm>::from_slice(&Sha256::digest(passphrase.as_bytes()))
}

/// Encrypt under an explicit passphrase (pure — used by tests and the env wrapper).
pub fn encrypt_with(passphrase: &str, plaintext: &str) -> Result<SecretRef, SecretError> {
    let cipher = Aes256Gcm::new(&derive_key(passphrase));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| SecretError::Decrypt)?;
    Ok(SecretRef::EncryptedInline {
        ciphertext: STANDARD.encode(ct),
        nonce: STANDARD.encode(nonce),
    })
}

/// Decrypt under an explicit passphrase (pure).
pub fn decrypt_with(
    passphrase: &str,
    ciphertext: &str,
    nonce: &str,
) -> Result<String, SecretError> {
    let ct = STANDARD
        .decode(ciphertext)
        .map_err(|e| SecretError::Malformed(e.to_string()))?;
    let nonce_bytes = STANDARD
        .decode(nonce)
        .map_err(|e| SecretError::Malformed(e.to_string()))?;
    if nonce_bytes.len() != 12 {
        return Err(SecretError::Malformed("nonce must be 12 bytes".into()));
    }
    let cipher = Aes256Gcm::new(&derive_key(passphrase));
    let pt = cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ct.as_ref())
        .map_err(|_| SecretError::Decrypt)?;
    String::from_utf8(pt).map_err(|e| SecretError::Malformed(e.to_string()))
}

/// Encrypt `plaintext` into a [`SecretRef::EncryptedInline`] under the configured
/// master key. Fails closed if `BIFROST_SECRET_KEY` is unset.
pub fn encrypt_inline(plaintext: &str) -> Result<SecretRef, SecretError> {
    let raw = std::env::var("BIFROST_SECRET_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(SecretError::NoMasterKey)?;
    encrypt_with(&raw, plaintext)
}

/// Decrypt an inline secret under the configured master key.
pub fn decrypt_inline(ciphertext: &str, nonce: &str) -> Result<String, SecretError> {
    let raw = std::env::var("BIFROST_SECRET_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(SecretError::NoMasterKey)?;
    decrypt_with(&raw, ciphertext, nonce)
}

/// Resolves a [`SecretRef`] to a usable secret value at the point of use.
#[async_trait]
pub trait SecretResolver: Send + Sync {
    async fn resolve(&self, secret: &SecretRef) -> Result<String, SecretError>;
}

/// The default resolver: env vars + the encrypted inline fallback. Key Vault and
/// GitHub App / Entra resolution land in later M7 phases (they need Azure/GitHub
/// auth) and currently return [`SecretError::Unsupported`].
#[derive(Debug, Clone, Default)]
pub struct DefaultSecretResolver;

#[async_trait]
impl SecretResolver for DefaultSecretResolver {
    async fn resolve(&self, secret: &SecretRef) -> Result<String, SecretError> {
        match secret {
            SecretRef::EnvVar { name } => {
                std::env::var(name).map_err(|_| SecretError::NotFound(name.clone()))
            }
            SecretRef::EncryptedInline { ciphertext, nonce } => decrypt_inline(ciphertext, nonce),
            SecretRef::KeyVault { .. } => Err(SecretError::Unsupported("key-vault".into())),
            SecretRef::GitHubApp { .. } => Err(SecretError::Unsupported("github-app".into())),
            SecretRef::EntraWif { .. } => Err(SecretError::Unsupported("entra-wif".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The pure `*_with` functions take the key explicitly, so these don't touch
    // the shared process env and are safe under parallel test execution.

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let r = encrypt_with("master-key", "super-secret-pat").unwrap();
        let SecretRef::EncryptedInline { ciphertext, nonce } = &r else {
            panic!("expected inline")
        };
        // The plaintext is nowhere in the serialized ref.
        assert!(!serde_json::to_string(&r)
            .unwrap()
            .contains("super-secret-pat"));
        assert_eq!(
            decrypt_with("master-key", ciphertext, nonce).unwrap(),
            "super-secret-pat"
        );
    }

    #[test]
    fn decrypt_fails_with_a_different_key() {
        let SecretRef::EncryptedInline { ciphertext, nonce } = encrypt_with("key-a", "x").unwrap()
        else {
            panic!()
        };
        assert!(matches!(
            decrypt_with("key-b", &ciphertext, &nonce),
            Err(SecretError::Decrypt)
        ));
    }

    #[test]
    fn decrypt_rejects_a_malformed_nonce() {
        let SecretRef::EncryptedInline { ciphertext, .. } = encrypt_with("k", "x").unwrap() else {
            panic!()
        };
        assert!(matches!(
            decrypt_with("k", &ciphertext, &STANDARD.encode([0u8; 8])),
            Err(SecretError::Malformed(_))
        ));
    }

    #[tokio::test]
    async fn resolver_decrypts_inline_with_the_master_key() {
        // Build the inline ref with a known key, then resolve via the env master key.
        std::env::set_var("BIFROST_SECRET_KEY", "resolver-key");
        let r = encrypt_with("resolver-key", "value").unwrap();
        assert_eq!(DefaultSecretResolver.resolve(&r).await.unwrap(), "value");
        std::env::remove_var("BIFROST_SECRET_KEY");
    }

    #[tokio::test]
    async fn resolver_reads_env_var_refs() {
        std::env::set_var("BIFROST_TEST_SECRET_X", "from-env");
        let r = SecretRef::EnvVar {
            name: "BIFROST_TEST_SECRET_X".into(),
        };
        assert_eq!(DefaultSecretResolver.resolve(&r).await.unwrap(), "from-env");
        std::env::remove_var("BIFROST_TEST_SECRET_X");
    }

    #[tokio::test]
    async fn vault_and_app_refs_are_unsupported_for_now() {
        let kv = SecretRef::KeyVault {
            uri: "https://kv/secrets/x".into(),
        };
        assert!(matches!(
            DefaultSecretResolver.resolve(&kv).await,
            Err(SecretError::Unsupported(_))
        ));
    }
}
