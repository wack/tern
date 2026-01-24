//! Schema migrate command.
//!
//! This command generates a migration from schema changes.

use std::io::{self, Write};
use std::path::PathBuf;

use clap::Args;
use miette::IntoDiagnostic;
use serde::Serialize;

use super::diff::BreakingChangeOutput;
use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::diff::breaking::{MitigationStrategy, analyze_breaking_changes};
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{
    MigrationPlan, PostgresRenderer, RenderConfig, compute_inverse_operations,
};
use crate::db::pglite::SchemaLoader;
use crate::db::state::{Migration, StateBackend, StateHash};
use crate::{info, newline, output, warn};

/// Generate migration from schema changes
///
/// Compares the current migration state to the edited schema.sql file
/// and generates a migration that would transform the schema. This is
/// the core of the model-first migration workflow.
#[derive(Debug, Clone, Args)]
pub struct Migrate {
    /// Path to the edited schema file (default: .tern/schema.sql)
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Migration description
    #[arg(short, long)]
    pub description: String,

    /// Output format (text shows summary, sql shows migration SQL, json includes metadata)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Preview without recording the migration
    #[arg(long)]
    pub dry_run: bool,

    /// Skip confirmation prompt for destructive changes
    #[arg(long)]
    pub force: bool,
}

impl Migrate {
    /// Dispatch the schema migrate command.
    pub async fn dispatch(self) -> miette::Result<()> {
        warn!("'schema migrate' is deprecated. Use 'tern generate' instead.");

        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        let source_hash = StateHash::from_namespace(&source);

        // Determine schema file path
        let schema_file = self.schema.unwrap_or_else(|| backend.schema_path());

        // Verify the schema file exists
        if !schema_file.exists() {
            return Err(miette::miette!(
                "Schema file not found: {}\n\nRun 'tern schema export' to generate the schema file first.",
                schema_file.display()
            ));
        }

        // Load the target state (edited schema file)
        let target = SchemaLoader::load_file(&schema_file)
            .await
            .map_err(|e| miette::miette!("Failed to load schema file: {}", e))?;
        let target_hash = StateHash::from_namespace(&target);

        // Generate diff
        let diff = diff_namespaces(&source, &target);

        // Analyze for breaking changes
        let analysis = analyze_breaking_changes(&diff);

        // Generate migration plan
        let plan = MigrationPlan::from_diff(&diff);

        // Check if there are any changes
        if plan.is_empty() {
            match self.format {
                OutputFormat::Text => {
                    info!("No changes detected.");
                    newline!();
                    info!("The schema file matches the current state.");
                }
                OutputFormat::Json => {
                    let output = SchemaMigrateOutput {
                        migration_id: "".to_string(),
                        description: self.description.clone(),
                        operation_count: 0,
                        has_breaking_changes: false,
                        has_destructive_changes: false,
                        source_state_hash: source_hash.to_hex(),
                        target_state_hash: target_hash.to_hex(),
                        dry_run: self.dry_run,
                        breaking_changes: vec![],
                    };
                    print_json(&output);
                }
                OutputFormat::Sql => {
                    output!("-- No changes detected.");
                }
            }
            return Ok(());
        }

        // Handle destructive changes
        let destructive_count = analysis.count_by_mitigation(MitigationStrategy::Destructive);
        if destructive_count > 0 && !self.force && !self.dry_run {
            // Show warning
            newline!();
            warn!("{} destructive change(s) detected:", destructive_count);
            newline!();
            for change in analysis.by_mitigation(MitigationStrategy::Destructive) {
                warn!("  - {}", change.description);
            }
            newline!();
            warn!("These changes will result in DATA LOSS and cannot be undone.");
            newline!();

            // Prompt for confirmation
            if !confirm_destructive_changes()? {
                info!("Migration cancelled.");
                return Ok(());
            }
        }

        // Get the breaking changes for the migration
        let breaking_changes = analysis.into_changes();

        // Compute inverse operations for the down migration
        let inverse_result = compute_inverse_operations(&plan.operations);
        let down_operations = inverse_result.operations;

        // Create the migration
        let migration = Migration::new(
            &self.description,
            plan.operations.clone(),
            down_operations,
            source_hash,
            target_hash,
            breaking_changes.clone(),
        );

        // Build output
        let breaking_change_outputs: Vec<BreakingChangeOutput> = breaking_changes
            .iter()
            .map(BreakingChangeOutput::from)
            .collect();

        let migrate_output = SchemaMigrateOutput {
            migration_id: migration.id.to_hex(),
            description: self.description.clone(),
            operation_count: migration.up_operations.len(),
            has_breaking_changes: !breaking_changes.is_empty(),
            has_destructive_changes: destructive_count > 0,
            source_state_hash: source_hash.to_hex(),
            target_state_hash: target_hash.to_hex(),
            dry_run: self.dry_run,
            breaking_changes: breaking_change_outputs,
        };

        // Record the migration (unless dry run)
        if !self.dry_run {
            backend
                .record_migration(&migration, &target)
                .await
                .into_diagnostic()?;
        }

        // Output based on format
        match self.format {
            OutputFormat::Text => output!("{}", migrate_output),
            OutputFormat::Json => print_json(&migrate_output),
            OutputFormat::Sql => {
                let renderer = PostgresRenderer::new(RenderConfig::default());
                let script = plan.render(&renderer);
                output!("{}", script.to_sql());
            }
        }

        Ok(())
    }
}

