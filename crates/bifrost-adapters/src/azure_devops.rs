//! Live Azure DevOps source adapter.
//!
//! Implements [`SourceAdapter`] against the ADO REST API (PAT or Entra auth via
//! [`crate::ado_auth`], rustls). The
//! JSON→domain parsing is pure and fixture-tested; the network methods are
//! integration-tested behind `#[ignore]` (they need a real PAT). Secret *values*
//! are never requested or stored — only variable/connection names and flags.

use std::sync::Arc;

use async_trait::async_trait;
use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup, VariableRef,
};
use serde_json::Value;

use crate::ado_auth::{AdoAuth, EntraAuth, PatAuth};
use crate::source::{AdapterError, SourceAdapter};

const API_VERSION: &str = "7.1";

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Parse `GET _apis/projects` → projects.
pub fn parse_projects(v: &Value) -> Vec<Project> {
    array(v, "value")
        .iter()
        .filter_map(|p| {
            Some(Project {
                id: str_field(p, "id")?.to_string(),
                name: str_field(p, "name")?.to_string(),
            })
        })
        .collect()
}

/// Classify a build definition: a YAML pipeline carries `process.yamlFilename`;
/// anything else (designer phases) is classic.
pub fn classify_definition(def: &Value) -> Classification {
    let has_yaml = def
        .get("process")
        .and_then(|p| p.get("yamlFilename"))
        .and_then(Value::as_str)
        .is_some();
    if has_yaml {
        Classification::Yaml
    } else {
        Classification::Classic
    }
}

/// Build a [`SourcePipeline`] from a fetched definition detail.
pub fn parse_pipeline(def: &Value) -> Option<SourcePipeline> {
    Some(SourcePipeline {
        id: id_field(def, "id")?,
        name: str_field(def, "name")?.to_string(),
        project: def
            .get("project")
            .and_then(|p| str_field(p, "name"))
            .unwrap_or("")
            .to_string(),
        classification: classify_definition(def),
        repository: def
            .get("repository")
            .and_then(|r| str_field(r, "name"))
            .map(String::from),
    })
}

/// Parse `GET _apis/serviceendpoint/endpoints` → connections (name + type only).
pub fn parse_service_connections(v: &Value, project: &str) -> Vec<ServiceConnection> {
    array(v, "value")
        .iter()
        .filter_map(|e| {
            Some(ServiceConnection {
                id: str_field(e, "id")?.to_string(),
                name: str_field(e, "name")?.to_string(),
                kind: str_field(e, "type").unwrap_or("unknown").to_string(),
                project: project.to_string(),
            })
        })
        .collect()
}

