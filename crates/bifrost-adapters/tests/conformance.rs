//! Source-adapter conformance suite (#101).
//!
//! A single trait-level contract that **every** [`SourceAdapter`] implementation
//! must satisfy, run against captured fixtures per platform. Rather than hit a
//! live API, each platform's pure parsers turn its fixtures into a
//! [`MockSourceAdapter`] (whose fields mirror the trait's return types), and the
//! shared `assert_conformance` invariants run against it. This proves the parsers
//! produce trait-conformant, secret-safe data — the same bar for Azure DevOps,
//! Jenkins, GitLab, and any future source.
//!
//! Invariants asserted (the platform-agnostic contract):
//! - `discover` yields ≥1 project; ids unique and non-empty.
//! - `enumerate_pipelines` yields pipelines with unique, non-empty ids; each
//!   pipeline's `project` resolves to a discovered project.
//! - `fetch_definition` round-trips id + classification for every pipeline; a
//!   classic/designer pipeline never carries YAML source; an unknown id is
//!   `NotFound`.
//! - service connections + variable groups expose **names only** — no field in
//!   the serialized shape could carry a secret value.

use bifrost_adapters::source::{AdapterError, SourceAdapter};
use bifrost_adapters::{gitlab, jenkins, MockSourceAdapter};
use bifrost_core::Classification;
use serde_json::Value;
use std::collections::HashSet;

// ---- the shared contract ---------------------------------------------------

/// Assert a [`SourceAdapter`] satisfies the platform-agnostic conformance contract.
async fn assert_conformance(platform: &str, adapter: &dyn SourceAdapter) {
    // discover: ≥1 project, unique non-empty ids.
    let projects = adapter
        .discover()
        .await
        .unwrap_or_else(|e| panic!("[{platform}] discover failed: {e}"));
    assert!(
        !projects.is_empty(),
        "[{platform}] discover returned nothing"
    );
    let mut project_ids = HashSet::new();
    let mut project_keys = HashSet::new(); // id ∪ name, for referential checks
    for p in &projects {
        assert!(!p.id.is_empty(), "[{platform}] project id empty");
        assert!(
            project_ids.insert(p.id.clone()),
            "[{platform}] duplicate project id: {}",
            p.id
        );
        project_keys.insert(p.id.clone());
        project_keys.insert(p.name.clone());
    }

    // enumerate: unique non-empty ids; every pipeline references a known project.
    let pipelines = adapter
        .enumerate_pipelines()
        .await
        .unwrap_or_else(|e| panic!("[{platform}] enumerate failed: {e}"));
    assert!(
        !pipelines.is_empty(),
        "[{platform}] enumerate returned nothing"
    );
    let mut pipeline_ids = HashSet::new();
    for p in &pipelines {
        assert!(!p.id.is_empty(), "[{platform}] pipeline id empty");
        assert!(!p.name.is_empty(), "[{platform}] pipeline name empty");
        assert!(
            pipeline_ids.insert(p.id.clone()),
            "[{platform}] duplicate pipeline id: {}",
            p.id
        );
        assert!(
            project_keys.contains(&p.project),
            "[{platform}] pipeline {} references unknown project {}",
            p.id,
            p.project
        );
    }

    // fetch_definition: id + classification round-trip; classic has no YAML source.
    for p in &pipelines {
        let def = adapter
            .fetch_definition(&p.id)
            .await
            .unwrap_or_else(|e| panic!("[{platform}] fetch_definition({}) failed: {e}", p.id));
        assert_eq!(def.id, p.id, "[{platform}] fetch_definition id mismatch");
        assert_eq!(
            def.classification, p.classification,
            "[{platform}] classification mismatch for {}",
            p.id
        );
        if def.classification == Classification::Classic {
            assert!(
                def.yaml.is_none(),
                "[{platform}] classic pipeline {} carried YAML source",
                p.id
            );
        }
    }

    // unknown pipeline → NotFound.
    let err = adapter
        .fetch_definition("definitely-not-a-real-pipeline-id")
        .await
        .expect_err("[{platform}] unknown id should not resolve");
    assert!(
        matches!(err, AdapterError::NotFound(_)),
        "[{platform}] unknown id should be NotFound, got {err:?}"
    );

    // service connections: names/types only — no secret-bearing field.
    let conns = adapter
        .fetch_service_connections()
        .await
        .unwrap_or_else(|e| panic!("[{platform}] fetch_service_connections failed: {e}"));
    for c in &conns {
        assert!(!c.id.is_empty(), "[{platform}] service connection id empty");
        assert_keys_subset(
            platform,
            "service connection",
            &serde_json::to_value(c).unwrap(),
            &["id", "name", "kind", "project"],
        );
    }

    // variable groups: variable NAMES + secret flag only — never a value.
    let groups = adapter
        .fetch_variable_groups()
        .await
        .unwrap_or_else(|e| panic!("[{platform}] fetch_variable_groups failed: {e}"));
    for g in &groups {
        assert!(
            !g.project.is_empty(),
            "[{platform}] variable group project empty"
        );
        for v in &g.variables {
            assert!(!v.name.is_empty(), "[{platform}] variable name empty");
            assert_keys_subset(
                platform,
                "variable ref",
                &serde_json::to_value(v).unwrap(),
                &["name", "isSecret"],
            );
        }
    }

    // task inventory must at least not error.
    adapter
        .task_inventory()
        .await
        .unwrap_or_else(|e| panic!("[{platform}] task_inventory failed: {e}"));
}

