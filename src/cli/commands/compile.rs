//! Compile migration command.
//!
//! This command compiles a migration by comparing the current state backend
//! to the live database and generating migration SQL.

use std::path::PathBuf;

use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, print_json};
use crate::db::compile::{CompileOptions, Target, compile_migration};
use crate::db::query::PostgresCatalog;
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::db::{self};

/// Compile output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct CompileOutput {
    /// Migration ID (hex).
    pub migration_id: String,
    /// Migration description.
    pub description: String,
    /// Number of SQL statements.
    pub statement_count: usize,
    /// Whether there are breaking changes.
    pub has_breaking_changes: bool,
    /// Whether there are destructive changes.
    pub has_destructive_changes: bool,
    /// Source state hash.
    pub source_state_hash: String,
    /// Target state hash.
    pub target_state_hash: String,
    /// Output path (if written).
    pub output_path: Option<String>,
    /// SQL statements (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statements: Option<Vec<StatementOutput>>,
    /// Breaking changes (if any).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breaking_changes: Vec<BreakingChangeOutput>,
}

/// Statement output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct StatementOutput {
    /// Statement description.
    pub description: String,
    /// SQL text.
    pub sql: String,
}

/// Breaking change output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct BreakingChangeOutput {
    /// Description of the breaking change.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: String,
}

impl std::fmt::Display for CompileOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration Compiled")?;
        writeln!(f, "==================")?;
        writeln!(f)?;
        writeln!(f, "  ID:          {}", self.migration_id)?;
        writeln!(f, "  Description: {}", self.description)?;
        writeln!(f, "  Statements:  {}", self.statement_count)?;
        writeln!(f, "  From state:  {}", &self.source_state_hash[..16])?;
        writeln!(f, "  To state:    {}", &self.target_state_hash[..16])?;

        if self.has_breaking_changes {
            writeln!(f)?;
            writeln!(f, "WARNING: Breaking Changes Detected")?;
            writeln!(f, "-----------------------------------")?;
            for bc in &self.breaking_changes {
                writeln!(f, "  [{}] {}", bc.mitigation, bc.description)?;
            }
        }

        if let Some(ref path) = self.output_path {
            writeln!(f)?;
            writeln!(f, "Output written to: {}", path)?;
        }

        Ok(())
    }
}