/// Parse `GET _apis/distributedtask/variablegroups` → variable groups.
/// Records variable **names** and the `isSecret` flag only — never values.
pub fn parse_variable_groups(v: &Value, project: &str) -> Vec<VariableGroup> {
    array(v, "value")
        .iter()
        .filter_map(|g| {
            let variables = g
                .get("variables")
                .and_then(Value::as_object)
                .map(|obj| {
                    obj.iter()
                        .map(|(name, meta)| VariableRef {
                            name: name.clone(),
                            is_secret: meta
                                .get("isSecret")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(VariableGroup {
                id: id_field(g, "id")?,
                name: str_field(g, "name")?.to_string(),
                project: project.to_string(),
                variables,
            })
        })
        .collect()
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// ADO ids are sometimes numbers, sometimes strings — normalise to String.
fn id_field(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

// ---- live adapter ----------------------------------------------------------

/// Read-only Azure DevOps adapter over the REST API. Authenticates with a PAT or
/// a Microsoft Entra token (#20), behind the [`AdoAuth`] seam.
pub struct AzureDevOpsAdapter {
    client: reqwest::Client,
    /// e.g. `https://dev.azure.com/<org>` (no trailing slash).
    org_url: String,
    project: String,
    auth: Arc<dyn AdoAuth>,
}

impl AzureDevOpsAdapter {
    /// Construct with a PAT (HTTP basic).
    pub fn new(
        org_url: impl Into<String>,
        project: impl Into<String>,
        pat: impl Into<String>,
    ) -> Self {
        Self::with_auth(org_url, project, Arc::new(PatAuth::new(pat)))
    }

    /// Construct with an explicit credential (PAT or Entra).
    pub fn with_auth(
        org_url: impl Into<String>,
        project: impl Into<String>,
        auth: Arc<dyn AdoAuth>,
    ) -> Self {
        let org_url = org_url.into().trim_end_matches('/').to_string();
        Self {
            client: reqwest::Client::new(),
            org_url,
            project: project.into(),
            auth,
        }
    }

    /// Build from the environment. Auth is **Entra** when `AZURE_TENANT_ID` is set
    /// (a service principal — least privilege), else a `AZDO_PAT`. `AZDO_ORG_URL`
    /// is required either way.
    pub fn from_env(project: impl Into<String>) -> Result<Self, AdapterError> {
        let org = std::env::var("AZDO_ORG_URL")
            .map_err(|_| AdapterError::Auth("AZDO_ORG_URL not set".into()))?;
        let auth: Arc<dyn AdoAuth> = match EntraAuth::from_env()? {
            Some(entra) => Arc::new(entra),
            None => {
                let pat = std::env::var("AZDO_PAT")
                    .map_err(|_| AdapterError::Auth("AZDO_PAT or AZURE_* not set".into()))?;
                Arc::new(PatAuth::new(pat))
            }
        };
        Ok(Self::with_auth(org, project, auth))
    }

    /// GET a project-scoped ADO API path and return the JSON body.
    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}/{}/_apis/{path}{sep}api-version={API_VERSION}",
            self.org_url, self.project
        );
        self.send(&url).await
    }

    /// GET an org-scoped (non-project) ADO API path.
    async fn get_org(&self, path: &str) -> Result<Value, AdapterError> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!(
            "{}/_apis/{path}{sep}api-version={API_VERSION}",
            self.org_url
        );
        self.send(&url).await
    }

    async fn send(&self, url: &str) -> Result<Value, AdapterError> {
        let req = self.auth.apply(self.client.get(url)).await?;
        let resp = req
            .send()
            .await
            .map_err(|e| AdapterError::Transport(e.to_string()))?;
        match resp.status().as_u16() {
            200 => resp
                .json()
                .await
                .map_err(|e| AdapterError::Transport(e.to_string())),
            401 | 403 => Err(AdapterError::Auth(format!(
                "ADO returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url.to_string())),
            s => Err(AdapterError::Transport(format!("ADO returned {s}"))),
        }
    }
}

#[async_trait]
impl SourceAdapter for AzureDevOpsAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        Ok(parse_projects(&self.get_org("projects").await?))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        // List definitions, then fetch each detail to classify (YAML vs classic).
        let list = self.get("build/definitions").await?;
        let mut pipelines = Vec::new();
        for d in array(&list, "value") {
            if let Some(id) = id_field(d, "id") {
                let detail = self.get(&format!("build/definitions/{id}")).await?;
                if let Some(p) = parse_pipeline(&detail) {
                    pipelines.push(p);
                }
            }
        }
        Ok(pipelines)
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        let detail = self
            .get(&format!("build/definitions/{pipeline_id}"))
            .await?;
        Ok(PipelineDefinition {
            id: pipeline_id.to_string(),
            classification: classify_definition(&detail),
            // The YAML source lives in the repo; the Importer reads it. Fetching it
            // via the Git API is a follow-up.
            yaml: None,
        })
    }

    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError> {
        let v = self.get("serviceendpoint/endpoints").await?;
        Ok(parse_service_connections(&v, &self.project))
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        let v = self.get("distributedtask/variablegroups").await?;
        Ok(parse_variable_groups(&v, &self.project))
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // Per-task usage for YAML pipelines comes from the Importer audit (which
        // lists every task). Classic-phase extraction is a follow-up; return empty
        // rather than guess.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROJECTS: &str = include_str!("../../../fixtures/ado/projects.json");
    const DEFINITION: &str = include_str!("../../../fixtures/ado/definition.json");
    const ENDPOINTS: &str = include_str!("../../../fixtures/ado/serviceendpoints.json");
    const VARGROUPS: &str = include_str!("../../../fixtures/ado/variablegroups.json");

    fn json(s: &str) -> Value {
        serde_json::from_str(s).expect("valid fixture json")
    }

    #[test]
    fn parses_projects() {
        let projects = parse_projects(&json(PROJECTS));
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "SARC");
    }

    #[test]
    fn classifies_yaml_pipeline_by_yamlfilename() {
        let def = json(DEFINITION);
        assert_eq!(classify_definition(&def), Classification::Yaml);
        let p = parse_pipeline(&def).unwrap();
        assert_eq!(p.name, "SARC-main");
        assert_eq!(p.classification, Classification::Yaml);
    }

    #[test]
    fn classifies_classic_when_no_yamlfilename() {
        let classic = serde_json::json!({
            "id": 42, "name": "legacy-release", "project": {"name": "SARC"},
            "process": {"type": 1, "phases": []}
        });
        assert_eq!(classify_definition(&classic), Classification::Classic);
    }

    #[test]
    fn parses_service_connections_name_and_type() {
        let conns = parse_service_connections(&json(ENDPOINTS), "SARC");
        assert_eq!(conns.len(), 2);
        assert!(conns.iter().any(|c| c.kind == "azurerm"));
    }

    #[test]
    fn parses_variable_groups_names_and_secret_flags_only() {
        let groups = parse_variable_groups(&json(VARGROUPS), "SARC");
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        let secret = g.variables.iter().find(|v| v.is_secret).unwrap();
        assert_eq!(secret.name, "TOKEN");
        // No value field exists on VariableRef — values can't leak by construction.
        assert!(g.variables.iter().any(|v| !v.is_secret));
    }

    /// Secret-handling guard (#107): even when the ADO API response carries secret
    /// VALUES and service-connection credentials, the parsers must keep only
    /// names/types/flags — the value must never survive into the domain model
    /// (and therefore never reach persistence, the audit log, or an LLM request).
    #[test]
    fn parsers_never_retain_secret_values_or_credentials() {
        // A variable group whose secret variable also carries a value (as a real
        // ADO response can), plus a non-secret value.
        let vg = serde_json::json!({
            "value": [{
                "id": 1, "name": "shared",
                "variables": {
                    "API_URL": { "value": "https://example.com", "isSecret": false },
                    "TOKEN":   { "value": "SUPER-SECRET-VALUE", "isSecret": true }
                }
            }]
        });
        // A service connection with credentials in its authorization parameters.
        let sc = serde_json::json!({
            "value": [{
                "id": "c1", "name": "prod-arm", "type": "azurerm",
                "authorization": { "parameters": { "serviceprincipalkey": "CREDENTIAL-XYZ" } }
            }]
        });

        let groups = parse_variable_groups(&vg, "proj");
        let conns = parse_service_connections(&sc, "proj");

        // Names + flags are kept.
        assert!(groups[0]
            .variables
            .iter()
            .any(|v| v.name == "TOKEN" && v.is_secret));
        assert_eq!(conns[0].kind, "azurerm");

        // No secret value or credential survives anywhere in the serialized model.
        let serialized = format!(
            "{}{}",
            serde_json::to_string(&groups).unwrap(),
            serde_json::to_string(&conns).unwrap()
        );
        for forbidden in [
            "SUPER-SECRET-VALUE",
            "CREDENTIAL-XYZ",
            "https://example.com",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "secret/credential value leaked into the model: {forbidden}"
            );
        }
    }

    /// Live smoke test against a real org. Skipped by default; run with creds via
    /// `AZDO_ORG_URL`/`AZDO_PAT` set and `cargo test -- --ignored`. Targets
    /// `BIFROST_TEST_PROJECT` (default `SARC`) so any seeded project can be checked.
    #[tokio::test]
    #[ignore = "requires live ADO credentials"]
    async fn live_discover_and_enumerate() {
        let project = std::env::var("BIFROST_TEST_PROJECT").unwrap_or_else(|_| "SARC".into());
        let adapter = AzureDevOpsAdapter::from_env(&project).expect("AZDO_* env set");
        assert!(!adapter.discover().await.unwrap().is_empty());
        let pipelines = adapter.enumerate_pipelines().await.unwrap();
        eprintln!("{project}: {} pipelines", pipelines.len());
        for p in &pipelines {
            eprintln!("  - {} [{:?}]", p.name, p.classification);
        }
        assert!(!pipelines.is_empty(), "{project} has pipelines");
        assert!(pipelines.iter().all(|p| !p.name.is_empty()));
    }
}
