//! Bifrost core domain.
//!
//! Home of the domain types, the job/proposal state machine, and the
//! deterministic risk model. Adapters, the LLM layer, and the API all depend
//! on this crate; it depends on none of them.
//!
//! Risk scoring lives here and is computed from explainable factors — never
//! from the LLM (see the implementation plan, §6).

pub mod model;
pub mod risk;

pub use model::{
    Classification, Pipeline, Portfolio, PortfolioSummary, PortfolioTotals, ProposalStatus,
    RiskBand, RiskFactor,
};
pub use risk::band_for_score;
