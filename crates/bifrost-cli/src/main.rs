//! Bifrost CLI entrypoint.

use anyhow::Result;
use bifrost_adapters::{AzureDevOpsAdapter, SourceAdapter};
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
    /// Audit an Azure DevOps project: discover pipelines, connections, and
    /// variable groups (names only). Reads AZDO_ORG_URL + AZDO_PAT from the env.
    Audit {
        /// ADO project name.
        #[arg(long, default_value = "SARC")]
        project: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Audit { project } => audit(&project).await,
    }
}

async fn audit(project: &str) -> Result<()> {
    let adapter = AzureDevOpsAdapter::from_env(project)?;

    let projects = adapter.discover().await?;
    let pipelines = adapter.enumerate_pipelines().await?;
    let connections = adapter.fetch_service_connections().await?;
    let groups = adapter.fetch_variable_groups().await?;

    println!("🌈 Bifrost audit — project '{project}'\n");

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

    println!("\nNote: risk scoring needs the Importer dry-run (see README) — this is the");
    println!("read-side inventory. Secret *values* are never fetched, only names + flags.");
    Ok(())
}
