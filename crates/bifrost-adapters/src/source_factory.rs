//! Build a [`SourceAdapter`] from a resolved source connection (#207).
//!
//! The portal stores a `ConnectionKind::Source { platform, base_url, auth,
//! username }`; at use-time the secret is resolved and this factory maps the
//! platform string to the matching adapter. Azure DevOps keeps its own dedicated
//! path (Entra/PAT auth); this covers the other six Importer-supported sources.

use crate::source::SourceAdapter;
use crate::{
    BambooAdapter, BitbucketAdapter, CircleCiAdapter, GitLabAdapter, JenkinsAdapter, TravisAdapter,
};

/// Construct a boxed [`SourceAdapter`] for `platform` from a resolved connection.
///
/// `base_url` is the platform's primary locator (host URL for Jenkins/GitLab/
/// Bamboo, the **workspace** for Bitbucket, ignored for CircleCI/Travis cloud);
/// `username` is required for the basic-auth platforms (Jenkins, Bitbucket);
/// `secret` is the resolved token / app password. Returns `None` for an unknown
/// platform or a missing required field.
pub fn source_adapter_from(
    platform: &str,
    base_url: Option<&str>,
    username: Option<&str>,
    secret: &str,
) -> Option<Box<dyn SourceAdapter>> {
    match platform {
        "jenkins" => Some(Box::new(JenkinsAdapter::new(base_url?, username?, secret))),
        "gitlab" => Some(Box::new(GitLabAdapter::new(
            base_url.unwrap_or("https://gitlab.com"),
            secret,
        ))),
        // For Bitbucket the connection's base_url carries the workspace slug.
        "bitbucket" => Some(Box::new(BitbucketAdapter::new(
            base_url?, username?, secret,
        ))),
        "circleci" => Some(Box::new(CircleCiAdapter::new(secret))),
        "travis" => Some(Box::new(TravisAdapter::new(
            base_url.unwrap_or("https://api.travis-ci.com"),
            secret,
        ))),
        "bamboo" => Some(Box::new(BambooAdapter::new(base_url?, secret))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_each_supported_platform() {
        for p in [
            "jenkins",
            "gitlab",
            "bitbucket",
            "circleci",
            "travis",
            "bamboo",
        ] {
            // Provide base_url + username so the basic-auth ones build too.
            let a = source_adapter_from(p, Some("https://host.example"), Some("user"), "secret");
            assert!(a.is_some(), "{p} should build");
        }
    }

    #[test]
    fn cloud_platforms_build_without_a_base_url() {
        assert!(source_adapter_from("gitlab", None, None, "t").is_some());
        assert!(source_adapter_from("circleci", None, None, "t").is_some());
        assert!(source_adapter_from("travis", None, None, "t").is_some());
    }

    #[test]
    fn missing_required_fields_and_unknown_platform_do_not_build() {
        // Jenkins needs a base_url and a username.
        assert!(source_adapter_from("jenkins", None, Some("u"), "t").is_none());
        assert!(source_adapter_from("jenkins", Some("https://j"), None, "t").is_none());
        // Bamboo needs a base_url.
        assert!(source_adapter_from("bamboo", None, None, "t").is_none());
        // Unknown platform.
        assert!(source_adapter_from("teamcity", Some("https://x"), Some("u"), "t").is_none());
    }
}
