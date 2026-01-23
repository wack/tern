//! CLI command definitions and dispatch.
//!
//! This module defines the command-line interface for Tern, including all
//! available commands and their argument structures.

pub mod build;
mod colors;
pub mod compile;
pub mod history;
pub mod init;
pub mod inspect;
pub mod record;
pub mod schema;
pub mod show;
pub mod status;
pub mod verify;

pub use colors::EnableColors;

use anstream::{eprintln, println};
use clap::{Parser, Subcommand};
use miette::{Context, IntoDiagnostic};
use serde::Serialize;
use tracing::level_filters::LevelFilter;

use crate::db::diff::breaking::analyze_breaking_changes;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::query::{PostgresCatalog, diff_from_empty};
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::db::{self};

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
    Version,

    /// Print the SQL migrations needed to recreate the current database schema
    ///
    /// Connects to the specified database and generates the SQL DDL statements
    /// that would recreate all objects in the specified schema from scratch.
    /// This is useful for creating an initial baseline migration or for
    /// understanding the current state of a database schema.
    PrintMigrations {
        /// PostgreSQL connection string (e.g., postgres://user:pass@localhost/db)
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// The schema to generate migrations for
        #[arg(long, default_value = "public")]
        schema: String,
    },

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

    /// Compile a migration to source code
    ///
    /// Compares the state backend to the live database and generates
    /// migration source code for the detected changes.
    Compile(compile::Compile),

    /// Build a migration executable or OCI image
    ///
    /// Compares the state backend to the live database and builds a
    /// standalone migration artifact (binary executable or OCI container image).
    Build(build::Build),

    /// List migration history
    ///
    /// Shows the ordered list of migrations in the state backend.
    History(history::History),

    /// Show details of a specific migration
    ///
    /// Displays detailed information about a migration, including
    /// its operations, state hashes, and breaking changes.
    Show(show::Show),

    /// Record a migration as applied
    ///
    /// Marks a migration as applied in the state backend without
    /// executing it. Useful for synchronizing state backends.
    Record(record::Record),

    /// Inspect a compiled migration file
    ///
    /// Examines a migration source file or JSON file and displays
    /// its contents and metadata.
    Inspect(inspect::Inspect),

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

    /// Show diff between current state and edited schema.sql
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and displays what operations would be needed to transform the schema.
    /// This is the preview step before generating a migration.
    Diff(schema::diff::Diff),

    /// Generate migration from schema changes
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and generates a migration that would transform the schema. This is
    /// the core of the model-first migration workflow.
    Migrate(schema::migrate::Migrate),
}

impl CliCommand {
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            CliCommand::Version => {
                println!("tern {}", env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            CliCommand::PrintMigrations {
                database_url,
                schema,
            } => print_migrations(&database_url, &schema).await,
            CliCommand::Init(args) => {
                init::run_init(args.from, &args.schema, args.path.as_deref()).await
            }
            CliCommand::Status(args) => status::run_status(args.format, args.path.as_deref()).await,
            CliCommand::Compile(args) => {
                compile::run_compile(
                    &args.database_url,
                    &args.schema,
                    args.output,
                    &args.description,
                    &args.target,
                    args.record,
                    args.dry_run,
                    args.show_sql,
                    args.format,
                    args.state_path.as_deref(),
                    args.allow_drift,
                )
                .await
            }
            CliCommand::Build(args) => {
                build::run_build(
                    &args.database_url,
                    &args.schema,
                    args.output,
                    &args.description,
                    &args.target,
                    &args.package_format,
                    args.record,
                    args.format,
                    args.state_path.as_deref(),
                    args.allow_drift,
                )
                .await
            }
            CliCommand::History(args) => {
                history::run_history(args.format, args.limit, args.path.as_deref()).await
            }
            CliCommand::Show(args) => {
                show::run_show(&args.migration_id, args.format, args.path.as_deref()).await
            }
            CliCommand::Record(args) => {
                record::run_record(
                    args.migration_id.as_deref(),
                    args.migration_file,
                    args.format,
                    args.path.as_deref(),
                )
                .await
            }
            CliCommand::Inspect(args) => inspect::run_inspect(args.path, args.format).await,
            CliCommand::Verify(args) => {
                verify::run_verify(
                    &args.database_url,
                    &args.schema,
                    args.format,
                    args.path.as_deref(),
                )
                .await
            }
            CliCommand::VerifyChain(args) => {
                verify::run_verify_chain(args.format, args.path.as_deref()).await
            }
            CliCommand::Schema(action) => action.dispatch().await,
        }
    }
}

impl SchemaAction {
    /// Dispatch schema subcommands.
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            SchemaAction::Export(args) => {
                schema::run_schema_export(args.output, args.path.as_deref(), args.format).await
            }
            SchemaAction::Diff(args) => {
                schema::run_schema_diff(args.schema, args.path.as_deref(), args.format).await
            }
            SchemaAction::Migrate(args) => {
                schema::run_schema_migrate(
                    args.schema,
                    &args.description,
                    args.path.as_deref(),
                    args.format,
                    args.dry_run,
                    args.force,
                )
                .await
            }
        }
    }
}

/// Connects to the database and prints migration SQL for the specified schema.
async fn print_migrations(database_url: &str, schema_name: &str) -> miette::Result<()> {
    // Connect to the database
    let client = db::connect(database_url)
        .await
        .into_diagnostic()
        .wrap_err("Failed to connect to database")?;

    // Create catalog adapter
    let catalog = PostgresCatalog::new(&client);

    // Get diff from empty schema to current state
    let diff = diff_from_empty(&catalog, schema_name)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to load schema '{}'", schema_name))?;

    // Analyze for breaking changes
    let analysis = analyze_breaking_changes(&diff);
    if !analysis.is_safe() {
        eprintln!("WARNING: {} breaking change(s) detected:", analysis.len());
        for change in analysis.iter() {
            eprintln!("  [{}] {}", change.mitigation.as_str(), change.description);
        }
        eprintln!();
    }

    // Create migration plan
    let plan = MigrationPlan::from_diff(&diff);

    // Render to SQL
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);

    // Print the SQL
    println!("{}", script.to_sql());

    Ok(())
}
