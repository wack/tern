//! CLI command handlers for Tern operations.
//!
//! This module contains the implementation of all CLI subcommands for managing
//! migrations and state backends.

pub mod compile;
pub mod history;
pub mod init;
pub mod inspect;
pub mod record;
pub mod schema;
pub mod show;
pub mod status;
pub mod verify;

pub use compile::run_compile;
pub use history::run_history;
pub use init::run_init;
pub use inspect::run_inspect;
pub use record::run_record;
pub use schema::run_schema_export;
pub use show::run_show;
pub use status::run_status;
pub use verify::{run_verify, run_verify_chain};

// =============================================================================
// Shared Helpers
// =============================================================================

pub use crate::db::state::LocalFileBackend;
use crate::db::state::StateBackend;
use miette::IntoDiagnostic;
use std::path::Path;

/// Loads or creates a state backend, preferring the specified path or the default location.
pub fn load_backend(state_path: Option<&Path>) -> LocalFileBackend {
    state_path
        .map(LocalFileBackend::at_path)
        .unwrap_or_else(LocalFileBackend::default_location)
}

/// Ensures the backend is initialized, returning an error if not.
pub async fn ensure_backend_initialized(backend: &LocalFileBackend) -> miette::Result<()> {
    if !backend.is_initialized().await.into_diagnostic()? {
        return Err(miette::miette!(
            "State backend not initialized at {}\n\nRun 'tern init' to initialize a new project.",
            backend.root().display()
        ));
    }
    Ok(())
}

// =============================================================================
// Shared Output Formatting
// =============================================================================

use serde::Serialize;

/// Output format for CLI commands.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for machine consumption.
    Json,
    /// SQL output (for applicable commands).
    Sql,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => write!(f, "text"),
            Self::Json => write!(f, "json"),
            Self::Sql => write!(f, "sql"),
        }
    }
}

/// Prints output in the specified format.
pub fn print_output<T: Serialize + std::fmt::Display>(output: &T, format: OutputFormat) {
    match format {
        OutputFormat::Text => println!("{}", output),
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(output).expect("failed to serialize output")
            );
        }
        OutputFormat::Sql => println!("{}", output),
    }
}

/// Prints a JSON-serializable value.
pub fn print_json<T: Serialize>(output: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(output).expect("failed to serialize output")
    );
}
