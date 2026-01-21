//! Tern Migration Runner - WASI CLI Component
//!
//! This crate implements the migration runner as a WASI CLI application.
//! It is compiled to `wasm32-wasip2` and composed with a migration component
//! to create standalone migration executables.
//!
//! # Architecture
//!
//! The runner component:
//! - **Imports** the `migration` interface (satisfied by composed migration component)
//! - **Exports** `wasi:cli/run` (the CLI entry point)
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Runner Component                         │
//! │                                                             │
//! │  Imports:                                                   │
//! │    - wasi:cli/* (environment, stdout, stderr)               │
//! │    - tern:migration/migration (describe, run, get-statements)│
//! │                                                             │
//! │  Exports:                                                   │
//! │    - wasi:cli/run (main entry point)                        │
//! │                                                             │
//! │  Flow:                                                      │
//! │    1. Parse CLI arguments                                   │
//! │    2. Call migration.describe() / migration.run()           │
//! │    3. Output results to stdout/stderr                       │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # CLI Usage
//!
//! When composed into a standalone executable:
//!
//! ```text
//! # Show migration metadata
//! ./migration --describe
//!
//! # Show SQL statements
//! ./migration --show-sql
//!
//! # Execute migration (placeholder until database connectivity is implemented)
//! ./migration --execute
//!
//! # JSON output
//! ./migration --describe --format json
//! ```

// Re-export WIT path for build scripts
pub use tern_migration_wit::{
    MAIN_WIT_FILE, RUNNER_WIT_FILE, RUNNER_WIT_PACKAGE, WIT_PACKAGE, WIT_PATH, WIT_VERSION,
};

// =============================================================================
// WASI Component Bindings (only for wasm target)
// =============================================================================

// Note: The wit_bindgen::generate! macro requires the actual WASI WIT files
// to be present. For now, we provide placeholder implementations that will
// be completed when the full WASI toolchain is set up.
//
// The eventual binding generation will look like:
//
// #[cfg(target_family = "wasm")]
// wit_bindgen::generate!({
//     path: "../tern-migration-wit/wit",
//     world: "tern-runner",
// });

// =============================================================================
// CLI Types
// =============================================================================

/// Output format for CLI commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Human-readable text output.
    #[default]
    Text,
    /// JSON output for programmatic consumption.
    Json,
}

impl OutputFormat {
    /// Parse output format from string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// Parsed CLI arguments.
#[derive(Debug, Clone, Default)]
pub struct CliArgs {
    /// Show migration metadata without executing.
    pub describe: bool,
    /// Show SQL statements without executing.
    pub show_sql: bool,
    /// Execute the migration.
    pub execute: bool,
    /// Output format (text or json).
    pub format: OutputFormat,
    /// Show help message.
    pub help: bool,
    /// Show version.
    pub version: bool,
}

impl CliArgs {
    /// Parse CLI arguments from a list of strings.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut result = Self::default();
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--describe" | "-d" => result.describe = true,
                "--show-sql" | "-s" => result.show_sql = true,
                "--execute" | "-e" => result.execute = true,
                "--help" | "-h" => result.help = true,
                "--version" | "-V" => result.version = true,
                "--format" | "-f" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("--format requires an argument".to_string());
                    }
                    result.format = OutputFormat::parse(&args[i])
                        .ok_or_else(|| format!("Unknown format: {}", args[i]))?;
                }
                arg if arg.starts_with("--format=") => {
                    let format_str = arg.strip_prefix("--format=").unwrap();
                    result.format = OutputFormat::parse(format_str)
                        .ok_or_else(|| format!("Unknown format: {}", format_str))?;
                }
                arg if arg.starts_with('-') => {
                    return Err(format!("Unknown argument: {}", arg));
                }
                _ => {
                    // Ignore positional arguments for now
                }
            }
            i += 1;
        }

        // Default to describe if no action specified
        if !result.describe
            && !result.show_sql
            && !result.execute
            && !result.help
            && !result.version
        {
            result.describe = true;
        }

        Ok(result)
    }
}

