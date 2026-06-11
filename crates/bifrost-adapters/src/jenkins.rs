//! Jenkins source adapter (#99).
//!
//! A second [`SourceAdapter`] implementation, proving the seam is platform-
//! agnostic: it discovers Jenkins jobs and inventory over the JSON REST API so a
//! Jenkins estate gets the same portfolio heatmap as Azure DevOps. The official
//! Importer handles the actual Jenkins → GitHub Actions conversion; this adapter
//! owns discovery/ingestion.
//!
//! Mapping to the domain model: a **Pipeline job** (`WorkflowJob`, a Jenkinsfile)
//! is pipeline-as-code → [`Classification::Yaml`]; a **Freestyle** job (designer,
//! no script) is the hard tail → [`Classification::Classic`]. Folders become
//! projects; top-level jobs sit in the `root` project. Credentials are recorded
//! by id + type only — never their secret material (hard rule).

use std::sync::Arc;

use async_trait::async_trait;
use bifrost_core::{
    Classification, PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskUsage,
    VariableGroup,
};
use serde_json::Value;

use crate::ado_auth::{AdoAuth, PatAuth};
use crate::source::{AdapterError, SourceAdapter};

// ---- pure parsers (fixture-tested) -----------------------------------------

/// Whether a Jenkins `_class` is a Pipeline (Jenkinsfile) job.
fn is_pipeline(class: &str) -> bool {
    class.contains("WorkflowJob") || class.contains("WorkflowMultiBranchProject")
}

/// Whether a Jenkins `_class` is a folder (a container of jobs).
fn is_folder(class: &str) -> bool {
    class.contains("Folder")
}

/// Flatten the Jenkins `jobs` tree (one folder level) into [`SourcePipeline`]s.
/// Folder name → project; top-level jobs → the `root` project.
pub fn parse_jobs(v: &Value) -> Vec<SourcePipeline> {
    let mut out = Vec::new();
    collect_jobs(v.get("jobs"), "root", &mut out);
    out
}

fn collect_jobs(jobs: Option<&Value>, project: &str, out: &mut Vec<SourcePipeline>) {
    let Some(arr) = jobs.and_then(Value::as_array) else {
        return;
    };
    for job in arr {
        let class = job.get("_class").and_then(Value::as_str).unwrap_or("");
        let name = job.get("name").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        if is_folder(class) {
            // Recurse one level: the folder is a project for its children.
            collect_jobs(job.get("jobs"), name, out);
            continue;
        }
        out.push(SourcePipeline {
            id: if project == "root" {
                name.to_string()
            } else {
                format!("{project}/{name}")
            },
            name: name.to_string(),
            project: project.to_string(),
            classification: if is_pipeline(class) {
                Classification::Yaml
            } else {
                Classification::Classic
            },
            repository: None,
        });
    }
}

/// Distinct projects (folders + `root`) implied by the jobs tree.
pub fn parse_projects(v: &Value) -> Vec<Project> {
    let mut projects: Vec<String> = parse_jobs(v).into_iter().map(|p| p.project).collect();
    projects.sort();
    projects.dedup();
    projects
        .into_iter()
        .map(|p| Project {
            id: p.clone(),
            name: p,
        })
        .collect()
}

/// Parse the Jenkins credentials list → service connections (id + type only).
pub fn parse_credentials(v: &Value) -> Vec<ServiceConnection> {
    v.get("credentials")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(ServiceConnection {
                        id: c.get("id").and_then(Value::as_str)?.to_string(),
                        name: c
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        kind: c
                            .get("typeName")
                            .and_then(Value::as_str)
                            .unwrap_or("credential")
                            .to_string(),
                        project: "jenkins".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---- live adapter ----------------------------------------------------------

/// Read-only Jenkins adapter over the JSON REST API. Auth is a user + API token
/// (HTTP basic), reusing [`PatAuth`] (Jenkins treats the token like a password).
pub struct JenkinsAdapter {
    client: reqwest::Client,
    base_url: String,
    auth: Arc<dyn AdoAuth>,
}

impl JenkinsAdapter {
    pub fn new(base_url: impl Into<String>, user: &str, token: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            // Jenkins basic auth is `user:token`; PatAuth sends `:token`, so encode
            // the pair as the token side isn't enough — use a combined credential.
            auth: Arc::new(PatAuth::new(format!("{user}:{token}"))),
        }
    }

    /// Build from `JENKINS_URL` + `JENKINS_USER` + `JENKINS_TOKEN`.
    pub fn from_env() -> Result<Self, AdapterError> {
        let base = std::env::var("JENKINS_URL")
            .map_err(|_| AdapterError::Auth("JENKINS_URL not set".into()))?;
        let user = std::env::var("JENKINS_USER")
            .map_err(|_| AdapterError::Auth("JENKINS_USER not set".into()))?;
        let token = std::env::var("JENKINS_TOKEN")
            .map_err(|_| AdapterError::Auth("JENKINS_TOKEN not set".into()))?;
        Ok(Self::new(base, &user, &token))
    }

    async fn get(&self, path: &str) -> Result<Value, AdapterError> {
        let url = format!("{}/{path}", self.base_url);
        let req = self.auth.apply(self.client.get(&url)).await?;
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
                "Jenkins returned {}",
                resp.status()
            ))),
            404 => Err(AdapterError::NotFound(url)),
            s => Err(AdapterError::Transport(format!("Jenkins returned {s}"))),
        }
    }
}

