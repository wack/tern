//! Compile migration command.
//!
//! This command compiles a migration by comparing the current state backend
//! to the live database and generating migration SQL.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::compile::{CompileOptions, Target, compile_migration};
use crate::db::query::PostgresCatalog;
use crate::db::state::{StateBackend, StateHash};
use crate::db::{self};

/// Compile a migration to source code
///
/// Compares the state backend to the live database and generates
/// migration source code for the detected changes.
#[derive(Debug, Clone, Args)]
pub struct Compile {
    /// PostgreSQL connection string
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    /// The database schema to compare
    #[arg(long, default_value = "public")]
    pub schema: String,

    /// Output path for the generated source code
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Migration description
    #[arg(long)]
    pub description: String,

    /// Target platform for executable (native, x86_64-linux-gnu, etc.)
    #[arg(long, default_value = "native")]
    pub target: String,

    /// Record the migration to the state backend
    #[arg(long)]
    pub record: bool,

    /// Preview without writing files
    #[arg(long)]
    pub dry_run: bool,

    /// Include SQL statements in output
    #[arg(long)]
    pub show_sql: bool,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub state_path: Option<PathBuf>,

    /// Allow compilation when schema drift is detected
    ///
    /// By default, compile will refuse to proceed if the database schema
    /// has drifted from the state backend (indicating manual changes).
    /// Use this flag to explicitly acknowledge and capture the drift.
    #[arg(long)]
    pub allow_drift: bool,
}

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
/// * `allow_drift` - Whether to allow compilation when drift is detected
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
    allow_drift: bool,
) -> miette::Result<()> {
    // Parse target
    let target = Target::from_str_name(target).ok_or_else(|| {
        miette!(
            "Invalid target: '{}'. Valid targets: native, x86_64-linux-gnu, x86_64-linux-musl, x86_64-macos, aarch64-macos, x86_64-windows",
            target
        )
    })?;

    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

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

    // Check for schema drift before compiling
    let source_hash = StateHash::from_namespace(&source_state);
    let target_hash = StateHash::from_namespace(&target_state);

    if source_hash != target_hash && !allow_drift {
        // Schema drift detected - warn the user
        let drift_summary = compute_drift_summary(&source_state, &target_state);

        return Err(miette!(
            help = "Run 'tern verify' to see detailed drift information.\n\
                    To proceed anyway, use --allow-drift to explicitly capture the drift.",
            "Schema drift detected between state backend and database.\n\n\
             The database has been modified outside of Tern migrations.\n\n\
             {}\n\n\
             Compiling now would capture these untracked changes as part of your migration.",
            drift_summary
        ));
    }

    if source_hash != target_hash && allow_drift && !dry_run {
        println!("WARNING: Schema drift detected. Proceeding with --allow-drift.");
    }

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

/// Compute a brief summary of schema drift for error messages.
fn compute_drift_summary(
    source: &crate::db::model::Namespace,
    target: &crate::db::model::Namespace,
) -> String {
    use crate::db::diff::diff_namespaces;

    let diff = diff_namespaces(source, target);

    let mut parts = Vec::new();

    let tables_added = diff.tables.added.len();
    let tables_removed = diff.tables.removed.len();
    let tables_modified = diff.tables.modified.len();

    if tables_added > 0 {
        parts.push(format!("{} table(s) added", tables_added));
    }
    if tables_removed > 0 {
        parts.push(format!("{} table(s) removed", tables_removed));
    }
    if tables_modified > 0 {
        parts.push(format!("{} table(s) modified", tables_modified));
    }

    let views_changed =
        diff.views.added.len() + diff.views.removed.len() + diff.views.modified.len();
    if views_changed > 0 {
        parts.push(format!("{} view(s) changed", views_changed));
    }

    let sequences_changed =
        diff.sequences.added.len() + diff.sequences.removed.len() + diff.sequences.modified.len();
    if sequences_changed > 0 {
        parts.push(format!("{} sequence(s) changed", sequences_changed));
    }

    let enums_changed =
        diff.enums.added.len() + diff.enums.removed.len() + diff.enums.modified.len();
    if enums_changed > 0 {
        parts.push(format!("{} enum(s) changed", enums_changed));
    }

    if parts.is_empty() {
        "No structural changes detected (possible metadata-only drift)".to_string()
    } else {
        format!("Drift summary: {}", parts.join(", "))
    }
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
