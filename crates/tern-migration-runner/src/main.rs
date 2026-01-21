//! Tern Migration Runner CLI
//!
//! This is the CLI entry point for standalone migration executables.
//! When a migration is compiled, this runner is bundled with the migration
//! component to create a self-contained executable.
//!
//! # Usage
//!
//! ```text
//! # Execute a migration
//! ./migration --database-url postgres://localhost/mydb
//!
//! # Dry run (show SQL without executing)
//! ./migration --database-url postgres://localhost/mydb --dry-run
//!
//! # Show metadata
//! ./migration --describe
//!
//! # Output as JSON
//! ./migration --describe --format json
//! ```

use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use tern_migration_runner::error::CliError;
use tern_migration_runner::host::{HostState, connect_database};
use tern_migration_runner::runtime::MigrationRuntime;

/// Output format for describe and dry-run commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for programmatic consumption.
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(format!("unknown format: {} (expected 'text' or 'json')", s)),
        }
    }
}

/// Tern Migration Runner - Execute compiled database migrations.
#[derive(Debug, Parser)]
#[command(name = "tern-migration")]
#[command(about = "Execute a compiled database migration")]
#[command(version)]
pub struct Cli {
    /// PostgreSQL database URL.
    ///
    /// Can also be set via the DATABASE_URL environment variable.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,

    /// Dry run mode - show SQL without executing.
    ///
    /// In this mode, the migration will run but no SQL will actually be
    /// executed against the database. The SQL statements that would be
    /// executed are printed to stdout.
    #[arg(long, default_value = "false")]
    dry_run: bool,

    /// Skip confirmation prompts for breaking changes.
    ///
    /// By default, the runner will prompt for confirmation before executing
    /// migrations that contain breaking changes. Use this flag to skip
    /// the prompt (useful for CI/CD pipelines).
    #[arg(long, short, default_value = "false")]
    yes: bool,

    /// Show migration metadata without executing.
    ///
    /// Displays information about the migration including description,
    /// statement count, and any breaking changes.
    #[arg(long, default_value = "false")]
    describe: bool,

    /// Output format for describe and dry-run modes.
    #[arg(long, default_value = "text")]
    format: OutputFormat,

    /// Show SQL statements without executing.
    ///
    /// Similar to dry-run but just lists the SQL statements.
    #[arg(long, default_value = "false")]
    show_sql: bool,

    /// Enable verbose logging.
    #[arg(long, short, default_value = "false")]
    verbose: bool,

    /// Path to the WebAssembly component file.
    ///
    /// If not provided, the runner expects the component to be embedded
    /// at compile time (for standalone executables).
    #[arg(long)]
    component: Option<std::path::PathBuf>,
}

/// Main entry point.
fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Run with tokio for async database operations
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {:?}", err);
            ExitCode::FAILURE
        }
    }
}

/// Run the CLI command.
async fn run(cli: Cli) -> Result<(), CliError> {
    // Load the component
    let component_bytes = load_component(&cli)?;
    let runtime = MigrationRuntime::from_bytes(&component_bytes)?;

    // Handle different modes
    if cli.describe {
        describe_migration(&runtime, cli.format)?;
    } else if cli.show_sql {
        show_sql(&runtime, cli.format)?;
    } else if cli.dry_run {
        dry_run(&runtime, cli.format)?;
    } else {
        execute_migration(&runtime, &cli).await?;
    }

    Ok(())
}

/// Load the WebAssembly component.
fn load_component(cli: &Cli) -> Result<Vec<u8>, CliError> {
    if let Some(path) = &cli.component {
        // Load from file
        std::fs::read(path).map_err(|e| CliError::file_read(path.display().to_string(), e))
    } else {
        // For embedded components, this would use include_bytes! in build.rs
        // For now, require the --component flag
        Err(CliError::missing_argument("--component"))
    }
}