#[async_trait]
impl SourceAdapter for JenkinsAdapter {
    async fn discover(&self) -> Result<Vec<Project>, AdapterError> {
        let v = self
            .get("api/json?tree=jobs[name,_class,jobs[name,_class]]")
            .await?;
        Ok(parse_projects(&v))
    }

    async fn enumerate_pipelines(&self) -> Result<Vec<SourcePipeline>, AdapterError> {
        let v = self
            .get("api/json?tree=jobs[name,_class,jobs[name,_class]]")
            .await?;
        Ok(parse_jobs(&v))
    }

    async fn fetch_definition(
        &self,
        pipeline_id: &str,
    ) -> Result<PipelineDefinition, AdapterError> {
        // The Jenkinsfile lives in SCM; the Importer fetches it at dry-run. Here we
        // only need the classification, recovered from the enumerated set.
        let classification = self
            .enumerate_pipelines()
            .await?
            .into_iter()
            .find(|p| p.id == pipeline_id)
            .map(|p| p.classification)
            .ok_or_else(|| AdapterError::NotFound(pipeline_id.to_string()))?;
        Ok(PipelineDefinition {
            id: pipeline_id.to_string(),
            classification,
            yaml: None,
        })
    }

    async fn fetch_service_connections(&self) -> Result<Vec<ServiceConnection>, AdapterError> {
        // Jenkins exposes credentials by id + type (never the secret) at this path.
        let v = self
            .get("credentials/store/system/domain/_/api/json?tree=credentials[id,typeName,description]")
            .await
            .unwrap_or(Value::Null);
        Ok(parse_credentials(&v))
    }

    async fn fetch_variable_groups(&self) -> Result<Vec<VariableGroup>, AdapterError> {
        // Jenkins has no first-class variable groups; global env is a v2 nicety.
        Ok(Vec::new())
    }

    async fn task_inventory(&self) -> Result<Vec<TaskUsage>, AdapterError> {
        // Plugin/step inventory is a v2 nicety; the Importer's audit reports it.
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JOBS: &str = include_str!("../../../fixtures/jenkins/jobs.json");
    const CREDS: &str = include_str!("../../../fixtures/jenkins/credentials.json");

    #[test]
    fn parses_jobs_classifying_pipeline_vs_freestyle_and_folders() {
        let v: Value = serde_json::from_str(JOBS).unwrap();
        let pipelines = parse_jobs(&v);
        // 2 top-level (one pipeline, one freestyle) + 1 inside the folder.
        assert_eq!(pipelines.len(), 3);

        let web = pipelines
            .iter()
            .find(|p| p.name == "web-portal-ci")
            .unwrap();
        assert_eq!(web.classification, Classification::Yaml);
        assert_eq!(web.project, "root");

        let legacy = pipelines.iter().find(|p| p.name == "legacy-build").unwrap();
        assert_eq!(legacy.classification, Classification::Classic);

        // The folder job is tagged with the folder as its project + a path id.
        let tf = pipelines
            .iter()
            .find(|p| p.name == "terraform-apply")
            .unwrap();
        assert_eq!(tf.project, "platform");
        assert_eq!(tf.id, "platform/terraform-apply");
    }

    #[test]
    fn projects_are_the_folders_plus_root() {
        let v: Value = serde_json::from_str(JOBS).unwrap();
        let names: Vec<_> = parse_projects(&v).into_iter().map(|p| p.name).collect();
        assert_eq!(names, vec!["platform".to_string(), "root".to_string()]);
    }

    #[test]
    fn credentials_record_id_and_type_only() {
        let v: Value = serde_json::from_str(CREDS).unwrap();
        let conns = parse_credentials(&v);
        assert_eq!(conns.len(), 2);
        assert!(conns
            .iter()
            .any(|c| c.id == "azure-sp" && c.kind.contains("Service Principal")));
        // The model carries only id/name/kind/project — there is no field that
        // could hold a secret value (type labels like "Username with password" are
        // not secrets).
        assert!(conns.iter().all(|c| !c.id.is_empty() && !c.kind.is_empty()));
    }

    /// Live: enumerate a real Jenkins. Ignored by default (needs JENKINS_* env).
    #[tokio::test]
    #[ignore = "requires a live Jenkins (JENKINS_URL / JENKINS_USER / JENKINS_TOKEN)"]
    async fn live_enumerate() {
        let adapter = JenkinsAdapter::from_env().expect("JENKINS_* set");
        let pipelines = adapter.enumerate_pipelines().await.expect("enumerate");
        assert!(!pipelines.is_empty());
    }
}
