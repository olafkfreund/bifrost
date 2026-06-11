//! Source adapters and the Importer wrapper.
//!
//! Defines the [`SourceAdapter`] trait (ADO is the first implementation) and the
//! wrapper around the official `gh actions-importer` Docker image. We wrap the
//! official tools; we never reimplement their conversion logic.

pub mod ado_auth;
pub mod azure_devops;
pub mod baseline;
pub mod convert;
pub mod docker_importer;
pub mod github_auth;
pub mod importer;
pub mod jenkins;
pub mod orchestrator;
pub mod publisher;
pub mod source;
pub mod validate;

/// Centralized fixture loader for tests (#17). Test-only.
#[cfg(test)]
mod test_fixtures;

pub use ado_auth::{AdoAuth, EntraAuth, PatAuth};
pub use azure_devops::AzureDevOpsAdapter;
pub use baseline::{
    AzureDevOpsBaseline, BaselineError, BaselineRequest, BaselineSource, MockBaselineSource,
};
pub use convert::{convert_pipeline, ConversionError, ConversionOutcome};
pub use docker_importer::DockerImporter;
pub use github_auth::{
    github_token_from_env, AuthError, GitHubAppAuth, GitHubAuth, StaticTokenAuth,
};
pub use importer::{
    parse_audit_summary, parse_dry_run, parse_forecast, Forecast, Importer, ImporterError,
    MockImporter,
};
pub use jenkins::JenkinsAdapter;
pub use orchestrator::{
    audit_org, audit_portfolio, merge_portfolios, AuditConfig, OrchestrationError,
};
pub use publisher::{
    CommitRequest, CommitResult, GitHubPublisher, MockPublisher, PublishError, Publisher,
};
pub use source::{AdapterError, MockSourceAdapter, SourceAdapter};
pub use validate::{
    declared_outputs, GitHubRunCollector, GitHubSandboxTrigger, MockRunCollector,
    MockSandboxTrigger, RunArtifact, RunCollector, RunJob, RunQuery, RunResult, SandboxTrigger,
    TriggerRequest, TriggerResult, ValidateError,
};