/// Show migration metadata.
fn describe_migration(runtime: &MigrationRuntime, format: OutputFormat) -> Result<(), CliError> {
    let metadata = runtime.describe()?;

    match format {
        OutputFormat::Text => {
            println!("Migration: {}", metadata.id);
            println!("Description: {}", metadata.description);
            println!("Statements: {}", metadata.statement_count);
            println!("Source State: {}", metadata.source_state_hash);
            println!("Target State: {}", metadata.target_state_hash);
            println!("Compiled At: {}", metadata.compiled_at);

            if metadata.has_breaking_changes() {
                println!();
                println!("Breaking Changes:");
                for bc in &metadata.breaking_changes {
                    println!("  - {} [{}]", bc.description, bc.mitigation);
                    for sql in &bc.affected_sql {
                        println!("    SQL: {}", truncate_sql(sql, 60));
                    }
                }
            } else {
                println!();
                println!("No breaking changes detected.");
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "id": metadata.id,
                "description": metadata.description,
                "statement_count": metadata.statement_count,
                "source_state_hash": metadata.source_state_hash,
                "target_state_hash": metadata.target_state_hash,
                "compiled_at": metadata.compiled_at,
                "breaking_changes": metadata.breaking_changes.iter().map(|bc| {
                    serde_json::json!({
                        "description": bc.description,
                        "mitigation": bc.mitigation.as_str(),
                        "affected_sql": bc.affected_sql,
                    })
                }).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    Ok(())
}

/// Show SQL statements.
fn show_sql(runtime: &MigrationRuntime, format: OutputFormat) -> Result<(), CliError> {
    let statements = runtime.get_statements()?;

    match format {
        OutputFormat::Text => {
            for stmt in &statements {
                println!("-- {} ({})", stmt.description, stmt.sequence);
                println!("{};", stmt.sql);
                println!();
            }
        }
        OutputFormat::Json => {
            let json: Vec<_> = statements
                .iter()
                .map(|stmt| {
                    serde_json::json!({
                        "sequence": stmt.sequence,
                        "description": stmt.description,
                        "sql": stmt.sql,
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    Ok(())
}

/// Perform a dry run.
fn dry_run(runtime: &MigrationRuntime, format: OutputFormat) -> Result<(), CliError> {
    let statements = runtime.run_dry()?;

    match format {
        OutputFormat::Text => {
            println!("Dry run - the following SQL would be executed:");
            println!();
            for (i, sql) in statements.iter().enumerate() {
                println!("-- Statement {}", i + 1);
                println!("{};", sql);
                println!();
            }
            println!("Total: {} statement(s)", statements.len());
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "statements": statements,
                "count": statements.len(),
            });
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
    }

    Ok(())
}

/// Execute the migration.
async fn execute_migration(runtime: &MigrationRuntime, cli: &Cli) -> Result<(), CliError> {
    // Require database URL for execution
    let database_url = cli
        .database_url
        .as_ref()
        .ok_or_else(|| CliError::missing_argument("--database-url"))?;

    // Get metadata to check for breaking changes
    let metadata = runtime.describe()?;

    // Prompt for confirmation if there are breaking changes
    if metadata.has_breaking_changes() && !cli.yes {
        println!("Warning: This migration contains breaking changes:");
        for bc in &metadata.breaking_changes {
            println!("  - {} [{}]", bc.description, bc.mitigation);
        }
        println!();

        if !confirm("Do you want to proceed?")? {
            return Err(CliError::cancelled());
        }
    }

    // Connect to database
    println!("Connecting to database...");
    let client = connect_database(database_url).await?;
    let host_state = HostState::with_client(client);

    // Execute migration
    println!("Executing migration: {}", metadata.description);
    let result_state = runtime.run(host_state)?;

    println!();
    println!("Migration completed successfully!");
    println!(
        "Executed {} statement(s)",
        result_state.dry_run_statements().len()
    );

    Ok(())
}

/// Prompt for user confirmation.
fn confirm(prompt: &str) -> Result<bool, CliError> {
    print!("{} [y/N] ", prompt);
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    Ok(input.trim().to_lowercase() == "y" || input.trim().to_lowercase() == "yes")
}

/// Truncate SQL for display.
fn truncate_sql(sql: &str, max_len: usize) -> String {
    let sql = sql.replace('\n', " ").replace("  ", " ");
    if sql.len() <= max_len {
        sql
    } else {
        format!("{}...", &sql[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod output_format_tests {
        use super::*;

        #[test]
        fn from_str_text() {
            let format: OutputFormat = "text".parse().unwrap();
            assert_eq!(format, OutputFormat::Text);
        }

        #[test]
        fn from_str_json() {
            let format: OutputFormat = "json".parse().unwrap();
            assert_eq!(format, OutputFormat::Json);
        }

        #[test]
        fn from_str_case_insensitive() {
            let format: OutputFormat = "JSON".parse().unwrap();
            assert_eq!(format, OutputFormat::Json);

            let format: OutputFormat = "Text".parse().unwrap();
            assert_eq!(format, OutputFormat::Text);
        }

        #[test]
        fn from_str_unknown() {
            let result: Result<OutputFormat, _> = "xml".parse();
            assert!(result.is_err());
        }

        #[test]
        fn default_is_text() {
            assert_eq!(OutputFormat::default(), OutputFormat::Text);
        }
    }

    mod truncate_sql_tests {
        use super::*;

        #[test]
        fn short_sql_unchanged() {
            let sql = "SELECT 1";
            assert_eq!(truncate_sql(sql, 20), "SELECT 1");
        }

        #[test]
        fn long_sql_truncated() {
            let sql = "SELECT * FROM very_long_table_name WHERE id = 1";
            let truncated = truncate_sql(sql, 30);
            assert!(truncated.len() <= 30);
            assert!(truncated.ends_with("..."));
        }

        #[test]
        fn newlines_replaced() {
            let sql = "SELECT\n*\nFROM\ntable";
            let result = truncate_sql(sql, 100);
            assert!(!result.contains('\n'));
        }

        #[test]
        fn double_spaces_collapsed() {
            let sql = "SELECT  *  FROM  table";
            let result = truncate_sql(sql, 100);
            assert!(!result.contains("  "));
        }

        #[test]
        fn exact_length_not_truncated() {
            let sql = "12345678901234567890"; // 20 chars
            assert_eq!(truncate_sql(sql, 20), sql);
        }

        #[test]
        fn one_over_truncated() {
            let sql = "123456789012345678901"; // 21 chars
            let result = truncate_sql(sql, 20);
            assert_eq!(result.len(), 20);
            assert!(result.ends_with("..."));
        }
    }

    mod cli_tests {
        use super::*;
        use clap::CommandFactory;

        #[test]
        fn verify_cli() {
            // This verifies that the CLI is properly configured
            Cli::command().debug_assert();
        }

        #[test]
        fn parse_minimal() {
            let cli = Cli::parse_from(["test", "--component", "test.wasm", "--describe"]);
            assert_eq!(cli.component, Some(std::path::PathBuf::from("test.wasm")));
            assert!(cli.describe);
        }

        #[test]
        fn parse_with_database_url() {
            let cli = Cli::parse_from([
                "test",
                "--component",
                "test.wasm",
                "--database-url",
                "postgres://localhost/db",
            ]);
            assert_eq!(
                cli.database_url,
                Some("postgres://localhost/db".to_string())
            );
        }

        #[test]
        fn parse_dry_run() {
            let cli = Cli::parse_from([
                "test",
                "--component",
                "test.wasm",
                "--dry-run",
                "--describe",
            ]);
            assert!(cli.dry_run);
        }

        #[test]
        fn parse_yes_flag() {
            let cli = Cli::parse_from([
                "test",
                "--component",
                "test.wasm",
                "-y",
                "--database-url",
                "postgres://localhost/db",
            ]);
            assert!(cli.yes);
        }

        #[test]
        fn parse_verbose() {
            let cli = Cli::parse_from(["test", "--component", "test.wasm", "-v", "--describe"]);
            assert!(cli.verbose);
        }

        #[test]
        fn parse_format_json() {
            let cli = Cli::parse_from([
                "test",
                "--component",
                "test.wasm",
                "--format",
                "json",
                "--describe",
            ]);
            assert_eq!(cli.format, OutputFormat::Json);
        }

        #[test]
        fn parse_show_sql() {
            let cli = Cli::parse_from(["test", "--component", "test.wasm", "--show-sql"]);
            assert!(cli.show_sql);
        }
    }
}
