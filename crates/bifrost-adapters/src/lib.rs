//! Source adapters and the Importer wrapper.
//!
//! Defines the [`SourceAdapter`] trait (ADO is the first implementation) and the
//! wrapper around the official `gh actions-importer` Docker image. We wrap the
//! official tools; we never reimplement their conversion logic.

pub mod ado_auth;
pub mod azure_devops;
pub mod bamboo;
pub mod baseline;
pub mod bitbucket;
pub mod board;
pub mod circleci;
pub mod convert;
pub mod docker_importer;
pub mod github_auth;
pub mod gitlab;
pub mod importer;
pub mod jenkins;
pub mod orchestrator;
pub mod publisher;
pub mod source;
pub mod source_factory;
pub mod travis;
pub mod validate;

/// Centralized fixture loader for tests (#17). Test-only.
#[cfg(test)]
mod test_fixtures;

pub use ado_auth::{AdoAuth, EntraAuth, PatAuth};
pub use azure_devops::AzureDevOpsAdapter;
pub use bamboo::BambooAdapter;
pub use baseline::{
    AzureDevOpsBaseline, BaselineError, BaselineRequest, BaselineSource, MockBaselineSource,
};
pub use bitbucket::BitbucketAdapter;
pub use board::{
    BoardProvisioner, GitHubBoardProvisioner, MockBoardProvisioner, ProvisionAction,
    ProvisionError, ProvisionOutcome, ProvisionResult, ProvisionTarget, ProvisionedField,
    ProvisionedIssue, ProvisionedOption, StatusSyncTarget,
};
pub use circleci::CircleCiAdapter;
pub use convert::{convert_pipeline, ConversionError, ConversionOutcome};
pub use docker_importer::DockerImporter;
pub use github_auth::{
    github_token_from_env, AuthError, GitHubAppAuth, GitHubAuth, StaticTokenAuth,
};
pub use gitlab::GitLabAdapter;
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
pub use source_factory::source_adapter_from;
pub use travis::TravisAdapter;
pub use validate::{
    declared_outputs, GitHubRunCollector, GitHubSandboxTrigger, MockRunCollector,
    MockSandboxTrigger, RunArtifact, RunCollector, RunJob, RunQuery, RunResult, SandboxTrigger,
    TriggerRequest, TriggerResult, ValidateError,
};