/// Output structure for schema migrate JSON format.
#[derive(Debug, Serialize)]
pub struct SchemaMigrateOutput {
    /// Migration ID (hex).
    pub migration_id: String,
    /// Migration description.
    pub description: String,
    /// Number of operations in the migration.
    pub operation_count: usize,
    /// Whether there are breaking changes.
    pub has_breaking_changes: bool,
    /// Whether there are destructive changes.
    pub has_destructive_changes: bool,
    /// Source state hash.
    pub source_state_hash: String,
    /// Target state hash.
    pub target_state_hash: String,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Breaking changes with details.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breaking_changes: Vec<BreakingChangeOutput>,
}

impl std::fmt::Display for SchemaMigrateOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.dry_run {
            writeln!(f, "Migration Preview (Dry Run)")?;
            writeln!(f, "===========================")?;
        } else {
            writeln!(f, "Migration Created")?;
            writeln!(f, "=================")?;
        }
        writeln!(f)?;
        writeln!(f, "  ID:          {}", self.migration_id)?;
        writeln!(f, "  Description: {}", self.description)?;
        writeln!(f, "  Operations:  {}", self.operation_count)?;
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

        if self.dry_run {
            writeln!(f)?;
            writeln!(f, "This was a dry run. No migration was recorded.")?;
            writeln!(f, "Run without --dry-run to create the migration.")?;
        }

        Ok(())
    }
}

/// Prompts the user to confirm destructive changes.
///
/// Returns `true` if the user confirms, `false` otherwise.
fn confirm_destructive_changes() -> miette::Result<bool> {
    eprint!("Continue with destructive changes? [y/N] ");
    io::stderr().flush().into_diagnostic()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).into_diagnostic()?;

    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_output_display() {
        let output = SchemaMigrateOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            operation_count: 3,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            dry_run: false,
            breaking_changes: vec![],
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration Created"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("Operations:  3"));
        assert!(!display.contains("dry run"));
    }

    #[test]
    fn migrate_dry_run_display() {
        let output = SchemaMigrateOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            operation_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            dry_run: true,
            breaking_changes: vec![],
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration Preview (Dry Run)"));
        assert!(display.contains("This was a dry run"));
        assert!(display.contains("Run without --dry-run"));
    }

    #[test]
    fn migrate_with_breaking_changes_display() {
        let output = SchemaMigrateOutput {
            migration_id: "abc123".to_string(),
            description: "Drop legacy".to_string(),
            operation_count: 2,
            has_breaking_changes: true,
            has_destructive_changes: true,
            source_state_hash: "src".repeat(16),
            target_state_hash: "tgt".repeat(16),
            dry_run: false,
            breaking_changes: vec![BreakingChangeOutput {
                description: "Table dropped".to_string(),
                mitigation: "destructive".to_string(),
            }],
        };
        let display = format!("{}", output);
        assert!(display.contains("WARNING: Breaking Changes"));
        assert!(display.contains("[destructive]"));
        assert!(display.contains("Table dropped"));
    }

    #[test]
    fn migrate_output_serializes() {
        let output = SchemaMigrateOutput {
            migration_id: "abc".to_string(),
            description: "Test".to_string(),
            operation_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "src".to_string(),
            target_state_hash: "tgt".to_string(),
            dry_run: false,
            breaking_changes: vec![],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"migration_id\":\"abc\""));
        assert!(json.contains("\"description\":\"Test\""));
    }

    #[test]
    fn migrate_output_skips_empty_breaking_changes() {
        let output = SchemaMigrateOutput {
            migration_id: "abc".to_string(),
            description: "Test".to_string(),
            operation_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            source_state_hash: "src".to_string(),
            target_state_hash: "tgt".to_string(),
            dry_run: false,
            breaking_changes: vec![],
        };
        let json = serde_json::to_string(&output).unwrap();
        // Empty breaking_changes should be skipped due to serde skip_serializing_if
        assert!(!json.contains("\"breaking_changes\""));
    }
}
