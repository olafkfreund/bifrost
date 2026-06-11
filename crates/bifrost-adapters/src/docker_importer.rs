//! Docker-backed [`Importer`] — drives the official `ghcr.io/actions-importer/cli`
//! image directly (no gh extension, which is read-only under home-manager).
//!
//! Tokens are passed by forwarding host env vars (`-e NAME`, value set on the
//! child process) so they never appear in the process argv. Audit output is
//! parsed by the validated [`parse_audit_summary`].
//!
//! Disk safety: the Importer can run away and write an unbounded log for some
//! pipeline shapes, which on a small `/tmp` (often tmpfs) fills the disk. Two
//! guards prevent that: a hard per-file size cap (`--ulimit fsize`) bounds any
//! single file, and the container runs as the host user (`--user`) so its output
//! is owned by us — cleanable, and cleaned both before and after each run.
//! `BIFROST_IMPORTER_WORKDIR` redirects the work dir off a small tmpfs.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use bifrost_core::{AuditSummary, DryRunResult};
use tokio::process::Command;

use crate::importer::{
    converted_ratio, parse_audit_summary, parse_converted_workflow, parse_forecast, Forecast,
    Importer, ImporterError,
};

const DEFAULT_IMAGE: &str = "ghcr.io/actions-importer/cli:latest";
/// Per-process counter giving each audit a unique work dir, so concurrent
/// audits (the bulk conversion job) never clobber each other's report.
static AUDIT_SEQ: AtomicU64 = AtomicU64::new(0);
/// Hard cap on any single file the Importer writes (1 GiB) — bounds a runaway
/// log so it can never fill the disk. Real audit reports are far smaller.
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;

