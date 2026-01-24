//! CLI command definitions and dispatch.
//!
//! This module defines the command-line interface for Tern, including all
//! available commands and their argument structures.

pub mod build;
pub mod check;
mod colors;
pub mod compile;
pub mod generate;
pub mod history;
pub mod import;
pub mod init;
pub mod inspect;
pub mod migrate;
pub mod print_migrations;
pub mod record;
pub mod schema;
pub mod show;
pub mod status;
pub mod up;
pub mod verify;
pub mod version;

pub use colors::EnableColors;

use anstream::println;
use clap::{Parser, Subcommand};
use miette::IntoDiagnostic;
use serde::Serialize;
use tracing::level_filters::LevelFilter;

use crate::db::state::{LocalFileBackend, StateBackend};

// =============================================================================
// Shared Helpers (used by command modules)
// =============================================================================

/// Loads or creates a state backend, preferring the specified path or the default location.
pub fn load_backend(state_path: Option<&std::path::Path>) -> LocalFileBackend {
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
#[allow(dead_code)]
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

// =============================================================================
// CLI Definition
// =============================================================================

#[derive(Debug, Default, Clone, clap::ValueEnum)]
pub enum LogFormat {
    #[default]
    Text,
    JSON,
}

#[derive(Debug, Parser, Clone)]
#[command(name = "tern")]
#[command(about = "A database migration tool written in Rust")]
#[command(version)]
pub struct Cli {
    #[arg(long, env = "LOG_LEVEL", default_value = "info", global = true)]
    pub log_level: LevelFilter,

    #[arg(long, env = "LOG_FORMAT", default_value = "text", global = true)]
    pub log_format: LogFormat,

    #[arg(long, default_value = "auto", global = true)]
    pub enable_colors: EnableColors,

    #[command(subcommand)]
    pub cmd: Option<CliCommand>,
}

#[derive(Debug, Subcommand, Clone)]
pub enum CliCommand {
    /// Print the CLI version and exit
    Version(version::Version),

    /// Initialize a new Tern project with state backend
    ///
    /// Creates the .tern/ directory structure and optionally captures
    /// the current database schema as the baseline migration.
    Init(init::Init),

    /// Show state backend status
    ///
    /// Displays information about the current state backend, including
    /// migration count, state hash, and schema summary.
    Status(status::Status),

    /// Verification commands
    ///
    /// Commands for verifying schema consistency and migration state.
    #[command(subcommand)]
    Check(CheckAction),

    /// Import schema changes from a live database
    ///
    /// Connects to a database, compares it to the current migration state,
    /// and generates a migration for any differences found.
    Import(import::Import),

    /// Generate migration from schema changes
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and generates a migration that would transform the schema.
    Generate(generate::Generate),

    /// Migration commands (up/down)
    ///
    /// Commands for running and reverting database migrations.
    #[command(subcommand)]
    Migrate(migrate::MigrateAction),

    /// Run pending migrations against a database (alias for 'migrate up')
    ///
    /// Connects to a database and applies all migrations that haven't been
    /// applied yet. Each migration runs in its own transaction.
    Up(migrate::Up),

    /// Revert the most recently applied migration (alias for 'migrate down')
    ///
    /// Connects to a database and reverts the last applied migration.
    /// Only one migration is reverted at a time for safety.
    Down(migrate::Down),

    /// List migration history
    ///
    /// Shows the ordered list of migrations in the state backend.
    History(history::History),

    /// Show details of a specific migration
    ///
    /// Displays detailed information about a migration, including
    /// its operations, state hashes, and breaking changes.
    Show(show::Show),

    /// Verify state backend matches database
    ///
    /// Compares the state backend to the live database schema and
    /// reports any drift (manual changes not captured in migrations).
    Verify(verify::Verify),

    /// Verify migration chain integrity
    ///
    /// Checks that all migrations in the state backend have valid
    /// parent-child relationships (chain integrity).
    VerifyChain(verify::VerifyChain),

    /// Schema management commands
    ///
    /// Commands for working with the schema DDL file, which forms the
    /// foundation of the model-first migration workflow.
    #[command(subcommand)]
    Schema(SchemaAction),

    // =========================================================================
    // Hidden/Deprecated Commands
    // =========================================================================
    /// [DEPRECATED] Print the SQL migrations needed to recreate the current database schema
    ///
    /// Connects to the specified database and generates the SQL DDL statements
    /// that would recreate all objects in the specified schema from scratch.
    /// This is useful for creating an initial baseline migration or for
    /// understanding the current state of a database schema.
    #[command(hide = true)]
    PrintMigrations(print_migrations::PrintMigrations),

    /// [DEPRECATED] Compile a migration to source code (use 'tern import' + 'tern build' instead)
    ///
    /// Compares the state backend to the live database and generates
    /// migration source code for the detected changes.
    #[command(hide = true)]
    Compile(compile::Compile),

    /// [DEPRECATED] Build a migration executable or OCI image
    ///
    /// Compares the state backend to the live database and builds a
    /// standalone migration artifact (binary executable or OCI container image).
    #[command(hide = true)]
    Build(build::Build),

    /// [DEPRECATED] Record a migration as applied
    ///
    /// Marks a migration as applied in the state backend without
    /// executing it. Useful for synchronizing state backends.
    #[command(hide = true)]
    Record(record::Record),

    /// [DEPRECATED] Inspect a compiled migration file
    ///
    /// Examines a migration source file or JSON file and displays
    /// its contents and metadata.
    #[command(hide = true)]
    Inspect(inspect::Inspect),
}

/// Verification-related subcommands.
#[derive(Debug, Subcommand, Clone)]
pub enum CheckAction {
    /// Verify that schema.sql is consistent with migrations
    ///
    /// Compares the schema defined in `.tern/schema.sql` with the schema
    /// that would result from replaying all migrations. Reports any drift.
    Schema(check::CheckSchema),
}

/// Schema-related subcommands.
///
/// These commands support the model-first migration workflow by allowing
/// users to work with the schema as SQL DDL.
#[derive(Debug, Subcommand, Clone)]
pub enum SchemaAction {
    /// Export the current schema as SQL DDL
    ///
    /// Generates a schema.sql file containing SQL DDL statements that
    /// would recreate the current schema from scratch. This file is
    /// useful for viewing the schema, documentation, and the model-first
    /// migration workflow.
    Export(schema::export::Export),

    /// [DEPRECATED] Show diff between current state and edited schema.sql (use 'tern check schema' instead)
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and displays what operations would be needed to transform the schema.
    /// This is the preview step before generating a migration.
    #[command(hide = true)]
    Diff(schema::diff::Diff),

    /// [DEPRECATED] Generate migration from schema changes (use 'tern generate' instead)
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and generates a migration that would transform the schema. This is
    /// the core of the model-first migration workflow.
    #[command(hide = true)]
    Migrate(schema::migrate::Migrate),
}

impl CliCommand {
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            CliCommand::Version(args) => args.dispatch().await,
            CliCommand::Init(args) => args.dispatch().await,
            CliCommand::Status(args) => args.dispatch().await,
            CliCommand::Check(action) => action.dispatch().await,
            CliCommand::Import(args) => args.dispatch().await,
            CliCommand::Generate(args) => args.dispatch().await,
            CliCommand::Migrate(action) => action.dispatch().await,
            CliCommand::Up(args) => args.dispatch().await,
            CliCommand::Down(args) => args.dispatch().await,
            CliCommand::History(args) => args.dispatch().await,
            CliCommand::Show(args) => args.dispatch().await,
            CliCommand::Verify(args) => args.dispatch().await,
            CliCommand::VerifyChain(args) => args.dispatch().await,
            CliCommand::Schema(action) => action.dispatch().await,
            CliCommand::PrintMigrations(args) => args.dispatch().await,
            CliCommand::Compile(args) => args.dispatch().await,
            CliCommand::Build(args) => args.dispatch().await,
            CliCommand::Record(args) => args.dispatch().await,
            CliCommand::Inspect(args) => args.dispatch().await,
        }
    }
}

impl CheckAction {
    /// Dispatch check subcommands.
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            CheckAction::Schema(args) => args.dispatch().await,
        }
    }
}

impl SchemaAction {
    /// Dispatch schema subcommands.
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            SchemaAction::Export(args) => args.dispatch().await,
            SchemaAction::Diff(args) => args.dispatch().await,
            SchemaAction::Migrate(args) => args.dispatch().await,
        }
    }
}