/// Runs the compile command.
///
/// Compiles a migration by comparing the current state to the live database.
///
/// # Arguments
///
/// * `database_url` - PostgreSQL connection string
/// * `schema` - Database schema name
/// * `output` - Optional output path for source code
/// * `description` - Migration description
/// * `target` - Target platform for compilation
/// * `record` - Whether to record the migration to the state backend
/// * `dry_run` - Preview without writing
/// * `show_sql` - Include SQL statements in output
/// * `format` - Output format
/// * `state_path` - Optional path to the state directory
#[allow(clippy::too_many_arguments)]
pub async fn run_compile(
    database_url: &str,
    schema: &str,
    output: Option<PathBuf>,
    description: &str,
    target: &str,
    record: bool,
    dry_run: bool,
    show_sql: bool,
    format: OutputFormat,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Parse target
    let target = Target::from_str_name(target).ok_or_else(|| {
        miette!(
            "Invalid target: '{}'. Valid targets: native, x86_64-linux-gnu, x86_64-linux-musl, x86_64-macos, aarch64-macos, x86_64-windows",
            target
        )
    })?;

    // Load the state backend
    let backend = match state_path {
        Some(p) => LocalFileBackend::at_path(p),
        None => LocalFileBackend::default_location(),
    };

    if !backend.is_initialized().await.into_diagnostic()? {
        return Err(miette!(
            "State backend not initialized at {}\n\nRun 'tern init' to initialize a new project.",
            backend.root().display()
        ));
    }

    // Get current state from backend
    let source_state = backend
        .get_current_state()
        .await
        .into_diagnostic()
        .wrap_err("Failed to load current state from backend")?;

    // Connect to database and load target state
    if !dry_run {
        println!("Connecting to database...");
    }

    let client = db::connect(database_url)
        .await
        .into_diagnostic()
        .wrap_err("Failed to connect to database")?;

    let catalog = PostgresCatalog::new(&client);

    if !dry_run {
        println!("Loading schema '{}'...", schema);
    }

    let target_state = crate::db::query::load_namespace(&catalog, schema)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to load schema '{}'", schema))?;

    // Compile the migration
    if !dry_run {
        println!("Compiling migration...");
    }

    let options = CompileOptions::new(description).with_target(target);
    let result = compile_migration(&source_state, &target_state, options).into_diagnostic()?;

    // Build output
    let statements = if show_sql {
        Some(
            result
                .compilation
                .statements
                .iter()
                .map(|s| StatementOutput {
                    description: s.description.clone(),
                    sql: s.sql.clone(),
                })
                .collect(),
        )
    } else {
        None
    };

    let breaking_changes: Vec<BreakingChangeOutput> = result
        .compilation
        .breaking_changes
        .iter()
        .map(|bc| BreakingChangeOutput {
            description: bc.description.clone(),
            mitigation: format!("{:?}", bc.mitigation),
        })
        .collect();

    let compile_output = CompileOutput {
        migration_id: result.migration_id(),
        description: description.to_string(),
        statement_count: result.statement_count(),
        has_breaking_changes: result.has_breaking_changes(),
        has_destructive_changes: result.has_destructive_changes(),
        source_state_hash: result.source_hash.to_hex(),
        target_state_hash: result.target_hash.to_hex(),
        output_path: output.as_ref().map(|p| p.display().to_string()),
        statements,
        breaking_changes,
    };

    // Write output if requested
    if let Some(ref output_path) = output
        && !dry_run
    {
        // Create parent directories if needed
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .into_diagnostic()
                .wrap_err("Failed to create output directory")?;
        }

        // Write the source code
        std::fs::write(output_path, result.source_code())
            .into_diagnostic()
            .wrap_err("Failed to write output file")?;
    }

    // Record migration if requested
    if record && !dry_run {
        backend
            .record_migration(&result.migration, &target_state)
            .await
            .into_diagnostic()
            .wrap_err("Failed to record migration")?;

        println!("Migration recorded to state backend.");
    }

    // Output results
    match format {
        OutputFormat::Text => println!("{}", compile_output),
        OutputFormat::Json => print_json(&compile_output),
        OutputFormat::Sql => {
            for stmt in &result.compilation.statements {
                println!("{};", stmt.sql);
                println!();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_output_display() {
        let output = CompileOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            statement_count: 3,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            output_path: Some("./migrations/add_users.rs".to_string()),
            statements: None,
            breaking_changes: vec![],
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration Compiled"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("3"));
    }

    #[test]
    fn compile_output_with_breaking_changes() {
        let output = CompileOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Drop users table".to_string(),
            statement_count: 1,
            has_breaking_changes: true,
            has_destructive_changes: true,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            output_path: None,
            statements: None,
            breaking_changes: vec![BreakingChangeOutput {
                description: "Dropping table 'users'".to_string(),
                mitigation: "Destructive".to_string(),
            }],
        };

        let display = format!("{}", output);
        assert!(display.contains("WARNING"));
        assert!(display.contains("Breaking Changes"));
        assert!(display.contains("Destructive"));
    }

    #[test]
    fn compile_output_json_serializes() {
        let output = CompileOutput {
            migration_id: "abc".to_string(),
            description: "Test".to_string(),
            statement_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "src".to_string(),
            target_state_hash: "tgt".to_string(),
            output_path: None,
            statements: Some(vec![StatementOutput {
                description: "Create table".to_string(),
                sql: "CREATE TABLE foo ()".to_string(),
            }]),
            breaking_changes: vec![],
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"migration_id\":\"abc\""));
        assert!(json.contains("CREATE TABLE foo"));
    }
}
