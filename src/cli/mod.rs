//! CLI command definitions and dispatch.
//!
//! This module defines the command-line interface for Tern, including all
//! available commands and their argument structures.

mod colors;
pub mod commands;

pub use colors::EnableColors;
pub use commands::OutputFormat;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::{Context, IntoDiagnostic};
use tracing::level_filters::LevelFilter;

use crate::db::diff::breaking::analyze_breaking_changes;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::query::{PostgresCatalog, diff_from_empty};
use crate::db::{self};

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
    Init {
        /// PostgreSQL connection string to initialize from (captures current schema)
        #[arg(long, env = "DATABASE_URL")]
        from: Option<String>,

        /// The database schema to capture
        #[arg(long, default_value = "public")]
        schema: String,

        /// Path to create the state directory (default: current directory)
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Show state backend status
    ///
    /// Displays information about the current state backend, including
    /// migration count, state hash, and schema summary.
    Status {
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Compile a migration to source code
    ///
    /// Compares the state backend to the live database and generates
    /// migration source code for the detected changes.
    Compile {
        /// PostgreSQL connection string
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// The database schema to compare
        #[arg(long, default_value = "public")]
        schema: String,

        /// Output path for the generated source code
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Migration description
        #[arg(long)]
        description: String,

        /// Target platform for executable (native, x86_64-linux-gnu, etc.)
        #[arg(long, default_value = "native")]
        target: String,

        /// Record the migration to the state backend
        #[arg(long)]
        record: bool,

        /// Preview without writing files
        #[arg(long)]
        dry_run: bool,

        /// Include SQL statements in output
        #[arg(long)]
        show_sql: bool,

        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        state_path: Option<PathBuf>,
    },

    /// Build a migration executable or OCI image
    ///
    /// Compares the state backend to the live database and builds a
    /// standalone migration artifact (binary executable or OCI container image).
    Build {
        /// PostgreSQL connection string
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// The database schema to compare
        #[arg(long, default_value = "public")]
        schema: String,

        /// Output path for the built artifact
        #[arg(short, long)]
        output: PathBuf,

        /// Migration description
        #[arg(long)]
        description: String,

        /// Target platform (native, x86_64-linux-gnu, x86_64-linux-musl, x86_64-macos, aarch64-macos, x86_64-windows)
        #[arg(long, default_value = "native")]
        target: String,

        /// Output format: binary (standalone executable) or oci (OCI container image)
        #[arg(long, default_value = "binary")]
        package_format: String,

        /// Record the migration to the state backend
        #[arg(long)]
        record: bool,

        /// CLI output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        state_path: Option<PathBuf>,
    },

    /// List migration history
    ///
    /// Shows the ordered list of migrations in the state backend.
    History {
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Maximum number of migrations to show (from most recent)
        #[arg(long)]
        limit: Option<usize>,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Show details of a specific migration
    ///
    /// Displays detailed information about a migration, including
    /// its operations, state hashes, and breaking changes.
    Show {
        /// Migration ID (full hex or prefix)
        migration_id: String,

        /// Output format (text, json, or sql)
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Record a migration as applied
    ///
    /// Marks a migration as applied in the state backend without
    /// executing it. Useful for synchronizing state backends.
    Record {
        /// Migration ID to record (if already in backend)
        #[arg(long)]
        migration_id: Option<String>,

        /// Path to a migration JSON file
        #[arg(long)]
        migration_file: Option<PathBuf>,

        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Inspect a compiled migration file
    ///
    /// Examines a migration source file or JSON file and displays
    /// its contents and metadata.
    Inspect {
        /// Path to the file to inspect (.json or .rs)
        path: PathBuf,

        /// Output format (text, json, or sql)
        #[arg(long, default_value = "text")]
        format: OutputFormat,
    },

    /// Verify state backend matches database
    ///
    /// Compares the state backend to the live database schema and
    /// reports any drift (manual changes not captured in migrations).
    Verify {
        /// PostgreSQL connection string
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// The database schema to verify
        #[arg(long, default_value = "public")]
        schema: String,

        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Verify migration chain integrity
    ///
    /// Checks that all migrations in the state backend have valid
    /// parent-child relationships (chain integrity).
    VerifyChain {
        /// Output format
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

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
    Export {
        /// Output path for the schema file (default: .tern/schema.sql, use "-" for stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output format (text shows summary, sql shows DDL, json includes metadata)
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Show diff between current state and edited schema.sql
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and displays what operations would be needed to transform the schema.
    /// This is the preview step before generating a migration.
    #[cfg(feature = "pglite")]
    Diff {
        /// Path to the edited schema file (default: .tern/schema.sql)
        #[arg(short, long)]
        schema: Option<PathBuf>,

        /// Output format (text shows summary, sql shows migration SQL, json includes metadata)
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Generate migration from schema changes
    ///
    /// Compares the current migration state to the edited schema.sql file
    /// and generates a migration that would transform the schema. This is
    /// the core of the model-first migration workflow.
    #[cfg(feature = "pglite")]
    Migrate {
        /// Path to the edited schema file (default: .tern/schema.sql)
        #[arg(short, long)]
        schema: Option<PathBuf>,

        /// Migration description
        #[arg(short, long)]
        description: String,

        /// Output format (text shows summary, sql shows migration SQL, json includes metadata)
        #[arg(long, default_value = "text")]
        format: OutputFormat,

        /// Path to the state directory
        #[arg(long)]
        path: Option<PathBuf>,

        /// Preview without recording the migration
        #[arg(long)]
        dry_run: bool,

        /// Skip confirmation prompt for destructive changes
        #[arg(long)]
        force: bool,
    },
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
            CliCommand::Init { from, schema, path } => {
                commands::run_init(from, &schema, path.as_deref()).await
            }
            CliCommand::Status { format, path } => {
                commands::run_status(format, path.as_deref()).await
            }
            CliCommand::Compile {
                database_url,
                schema,
                output,
                description,
                target,
                record,
                dry_run,
                show_sql,
                format,
                state_path,
            } => {
                commands::run_compile(
                    &database_url,
                    &schema,
                    output,
                    &description,
                    &target,
                    record,
                    dry_run,
                    show_sql,
                    format,
                    state_path.as_deref(),
                )
                .await
            }
            CliCommand::Build {
                database_url,
                schema,
                output,
                description,
                target,
                package_format,
                record,
                format,
                state_path,
            } => {
                commands::run_build(
                    &database_url,
                    &schema,
                    output,
                    &description,
                    &target,
                    &package_format,
                    record,
                    format,
                    state_path.as_deref(),
                )
                .await
            }
            CliCommand::History {
                format,
                limit,
                path,
            } => commands::run_history(format, limit, path.as_deref()).await,
            CliCommand::Show {
                migration_id,
                format,
                path,
            } => commands::run_show(&migration_id, format, path.as_deref()).await,
            CliCommand::Record {
                migration_id,
                migration_file,
                format,
                path,
            } => {
                commands::run_record(
                    migration_id.as_deref(),
                    migration_file,
                    format,
                    path.as_deref(),
                )
                .await
            }
            CliCommand::Inspect { path, format } => commands::run_inspect(path, format).await,
            CliCommand::Verify {
                database_url,
                schema,
                format,
                path,
            } => commands::run_verify(&database_url, &schema, format, path.as_deref()).await,
            CliCommand::VerifyChain { format, path } => {
                commands::run_verify_chain(format, path.as_deref()).await
            }
            CliCommand::Schema(action) => action.dispatch().await,
        }
    }
}

impl SchemaAction {
    /// Dispatch schema subcommands.
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            SchemaAction::Export {
                output,
                format,
                path,
            } => commands::run_schema_export(output, path.as_deref(), format).await,
            #[cfg(feature = "pglite")]
            SchemaAction::Diff {
                schema,
                format,
                path,
            } => commands::run_schema_diff(schema, path.as_deref(), format).await,
            #[cfg(feature = "pglite")]
            SchemaAction::Migrate {
                schema,
                description,
                format,
                path,
                dry_run,
                force,
            } => {
                commands::run_schema_migrate(
                    schema,
                    &description,
                    path.as_deref(),
                    format,
                    dry_run,
                    force,
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
