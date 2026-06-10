//! LLM provider layer.
//!
//! Defines the `LlmProvider` trait (structured JSON output) plus routing and
//! air-gap mode. Orchestration code calls only this trait, never a vendor SDK
//! directly. The model fills gaps and explains; it never produces the risk
//! score and never converts a pipeline from scratch.
