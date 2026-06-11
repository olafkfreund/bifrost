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
pub mod conversion;
pub mod gap;
pub mod ingestion;
pub mod model;
pub mod parity;
pub mod proposal;
pub mod risk;
pub mod runbook;

pub use assemble::{assemble_workflow, GapFill};
pub use attest::{
    MigrationAttestation, MigrationPredicate, Signature, SignedMigrationAttestation,
    MIGRATION_PREDICATE_TYPE,
};
pub use audit::{AuditCounts, AuditSummary, ManualTask, ManualTaskKind, UnsupportedStep};
pub use audit_log::{AuditEvent, AuditLog};
pub use conversion::{build_pipeline, pipeline_from_dry_run, signals_from_dry_run, PipelineMeta};
pub use gap::{DryRunResult, Gap, GapKind};
pub use ingestion::{
    PipelineDefinition, Project, ServiceConnection, SourcePipeline, TaskKind, TaskUsage,
    VariableGroup, VariableRef,
};
pub use model::{
    Classification, Pipeline, Portfolio, PortfolioSummary, PortfolioTotals, ProposalStatus,
    RiskBand, RiskFactor,
};
pub use parity::{
    compare as compare_parity, ParityReport, ParityVerdict, RunFacts, SetDiff, SMOKE_PARITY_CAVEAT,
};
pub use proposal::{is_legal_transition, Attestation, Proposal, ProposalError};
pub use risk::{assess, band_for_score, RiskAssessment, RiskSignals};
pub use runbook::{gap_is_manual, ChecklistCategory, ChecklistItem, Runbook};
