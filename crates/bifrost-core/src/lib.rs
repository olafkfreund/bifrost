//! Bifrost core domain.
//!
//! Home of the domain types, the job/proposal state machine, and the
//! deterministic risk model. Adapters, the LLM layer, and the API all depend
//! on this crate; it depends on none of them.
//!
//! Risk scoring lives here and is computed from explainable factors — never
//! from the LLM (see the implementation plan, §6).

pub mod assemble;
pub mod attest;
pub mod audit;
pub mod audit_log;
pub mod completeness;
pub mod connection;
pub mod conversion;
pub mod forecast;
pub mod gap;
pub mod identity;
pub mod ingestion;
pub mod model;
pub mod parity;
pub mod program;
pub mod proposal;
pub mod readiness;
pub mod report;
pub mod risk;
pub mod runbook;
pub mod source_stats;

pub use assemble::{assemble_workflow, GapFill};
pub use attest::{
    AuditPack, AuditPackSummary, MigrationAttestation, MigrationPredicate, Signature,
    SignedAuditPack, SignedMigrationAttestation, MIGRATION_PREDICATE_TYPE,
};
pub use audit::{AuditCounts, AuditSummary, ManualTask, ManualTaskKind, UnsupportedStep};
pub use audit_log::{AuditEvent, AuditLog};
pub use completeness::{completeness, CategoryStatus, CompletenessRow};
pub use connection::{ConfigAction, ConfigEvent, Connection, ConnectionKind, SecretRef};
pub use conversion::{build_pipeline, pipeline_from_dry_run, signals_from_dry_run, PipelineMeta};
pub use forecast::{forecast, CapacityForecast, Forecast, ProjectForecast, RunnerRate};
pub use gap::{DryRunResult, Gap, GapKind};
pub use identity::{Identity, Role};
pub use ingestion::{
    PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskKind, TaskUsage,
    VariableGroup, VariableRef,
};
pub use model::{
    Classification, Pipeline, Portfolio, PortfolioAudit, PortfolioSummary, PortfolioTotals,
    ProposalStatus, RiskBand, RiskFactor,
};
pub use parity::{
    compare as compare_parity, ParityReport, ParityVerdict, RunFacts, SetDiff, SMOKE_PARITY_CAVEAT,
};
pub use program::{program, WavePlan};
pub use proposal::{is_legal_transition, Attestation, Proposal, ProposalError};
pub use readiness::{readiness, ReadinessItem, ReadinessStatus};
pub use report::{report_markdown, report_stats, ReportStats};
pub use risk::{assess, band_for_score, RiskAssessment, RiskSignals};
pub use runbook::{gap_is_manual, ChecklistCategory, ChecklistItem, Runbook};
pub use source_stats::{source_stats, ProjectStat, SourceStats};
