//! Docker-backed [`Importer`] — drives the official `ghcr.io/actions-importer/cli`
//! image directly (no gh extension, which is read-only under home-manager).
//!
//! Tokens are passed by forwarding host env vars (`-e NAME`, value set on the
//! child process) so they never appear in the process argv. Audit output is
//! parsed by the validated [`parse_audit_summary`].

use bifrost_core::{AuditSummary, DryRunResult};
use tokio::process::Command;

use crate::importer::{parse_audit_summary, Importer, ImporterError};

const DEFAULT_IMAGE: &str = "ghcr.io/actions-importer/cli:latest";

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
        let (gh, pat) = Self::creds()?;
        let out_dir = std::env::temp_dir().join("bifrost-importer-audit");
        let _ = tokio::fs::remove_dir_all(&out_dir).await; // best-effort clean
        tokio::fs::create_dir_all(&out_dir)
            .await
            .map_err(|e| Self::err(format!("could not create output dir: {e}")))?;

        // Tokens forwarded via `-e NAME` (values set on the child env), never argv.
        // Capture output (don't inherit) so the importer's progress logs can't
        // pollute our stdout — callers may pipe a clean JSON portfolio from it.
        let out = Command::new("docker")
            .args(["run", "--rm"])
            .args(["-e", "GITHUB_ACCESS_TOKEN", "-e", "GITHUB_INSTANCE_URL"])
            .args([
                "-e",
                "AZURE_DEVOPS_ACCESS_TOKEN",
                "-e",
                "AZURE_DEVOPS_INSTANCE_URL",
            ])
            .arg("-v")
            .arg(format!("{}:/data", out_dir.display()))
            .args(["-w", "/data", &self.image])
            .args(["audit", "azure-devops", "--output-dir", "report"])
            .args(["--azure-devops-organization", &self.organization])
            .args(["--azure-devops-project", &self.project])
            .env("GITHUB_ACCESS_TOKEN", gh)
            .env("GITHUB_INSTANCE_URL", "https://github.com")
            .env("AZURE_DEVOPS_ACCESS_TOKEN", pat)
            .env("AZURE_DEVOPS_INSTANCE_URL", "https://dev.azure.com")
            .output()
            .await
            .map_err(|e| Self::err(format!("docker spawn failed: {e}")))?;
        // Surface the importer's own logs on stderr for visibility.
        eprint!("{}", String::from_utf8_lossy(&out.stdout));
        eprint!("{}", String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            return Err(Self::err("importer audit failed (see docker output)"));
        }

        let summary_path = out_dir.join("report").join("audit_summary.md");
        let md = tokio::fs::read_to_string(&summary_path)
            .await
            .map_err(|e| Self::err(format!("could not read audit_summary.md: {e}")))?;
        Ok(parse_audit_summary(&md))
    }

    async fn dry_run(&self, _pipeline_id: &str) -> Result<DryRunResult, ImporterError> {
        // The real dry-run log format hasn't been captured/validated yet; the
        // audit already yields per-pipeline conversion stats for single-pipeline
        // orgs. Tracked as a follow-up rather than shipping an unvalidated parser.
        Err(Self::err(
            "dry_run via Docker not yet wrapped — use audit()",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_org_from_url() {
        assert_eq!(org_from_url("https://dev.azure.com/contoso"), "contoso");
        assert_eq!(org_from_url("https://dev.azure.com/contoso/"), "contoso");
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
}
