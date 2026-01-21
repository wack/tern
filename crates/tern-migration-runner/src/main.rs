//! Tern Migration Runner CLI (Native Testing Binary)
//!
//! This is a native testing binary for the migration runner. It is NOT the
//! actual WASI component that gets embedded in migration executables.
//!
//! The native binary is built with the `native` feature and is used for:
//! - Testing CLI argument parsing
//! - Testing output formatting
//! - Development and debugging
//!
//! For actual migration execution, the runner is compiled to `wasm32-wasip2`
//! and composed with a migration component to create standalone executables.

#![cfg(feature = "native")]

use std::process::ExitCode;

use clap::Parser;

use tern_migration_runner::{
    BreakingChange, MigrationMetadata, MitigationStrategy, OutputFormat, Statement,
    format_metadata_json, format_metadata_text, format_statements_json, format_statements_text,
    help_message, truncate_sql, version_string,
};

/// Output format for describe and dry-run commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CliOutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for programmatic consumption.
    Json,
}

impl std::str::FromStr for CliOutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            _ => Err(format!("unknown format: {} (expected 'text' or 'json')", s)),
        }
    }
}

impl From<CliOutputFormat> for OutputFormat {
    fn from(value: CliOutputFormat) -> Self {
        match value {
            CliOutputFormat::Text => OutputFormat::Text,
            CliOutputFormat::Json => OutputFormat::Json,
        }
    }
}

/// Tern Migration Runner - Native Testing CLI
///
/// This is a native testing binary for development purposes.
/// The actual runner is compiled to WASI for embedding in migration executables.
#[derive(Debug, Parser)]
#[command(name = "tern-migration-runner")]
#[command(about = "Native testing CLI for migration runner development")]
#[command(version)]
pub struct Cli {
    /// Show migration metadata without executing.
    #[arg(long, short, default_value = "false")]
    describe: bool,

    /// Show SQL statements without executing.
    #[arg(long, short, default_value = "false")]
    show_sql: bool,

    /// Output format.
    #[arg(long, short, default_value = "text")]
    format: CliOutputFormat,

    /// Show help message (library version).
    #[arg(long)]
    show_help: bool,

    /// Show version (library version).
    #[arg(long)]
    show_version: bool,

    /// Enable verbose output.
    #[arg(long, short, default_value = "false")]
    verbose: bool,

    /// Use test data instead of requiring a component.
    #[arg(long, default_value = "true")]
    test_mode: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if cli.show_help {
        println!("{}", help_message());
        return ExitCode::SUCCESS;
    }

    if cli.show_version {
        println!("{}", version_string());
        return ExitCode::SUCCESS;
    }

    // In test mode, use sample data
    if cli.test_mode {
        if cli.describe || (!cli.show_sql && !cli.describe) {
            describe_test_migration(cli.format.into());
        }
        if cli.show_sql {
            show_test_sql(cli.format.into());
        }
        return ExitCode::SUCCESS;
    }

    // Without test mode, we'd need an actual WASI component
    eprintln!("Error: Non-test mode not yet supported. Use --test-mode for development testing.");
    ExitCode::FAILURE
}

/// Show test migration metadata.
fn describe_test_migration(format: OutputFormat) {
    let metadata = test_metadata();

    match format {
        OutputFormat::Text => {
            print!("{}", format_metadata_text(&metadata));
        }
        OutputFormat::Json => {
            println!("{}", format_metadata_json(&metadata));
        }
    }
}

/// Show test SQL statements.
fn show_test_sql(format: OutputFormat) {
    let statements = test_statements();

    match format {
        OutputFormat::Text => {
            print!("{}", format_statements_text(&statements));
        }
        OutputFormat::Json => {
            println!("{}", format_statements_json(&statements));
        }
    }
}

/// Generate test metadata for development.
fn test_metadata() -> MigrationMetadata {
    MigrationMetadata {
        id: "test-migration-abc123".to_string(),
        description: "Add users table with email column".to_string(),
        breaking_changes: vec![BreakingChange {
            description: "Adding NOT NULL column without default".to_string(),
            mitigation: MitigationStrategy::Backfill,
            affected_sql: vec!["ALTER TABLE users ADD COLUMN email TEXT NOT NULL".to_string()],
        }],
        statement_count: 2,
        source_state_hash: "source-hash-def456".to_string(),
        target_state_hash: "target-hash-ghi789".to_string(),
        compiled_at: "2024-01-15T10:00:00Z".to_string(),
    }
}

/// Generate test statements for development.
fn test_statements() -> Vec<Statement> {
    vec![
        Statement {
            sql: "CREATE TABLE users (\n    id SERIAL PRIMARY KEY,\n    name TEXT NOT NULL\n)"
                .to_string(),
            description: "Create users table".to_string(),
            sequence: 1,
        },
        Statement {
            sql: "ALTER TABLE users ADD COLUMN email TEXT NOT NULL DEFAULT ''".to_string(),
            description: "Add email column".to_string(),
            sequence: 2,
        },
    ]
}

// Silence unused warning for truncate_sql (imported for completeness)
#[allow(dead_code)]
fn _use_truncate_sql() {
    let _ = truncate_sql("test", 10);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_describe() {
        let cli = Cli::parse_from(["test", "--describe"]);
        assert!(cli.describe);
    }

    #[test]
    fn parse_show_sql() {
        let cli = Cli::parse_from(["test", "--show-sql"]);
        assert!(cli.show_sql);
    }

    #[test]
    fn parse_format_json() {
        let cli = Cli::parse_from(["test", "--format", "json"]);
        assert_eq!(cli.format, CliOutputFormat::Json);
    }

    #[test]
    fn default_test_mode() {
        let cli = Cli::parse_from(["test"]);
        assert!(cli.test_mode);
    }

    #[test]
    fn test_metadata_has_breaking_changes() {
        let metadata = test_metadata();
        assert!(metadata.has_breaking_changes());
    }

    #[test]
    fn test_statements_has_correct_count() {
        let statements = test_statements();
        assert_eq!(statements.len(), 2);
    }

    #[test]
    fn cli_output_format_from_str() {
        let text: CliOutputFormat = "text".parse().unwrap();
        assert_eq!(text, CliOutputFormat::Text);

        let json: CliOutputFormat = "json".parse().unwrap();
        assert_eq!(json, CliOutputFormat::Json);
    }

    #[test]
    fn cli_output_format_to_output_format() {
        let text: OutputFormat = CliOutputFormat::Text.into();
        assert_eq!(text, OutputFormat::Text);

        let json: OutputFormat = CliOutputFormat::Json.into();
        assert_eq!(json, OutputFormat::Json);
    }
}
