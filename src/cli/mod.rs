mod colors;

pub use colors::EnableColors;

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