// =============================================================================
// Types mirroring the WIT interfaces (for native testing)
// =============================================================================

/// Mitigation strategy for breaking changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MitigationStrategy {
    DualWrite,
    Backfill,
    Ratchet,
    Destructive,
}

impl MitigationStrategy {
    /// Get string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DualWrite => "dual-write",
            Self::Backfill => "backfill",
            Self::Ratchet => "ratchet",
            Self::Destructive => "destructive",
        }
    }
}

/// A breaking change in the migration.
#[derive(Debug, Clone)]
pub struct BreakingChange {
    pub description: String,
    pub mitigation: MitigationStrategy,
    pub affected_sql: Vec<String>,
}

/// Migration metadata.
#[derive(Debug, Clone)]
pub struct MigrationMetadata {
    pub id: String,
    pub description: String,
    pub breaking_changes: Vec<BreakingChange>,
    pub statement_count: u32,
    pub source_state_hash: String,
    pub target_state_hash: String,
    pub compiled_at: String,
}

impl MigrationMetadata {
    /// Check if migration has breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        !self.breaking_changes.is_empty()
    }
}

/// A SQL statement with metadata.
#[derive(Debug, Clone)]
pub struct Statement {
    pub sql: String,
    pub description: String,
    pub sequence: u32,
}

// =============================================================================
// Output Formatting
// =============================================================================

/// Format metadata for text output.
pub fn format_metadata_text(metadata: &MigrationMetadata) -> String {
    let mut output = String::new();
    output.push_str(&format!("Migration: {}\n", metadata.id));
    output.push_str(&format!("Description: {}\n", metadata.description));
    output.push_str(&format!("Statements: {}\n", metadata.statement_count));
    output.push_str(&format!("Source State: {}\n", metadata.source_state_hash));
    output.push_str(&format!("Target State: {}\n", metadata.target_state_hash));
    output.push_str(&format!("Compiled At: {}\n", metadata.compiled_at));

    if metadata.has_breaking_changes() {
        output.push_str("\nBreaking Changes:\n");
        for bc in &metadata.breaking_changes {
            output.push_str(&format!(
                "  - {} [{}]\n",
                bc.description,
                bc.mitigation.as_str()
            ));
            for sql in &bc.affected_sql {
                output.push_str(&format!("    SQL: {}\n", truncate_sql(sql, 60)));
            }
        }
    } else {
        output.push_str("\nNo breaking changes detected.\n");
    }

    output
}

/// Format metadata as JSON.
pub fn format_metadata_json(metadata: &MigrationMetadata) -> String {
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
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "{}".to_string())
}

/// Format statements for text output.
pub fn format_statements_text(statements: &[Statement]) -> String {
    let mut output = String::new();
    for stmt in statements {
        output.push_str(&format!("-- {} ({})\n", stmt.description, stmt.sequence));
        output.push_str(&format!("{};\n\n", stmt.sql));
    }
    output
}

/// Format statements as JSON.
pub fn format_statements_json(statements: &[Statement]) -> String {
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
    serde_json::to_string_pretty(&json).unwrap_or_else(|_| "[]".to_string())
}

/// Truncate SQL for display.
pub fn truncate_sql(sql: &str, max_len: usize) -> String {
    let sql = sql.replace('\n', " ").replace("  ", " ");
    if sql.len() <= max_len {
        sql
    } else {
        format!("{}...", &sql[..max_len - 3])
    }
}

/// Generate help message.
pub fn help_message() -> String {
    r#"Tern Migration Runner

Execute a compiled database migration.

USAGE:
    migration [OPTIONS]

OPTIONS:
    -d, --describe       Show migration metadata without executing
    -s, --show-sql       Show SQL statements without executing
    -e, --execute        Execute the migration
    -f, --format <fmt>   Output format: text (default) or json
    -h, --help           Show this help message
    -V, --version        Show version

EXAMPLES:
    migration --describe
    migration --show-sql --format json
    migration --execute
"#
    .to_string()
}

