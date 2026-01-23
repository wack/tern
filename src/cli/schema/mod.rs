//! Schema-related CLI commands.
//!
//! This module implements commands for working with the schema DDL file,
//! which is the foundation of the model-first migration workflow.

pub mod diff;
pub mod export;
pub mod migrate;

pub use diff::run_schema_diff;
pub use export::run_schema_export;
pub use migrate::run_schema_migrate;
