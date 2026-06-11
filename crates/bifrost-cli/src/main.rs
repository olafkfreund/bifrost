//! Bifrost CLI entrypoint.

use anyhow::Result;
use bifrost_adapters::{
    audit_portfolio, AuditConfig, AzureDevOpsAdapter, DockerImporter, Importer, SourceAdapter,
};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "bifrost",
    about = "ADO → GitHub Actions migration — audit, convert, report"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Audit an Azure DevOps project. Reads AZDO_ORG_URL + AZDO_PAT (+ GITHUB_TOKEN
    /// for `--json`) from the env. Default output is a human-readable inventory;
    /// `--json` runs the Importer audit and emits the computed portfolio the portal
    /// renders.
    Audit {
        /// ADO project name.
        #[arg(long, default_value = "SARC")]
        project: String,
        /// Emit the computed portfolio as JSON (runs the Importer via Docker).
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Audit { project, json } if json => audit_json(&project).await,
        Command::Audit { project, .. } => audit_inventory(&project).await,
    }
}

/// Human-readable read-side inventory (ADO REST only, no Docker).
async fn audit_inventory(project: &str) -> Result<()> {
    let adapter = AzureDevOpsAdapter::from_env(project)?;
    let projects = adapter.discover().await?;
    let pipelines = adapter.enumerate_pipelines().await?;
    let connections = adapter.fetch_service_connections().await?;
    let groups = adapter.fetch_variable_groups().await?;

    println!("❄️ Bifrost audit — project '{project}'\n");
    println!("Projects ({}):", projects.len());
    for p in &projects {
        println!("  - {}", p.name);
    }
    println!("\nPipelines ({}):", pipelines.len());
    for p in &pipelines {
        println!("  - {:<28} [{:?}]", p.name, p.classification);
    }
    println!(
        "\nService connections ({}) — OIDC-federation candidates:",
        connections.len()
    );
    for c in &connections {
        println!("  - {:<40} {}", c.name, c.kind);
    }
    let secret_count: usize = groups
        .iter()
        .flat_map(|g| &g.variables)
        .filter(|v| v.is_secret)
        .count();
    println!(
        "\nVariable groups ({}) — {} secret value(s) to provision as GitHub secrets:",
        groups.len(),
        secret_count
    );
    for g in &groups {
        let secrets = g.variables.iter().filter(|v| v.is_secret).count();
        println!(
            "  - {:<24} {} vars ({} secret)",
            g.name,
            g.variables.len(),
            secrets
        );
    }
    println!("\nFor the computed portfolio (runs the Importer audit), use --json.");
    Ok(())
}

/// Computed portfolio as JSON — the shape `bifrost-api` serves and the portal renders.
async fn audit_json(project: &str) -> Result<()> {
    let adapter = AzureDevOpsAdapter::from_env(project)?;
    let importer = DockerImporter::from_env(project)?;
    let version = importer
        .version()
        .await
        .unwrap_or_else(|_| "unknown".into());
    let importer_image_digest = importer.image_digest().await.unwrap_or_default();

    let org = std::env::var("AZDO_ORG_URL").unwrap_or_default();
    let config = AuditConfig {
        org: org.rsplit('/').next().unwrap_or("unknown").to_string(),
        importer_version: version,
        importer_image_digest,
        ado2gh_version: "n/a".into(),
        air_gap: false,
        generated_at: now_iso8601(),
    };

    let portfolio = audit_portfolio(&adapter, &importer, config).await?;
    println!("{}", serde_json::to_string_pretty(&portfolio)?);
    Ok(())
}

/// Current UTC timestamp via coreutils `date` (avoids a chrono dependency).
fn now_iso8601() -> String {
    std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}