/// Get version string.
pub fn version_string() -> String {
    format!("tern-migration-runner {}", env!("CARGO_PKG_VERSION"))
}

/// Current version of the migration runner.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name of the runner executable.
pub const EXECUTABLE_NAME: &str = "tern-migration-runner";

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod cli_args_tests {
        use super::*;

        #[test]
        fn parse_empty_defaults_to_describe() {
            let args = CliArgs::parse(&[]).unwrap();
            assert!(args.describe);
            assert!(!args.show_sql);
            assert!(!args.execute);
        }

        #[test]
        fn parse_describe_flag() {
            let args = CliArgs::parse(&["--describe".to_string()]).unwrap();
            assert!(args.describe);
        }

        #[test]
        fn parse_describe_short_flag() {
            let args = CliArgs::parse(&["-d".to_string()]).unwrap();
            assert!(args.describe);
        }

        #[test]
        fn parse_show_sql_flag() {
            let args = CliArgs::parse(&["--show-sql".to_string()]).unwrap();
            assert!(args.show_sql);
        }

        #[test]
        fn parse_execute_flag() {
            let args = CliArgs::parse(&["--execute".to_string()]).unwrap();
            assert!(args.execute);
        }

        #[test]
        fn parse_help_flag() {
            let args = CliArgs::parse(&["--help".to_string()]).unwrap();
            assert!(args.help);
        }

        #[test]
        fn parse_version_flag() {
            let args = CliArgs::parse(&["--version".to_string()]).unwrap();
            assert!(args.version);
        }

        #[test]
        fn parse_format_text() {
            let args = CliArgs::parse(&["--format".to_string(), "text".to_string()]).unwrap();
            assert_eq!(args.format, OutputFormat::Text);
        }

        #[test]
        fn parse_format_json() {
            let args = CliArgs::parse(&["--format".to_string(), "json".to_string()]).unwrap();
            assert_eq!(args.format, OutputFormat::Json);
        }

        #[test]
        fn parse_format_equals_syntax() {
            let args = CliArgs::parse(&["--format=json".to_string()]).unwrap();
            assert_eq!(args.format, OutputFormat::Json);
        }

        #[test]
        fn parse_multiple_flags() {
            let args =
                CliArgs::parse(&["--describe".to_string(), "--format=json".to_string()]).unwrap();
            assert!(args.describe);
            assert_eq!(args.format, OutputFormat::Json);
        }

        #[test]
        fn parse_unknown_flag_error() {
            let result = CliArgs::parse(&["--unknown".to_string()]);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Unknown argument"));
        }

        #[test]
        fn parse_format_missing_argument() {
            let result = CliArgs::parse(&["--format".to_string()]);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("requires an argument"));
        }

        #[test]
        fn parse_format_invalid_value() {
            let result = CliArgs::parse(&["--format".to_string(), "xml".to_string()]);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Unknown format"));
        }
    }

    mod output_format_tests {
        use super::*;

        #[test]
        fn parse_text() {
            assert_eq!(OutputFormat::parse("text"), Some(OutputFormat::Text));
            assert_eq!(OutputFormat::parse("TEXT"), Some(OutputFormat::Text));
        }

        #[test]
        fn parse_json() {
            assert_eq!(OutputFormat::parse("json"), Some(OutputFormat::Json));
            assert_eq!(OutputFormat::parse("JSON"), Some(OutputFormat::Json));
        }

        #[test]
        fn parse_invalid() {
            assert_eq!(OutputFormat::parse("xml"), None);
            assert_eq!(OutputFormat::parse(""), None);
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
    }

    mod help_tests {
        use super::*;

        #[test]
        fn help_message_contains_usage() {
            let help = help_message();
            assert!(help.contains("USAGE:"));
            assert!(help.contains("OPTIONS:"));
        }

        #[test]
        fn help_message_contains_all_flags() {
            let help = help_message();
            assert!(help.contains("--describe"));
            assert!(help.contains("--show-sql"));
            assert!(help.contains("--execute"));
            assert!(help.contains("--format"));
            assert!(help.contains("--help"));
            assert!(help.contains("--version"));
        }

        #[test]
        fn version_string_contains_version() {
            let version = version_string();
            assert!(version.contains("tern-migration-runner"));
        }
    }

    mod format_tests {
        use super::*;

        fn test_metadata() -> MigrationMetadata {
            MigrationMetadata {
                id: "test-id-123".to_string(),
                description: "Test migration".to_string(),
                breaking_changes: vec![],
                statement_count: 2,
                source_state_hash: "source-hash".to_string(),
                target_state_hash: "target-hash".to_string(),
                compiled_at: "2024-01-15T10:00:00Z".to_string(),
            }
        }

        fn test_metadata_with_breaking_changes() -> MigrationMetadata {
            let mut metadata = test_metadata();
            metadata.breaking_changes = vec![BreakingChange {
                description: "Dropping column".to_string(),
                mitigation: MitigationStrategy::Destructive,
                affected_sql: vec!["ALTER TABLE t DROP COLUMN c".to_string()],
            }];
            metadata
        }

        fn test_statements() -> Vec<Statement> {
            vec![
                Statement {
                    sql: "CREATE TABLE t (id INT)".to_string(),
                    description: "Create table".to_string(),
                    sequence: 1,
                },
                Statement {
                    sql: "CREATE INDEX idx ON t(id)".to_string(),
                    description: "Create index".to_string(),
                    sequence: 2,
                },
            ]
        }

        #[test]
        fn format_metadata_text_output() {
            let metadata = test_metadata();
            let output = format_metadata_text(&metadata);

            assert!(output.contains("Migration: test-id-123"));
            assert!(output.contains("Description: Test migration"));
            assert!(output.contains("Statements: 2"));
            assert!(output.contains("No breaking changes detected"));
        }

        #[test]
        fn format_metadata_text_with_breaking_changes() {
            let metadata = test_metadata_with_breaking_changes();
            let output = format_metadata_text(&metadata);

            assert!(output.contains("Breaking Changes:"));
            assert!(output.contains("Dropping column"));
            assert!(output.contains("destructive"));
        }

        #[test]
        fn format_metadata_json_output() {
            let metadata = test_metadata();
            let output = format_metadata_json(&metadata);

            assert!(output.contains("\"id\": \"test-id-123\""));
            assert!(output.contains("\"description\": \"Test migration\""));
            assert!(output.contains("\"statement_count\": 2"));
        }

        #[test]
        fn format_statements_text_output() {
            let statements = test_statements();
            let output = format_statements_text(&statements);

            assert!(output.contains("-- Create table (1)"));
            assert!(output.contains("CREATE TABLE t (id INT);"));
            assert!(output.contains("-- Create index (2)"));
        }

        #[test]
        fn format_statements_json_output() {
            let statements = test_statements();
            let output = format_statements_json(&statements);

            assert!(output.contains("\"sequence\": 1"));
            assert!(output.contains("\"description\": \"Create table\""));
            assert!(output.contains("\"sql\": \"CREATE TABLE t (id INT)\""));
        }
    }

    mod mitigation_strategy_tests {
        use super::*;

        #[test]
        fn as_str_returns_correct_values() {
            assert_eq!(MitigationStrategy::DualWrite.as_str(), "dual-write");
            assert_eq!(MitigationStrategy::Backfill.as_str(), "backfill");
            assert_eq!(MitigationStrategy::Ratchet.as_str(), "ratchet");
            assert_eq!(MitigationStrategy::Destructive.as_str(), "destructive");
        }
    }
}