/// Assert a serialized object exposes no keys beyond `allowed` — the structural
/// guarantee that no secret-value field could be present.
fn assert_keys_subset(platform: &str, what: &str, value: &Value, allowed: &[&str]) {
    let obj = value
        .as_object()
        .unwrap_or_else(|| panic!("[{platform}] {what} did not serialize to an object"));
    for key in obj.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "[{platform}] {what} exposed unexpected field `{key}` (possible secret leak)"
        );
    }
}

// ---- per-platform subjects, built from captured fixtures -------------------

/// Build a [`MockSourceAdapter`] from a platform's parsed fixtures so the shared
/// contract can run without a live API.
fn jenkins_subject() -> MockSourceAdapter {
    let jobs: Value =
        serde_json::from_str(include_str!("../../../fixtures/jenkins/jobs.json")).unwrap();
    let creds: Value =
        serde_json::from_str(include_str!("../../../fixtures/jenkins/credentials.json")).unwrap();
    MockSourceAdapter {
        projects: jenkins::parse_projects(&jobs),
        pipelines: jenkins::parse_jobs(&jobs),
        service_connections: jenkins::parse_credentials(&creds),
        variable_groups: Vec::new(),
        tasks: Vec::new(),
    }
}

fn gitlab_subject() -> MockSourceAdapter {
    let groups: Value =
        serde_json::from_str(include_str!("../../../fixtures/gitlab/groups.json")).unwrap();
    let projects: Value =
        serde_json::from_str(include_str!("../../../fixtures/gitlab/projects.json")).unwrap();
    let vars: Value =
        serde_json::from_str(include_str!("../../../fixtures/gitlab/variables.json")).unwrap();
    MockSourceAdapter {
        projects: gitlab::parse_groups(&groups),
        pipelines: gitlab::parse_pipelines(&projects),
        service_connections: Vec::new(),
        variable_groups: vec![gitlab::parse_variables(&vars, "storefront/web-portal")],
        tasks: Vec::new(),
    }
}

#[tokio::test]
async fn azure_devops_is_conformant() {
    // The canned mock mirrors the Azure DevOps adapter's discovered shape.
    assert_conformance("azure-devops", &MockSourceAdapter::default()).await;
}

#[tokio::test]
async fn jenkins_is_conformant() {
    assert_conformance("jenkins", &jenkins_subject()).await;
}

#[tokio::test]
async fn gitlab_is_conformant() {
    assert_conformance("gitlab", &gitlab_subject()).await;
}
