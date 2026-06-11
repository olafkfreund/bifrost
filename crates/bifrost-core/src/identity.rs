//! Authenticated identity and role model (#65/#66).
//!
//! An [`Identity`] is who is acting — resolved by the API's authenticator (a
//! mock, or Entra ID OIDC) — carrying the roles that gate what they may do. The
//! role set is deliberately small and ordered by privilege; RBAC checks (#66) ask
//! "does this identity have at least role X".

use serde::{Deserialize, Serialize};

/// What an identity is allowed to do, least privilege first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read-only: view the portfolio, proposals, and audit trail.
    Viewer,
    /// Review: convert, edit, and move proposals through the lifecycle.
    Reviewer,
    /// Full control, including settings and (later) tenant administration.
    Admin,
}

impl Role {
    /// Map an identity-provider role/group claim value to a Bifrost role
    /// (case-insensitive). Unknown values return `None` and are ignored.
    pub fn from_claim(value: &str) -> Option<Role> {
        match value.trim().to_ascii_lowercase().as_str() {
            "admin" | "administrator" | "owner" => Some(Role::Admin),
            "reviewer" | "approver" | "editor" => Some(Role::Reviewer),
            "viewer" | "reader" | "read" => Some(Role::Viewer),
            _ => None,
        }
    }
}

/// An authenticated principal and the roles it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Stable subject id (Entra `oid`/`sub`).
    pub subject: String,
    /// Display name, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Email / UPN, if known — used as the audit actor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Tenant the principal belongs to (multi-tenancy, #66). `default` when
    /// tenancy isn't configured.
    #[serde(default = "default_tenant")]
    pub tenant: String,
    /// Granted roles.
    pub roles: Vec<Role>,
}

fn default_tenant() -> String {
    "default".to_string()
}

impl Identity {
    /// A local/dev identity with full rights — used when authentication is
    /// disabled so the system is usable out of the box (and clearly labelled).
    pub fn local_admin() -> Self {
        Self {
            subject: "local".into(),
            name: Some("Local Admin".into()),
            email: Some("local@bifrost".into()),
            tenant: default_tenant(),
            roles: vec![Role::Admin],
        }
    }

    /// The highest role held (privilege order), or `Viewer` if none.
    pub fn top_role(&self) -> Role {
        self.roles.iter().copied().max().unwrap_or(Role::Viewer)
    }

    /// Whether the identity holds at least `required` (privilege-ordered).
    pub fn has_role(&self, required: Role) -> bool {
        self.top_role() >= required
    }

    /// The actor string for the audit log: email, else name, else subject.
    pub fn actor(&self) -> String {
        self.email
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.subject.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_privilege_ordered() {
        assert!(Role::Admin > Role::Reviewer);
        assert!(Role::Reviewer > Role::Viewer);
    }

    #[test]
    fn role_from_claim_is_case_insensitive_and_aliased() {
        assert_eq!(Role::from_claim("Admin"), Some(Role::Admin));
        assert_eq!(Role::from_claim("APPROVER"), Some(Role::Reviewer));
        assert_eq!(Role::from_claim("reader"), Some(Role::Viewer));
        assert_eq!(Role::from_claim("nonsense"), None);
    }

    #[test]
    fn has_role_uses_highest_held() {
        let id = Identity {
            subject: "s".into(),
            name: None,
            email: None,
            tenant: "default".into(),
            roles: vec![Role::Viewer, Role::Reviewer],
        };
        assert!(id.has_role(Role::Reviewer));
        assert!(id.has_role(Role::Viewer));
        assert!(!id.has_role(Role::Admin));
        assert_eq!(id.top_role(), Role::Reviewer);
    }

    #[test]
    fn actor_prefers_email_then_name_then_subject() {
        let mut id = Identity::local_admin();
        assert_eq!(id.actor(), "local@bifrost");
        id.email = None;
        assert_eq!(id.actor(), "Local Admin");
        id.name = None;
        assert_eq!(id.actor(), "local");
    }

    #[test]
    fn no_roles_defaults_to_viewer() {
        let id = Identity {
            subject: "s".into(),
            name: None,
            email: None,
            tenant: "default".into(),
            roles: vec![],
        };
        assert_eq!(id.top_role(), Role::Viewer);
        assert!(!id.has_role(Role::Reviewer));
    }
}