/// `id <flag>` as a string (uid/gid), or `None` if it can't be read.
async fn id_value(flag: &str) -> Option<String> {
    let out = Command::new("id").arg(flag).output().await.ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Host `uid:gid` for `docker --user`, so the Importer's output is owned by us
/// (cleanable, no root-owned leftovers). `None` falls back to the image default.
async fn host_user() -> Option<String> {
    Some(format!(
        "{}:{}",
        id_value("-u").await?,
        id_value("-g").await?
    ))
}

/// Extract the org name from an ADO org URL (`https://dev.azure.com/<org>` → `<org>`).
pub fn org_from_url(url: &str) -> &str {
    url.trim_end_matches('/').rsplit('/').next().unwrap_or(url)
}

/// Runs the official Importer image as a subprocess.
pub struct DockerImporter {
    image: String,
    organization: String,
    project: String,
}

impl DockerImporter {
    pub fn new(organization: impl Into<String>, project: impl Into<String>) -> Self {
        Self {
            image: DEFAULT_IMAGE.to_string(),
            organization: organization.into(),
            project: project.into(),
        }
    }

    /// Derive the organization from `AZDO_ORG_URL`; project supplied by caller.
    pub fn from_env(project: impl Into<String>) -> Result<Self, ImporterError> {
        let url = std::env::var("AZDO_ORG_URL")
            .map_err(|_| ImporterError::Subprocess("AZDO_ORG_URL not set".into()))?;
        Ok(Self::new(org_from_url(&url).to_string(), project))
    }

    fn err(msg: impl Into<String>) -> ImporterError {
        ImporterError::Subprocess(msg.into())
    }

    /// Read the credentials the Importer needs from the environment.
    fn creds() -> Result<(String, String), ImporterError> {
        let gh = std::env::var("GITHUB_TOKEN").map_err(|_| Self::err("GITHUB_TOKEN not set"))?;
        let pat = std::env::var("AZDO_PAT").map_err(|_| Self::err("AZDO_PAT not set"))?;
        Ok((gh, pat))
    }
}

#[async_trait::async_trait]
impl Importer for DockerImporter {
    async fn version(&self) -> Result<String, ImporterError> {
        let out = Command::new("docker")
            .args(["run", "--rm", &self.image, "version"])
            .output()
            .await
            .map_err(|e| Self::err(format!("docker spawn failed: {e}")))?;
        if !out.status.success() {
            return Err(Self::err("docker run version failed"));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    async fn audit(&self) -> Result<AuditSummary, ImporterError> {
        let out_dir = self.run_audit().await?;
        let summary_path = out_dir.join("report").join("audit_summary.md");
        let result = tokio::fs::read_to_string(&summary_path)
            .await
            .map(|md| parse_audit_summary(&md))
            .map_err(|e| Self::err(format!("could not read audit_summary.md: {e}")));
        let _ = tokio::fs::remove_dir_all(&out_dir).await;
        result
    }

    async fn forecast(&self) -> Result<Forecast, ImporterError> {
        let out_dir = self.run_forecast().await?;
        let report_path = out_dir.join("report").join("forecast_report.md");
        let result = tokio::fs::read_to_string(&report_path)
            .await
            .map(|md| parse_forecast(&md))
            .map_err(|e| Self::err(format!("could not read forecast_report.md: {e}")));
        let _ = tokio::fs::remove_dir_all(&out_dir).await;
        result
    }

    async fn dry_run(&self, pipeline_id: &str) -> Result<DryRunResult, ImporterError> {
        // The Importer's per-pipeline output already contains everything a
        // dry-run needs: the converted workflow (its gaps marked inline), the ADO
        // source, and the definition id (in config.json). Audit the project, then
        // read the requested pipeline's report.
        let out_dir = self.run_audit().await?;
        let pipelines_dir = out_dir.join("report").join("pipelines").join(&self.project);

        let result = self.read_pipeline(&pipelines_dir, pipeline_id).await;
        let _ = tokio::fs::remove_dir_all(&out_dir).await;
        result
    }
}

impl DockerImporter {
    /// Run `audit azure-devops` for the project into a fresh work dir.
    async fn run_audit(&self) -> Result<PathBuf, ImporterError> {
        self.run_command(vec![
            "audit".into(),
            "azure-devops".into(),
            "--output-dir".into(),
            "report".into(),
            "--azure-devops-organization".into(),
            self.organization.clone(),
            "--azure-devops-project".into(),
            self.project.clone(),
        ])
        .await
    }

    /// Run `forecast azure-devops` for the project into a fresh work dir.
    async fn run_forecast(&self) -> Result<PathBuf, ImporterError> {
        self.run_command(vec![
            "forecast".into(),
            "azure-devops".into(),
            "--output-dir".into(),
            "report".into(),
            "--azure-devops-organization".into(),
            self.organization.clone(),
            "--azure-devops-project".into(),
            self.project.clone(),
        ])
        .await
    }

    /// Run an Importer subcommand into a fresh work dir and return it. Guards
    /// against filling the disk: a per-file size cap, host-user output (cleanable),
    /// and a redirectable work dir. Callers clean the dir when done.
    async fn run_command(&self, sub_args: Vec<String>) -> Result<PathBuf, ImporterError> {
        let (gh, pat) = Self::creds()?;
        // Work dir — redirectable off a small tmpfs via BIFROST_IMPORTER_WORKDIR.
        let base = std::env::var("BIFROST_IMPORTER_WORKDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        // Unique per call: concurrent audits (the bulk job) mustn't share a dir.
        let seq = AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
        let out_dir = base.join(format!(
            "bifrost-importer-audit-{}-{seq}",
            std::process::id()
        ));
        let _ = tokio::fs::remove_dir_all(&out_dir).await; // works now (host-owned)
        tokio::fs::create_dir_all(&out_dir)
            .await
            .map_err(|e| Self::err(format!("could not create output dir: {e}")))?;

        // Tokens forwarded via `-e NAME` (values set on the child env), never argv.
        let mut cmd = Command::new("docker");
        cmd.args(["run", "--rm"]);
        // Hard cap on any single file so a runaway log can't fill the disk.
        cmd.args(["--ulimit", &format!("fsize={MAX_FILE_BYTES}")]);
        // Run as the host user so output is owned by us (cleanable).
        if let Some(user) = host_user().await {
            cmd.args(["--user", &user]);
        }
        cmd.args(["-e", "GITHUB_ACCESS_TOKEN", "-e", "GITHUB_INSTANCE_URL"]);
        cmd.args([
            "-e",
            "AZURE_DEVOPS_ACCESS_TOKEN",
            "-e",
            "AZURE_DEVOPS_INSTANCE_URL",
        ]);
        cmd.arg("-v").arg(format!("{}:/data", out_dir.display()));
        cmd.args(["-w", "/data", &self.image]);
        for a in &sub_args {
            cmd.arg(a);
        }
        cmd.env("GITHUB_ACCESS_TOKEN", gh)
            .env("GITHUB_INSTANCE_URL", "https://github.com")
            .env("AZURE_DEVOPS_ACCESS_TOKEN", pat)
            .env("AZURE_DEVOPS_INSTANCE_URL", "https://dev.azure.com");

        let out = cmd
            .output()
            .await
            .map_err(|e| Self::err(format!("docker spawn failed: {e}")))?;
        // Surface the importer's own logs on stderr for visibility.
        eprint!("{}", String::from_utf8_lossy(&out.stdout));
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            let _ = tokio::fs::remove_dir_all(&out_dir).await;
            return Err(Self::err("importer command failed (see docker output)"));
        }
        Ok(out_dir)
    }

    /// Read the per-pipeline report matching `pipeline_id` (by definition id in
    /// config.json, else by directory name) into a [`DryRunResult`].
    async fn read_pipeline(
        &self,
        pipelines_dir: &std::path::Path,
        pipeline_id: &str,
    ) -> Result<DryRunResult, ImporterError> {
        let mut entries = tokio::fs::read_dir(pipelines_dir)
            .await
            .map_err(|e| Self::err(format!("no pipeline reports: {e}")))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name_match = dir.file_name().and_then(|n| n.to_str()) == Some(pipeline_id);
            let config = tokio::fs::read_to_string(dir.join("config.json"))
                .await
                .unwrap_or_default();
            let id_match = definition_id(&config) == Some(pipeline_id.to_string());
            if !(name_match || id_match) {
                continue;
            }

            let source_yaml = tokio::fs::read_to_string(dir.join("source.yml"))
                .await
                .unwrap_or_default();
            let converted_yaml = read_converted_workflow(&dir).await.unwrap_or_default();
            let gaps = parse_converted_workflow(&converted_yaml);
            let converted_ratio = converted_ratio(&converted_yaml, &gaps);
            return Ok(DryRunResult {
                pipeline_id: pipeline_id.to_string(),
                converted_ratio,
                gaps,
                converted_yaml,
                source_yaml,
            });
        }
        Err(Self::err(format!(
            "no report for pipeline '{pipeline_id}' in project '{}'",
            self.project
        )))
    }
}

/// The ADO build-definition id from a report's `config.json` (`…/Definitions/<id>`).
fn definition_id(config_json: &str) -> Option<String> {
    let pos = config_json.find("Definitions/")? + "Definitions/".len();
    let id: String = config_json[pos..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!id.is_empty()).then_some(id)
}

/// Read the first converted workflow under `<dir>/.github/workflows/`.
async fn read_converted_workflow(dir: &std::path::Path) -> Option<String> {
    let wf_dir = dir.join(".github").join("workflows");
    let mut entries = tokio::fs::read_dir(&wf_dir).await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("yml") {
            return tokio::fs::read_to_string(&p).await.ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_org_from_url() {
        assert_eq!(org_from_url("https://dev.azure.com/contoso"), "contoso");
        assert_eq!(org_from_url("https://dev.azure.com/contoso/"), "contoso");
    }

    #[test]
    fn extracts_definition_id_from_config() {
        let cfg = r#"{"_links":{"self":{"href":"https://dev.azure.com/o/x/_apis/build/Definitions/11?revision=1"}}}"#;
        assert_eq!(definition_id(cfg), Some("11".to_string()));
        assert_eq!(definition_id("{}"), None);
    }

    /// Live: pull-and-run the image. Skipped by default (needs Docker + creds).
    #[tokio::test]
    #[ignore = "requires Docker + live ADO/GitHub credentials"]
    async fn live_audit_against_real_org() {
        let imp = DockerImporter::from_env("SARC").unwrap();
        let summary = imp.audit().await.expect("audit succeeds");
        assert!(summary.pipelines.total > 0);
        assert!(summary.build_steps.total > 0);
    }

    /// Live per-pipeline dry-run. Targets `BIFROST_TEST_PROJECT`/`BIFROST_TEST_PIPELINE`
    /// (default Contoso-Payments / 11). Run with creds + `-- --ignored`.
    #[tokio::test]
    #[ignore = "requires Docker + live ADO/GitHub credentials"]
    async fn live_dry_run_produces_a_real_conversion() {
        let project =
            std::env::var("BIFROST_TEST_PROJECT").unwrap_or_else(|_| "Contoso-Payments".into());
        let pipeline = std::env::var("BIFROST_TEST_PIPELINE").unwrap_or_else(|_| "11".into());
        let imp = DockerImporter::from_env(&project).unwrap();
        let dry = imp.dry_run(&pipeline).await.expect("dry_run succeeds");
        eprintln!(
            "{project}/{pipeline}: converted={:.0}% gaps={} source={}B converted={}B",
            dry.converted_ratio * 100.0,
            dry.gaps.len(),
            dry.source_yaml.len(),
            dry.converted_yaml.len(),
        );
        assert!(!dry.converted_yaml.is_empty(), "has converted workflow");
        assert!(!dry.source_yaml.is_empty(), "has ADO source");
        assert!(!dry.gaps.is_empty(), "found gaps (DownloadSecureFile etc.)");
    }
}
