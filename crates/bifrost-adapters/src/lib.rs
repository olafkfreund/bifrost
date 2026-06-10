//! Source adapters and the Importer wrapper.
//!
//! Defines the [`SourceAdapter`] trait (ADO is the first implementation) and the
//! wrapper around the official `gh actions-importer` Docker image. We wrap the
//! official tools; we never reimplement their conversion logic.

pub mod importer;
pub mod source;

pub use source::{AdapterError, MockSourceAdapter, SourceAdapter};
