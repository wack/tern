//! Import schema changes command.
//!
//! This command imports schema changes from a live database into the migration history.

use std::path::PathBuf;

use clap::Args;
use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::diff::breaking::analyze_breaking_changes;
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{
    MigrationPlan, PostgresRenderer, RenderConfig, compute_inverse_operations,
};
use crate::db::query::{PostgresCatalog, load_namespace};
use crate::db::state::{Migration, StateBackend, StateHash};
use crate::db::{self};
use crate::{info, output};

/// Import schema changes from a live database.
///
/// Connects to a live database, compares it to the current migration state,
/// and generates a migration for any differences found. This is useful for
/// capturing changes made directly to the database.
#[derive(Debug, Clone, Args)]
pub struct Import {
    /// PostgreSQL connection string
    #[arg(env = "DATABASE_URL")]
    pub url: Option<String>,

    /// PostgreSQL connection string (alternative to positional)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// The database schema to import
    #[arg(long, default_value = "public")]
    pub schema: String,

    /// Migration description
    #[arg(long, default_value = "Import from database")]
    pub description: String,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Preview without recording the migration
    #[arg(long)]
    pub dry_run: bool,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

/// Output structure for import JSON format.
#[derive(Debug, Serialize)]
pub struct ImportOutput {
    /// Whether changes were detected.
    pub has_changes: bool,
    /// Migration ID (if recorded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration_id: Option<String>,
    /// Migration description.
    pub description: String,
    /// Number of operations in the migration.
    pub operation_count: usize,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Summary of imported changes.
    pub summary: ImportSummary,
}

/// Summary of imported changes.
#[derive(Debug, Serialize)]
pub struct ImportSummary {
    /// Number of tables added.
    pub tables_added: usize,
    /// Number of tables removed.
    pub tables_removed: usize,
    /// Number of tables modified.
    pub tables_modified: usize,
    /// Number of columns added.
    pub columns_added: usize,
    /// Number of views added.
    pub views_added: usize,
    /// Number of views removed.
    pub views_removed: usize,
    /// Number of sequences added.
    pub sequences_added: usize,
    /// Number of sequences removed.
    pub sequences_removed: usize,
    /// Number of enums added.
    pub enums_added: usize,
    /// Number of enums removed.
    pub enums_removed: usize,
}

impl std::fmt::Display for ImportOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if !self.has_changes {
            writeln!(f, "Database matches migrations. No changes to import.")?;
            return Ok(());
        }

        if self.dry_run {
            writeln!(f, "Import Preview (Dry Run)")?;
            writeln!(f, "========================")?;
        } else {
            writeln!(f, "Import Summary")?;
            writeln!(f, "==============")?;
        }
        writeln!(f)?;

        let s = &self.summary;
        if s.tables_added > 0 {
            writeln!(f, "  Tables added:    {}", s.tables_added)?;
        }
        if s.tables_removed > 0 {
            writeln!(f, "  Tables removed:  {}", s.tables_removed)?;
        }
        if s.tables_modified > 0 {
            writeln!(f, "  Tables modified: {}", s.tables_modified)?;
        }
        if s.columns_added > 0 {
            writeln!(f, "  Columns added:   {}", s.columns_added)?;
        }
        if s.views_added > 0 {
            writeln!(f, "  Views added:     {}", s.views_added)?;
        }
        if s.views_removed > 0 {
            writeln!(f, "  Views removed:   {}", s.views_removed)?;
        }
        if s.sequences_added > 0 {
            writeln!(f, "  Sequences added: {}", s.sequences_added)?;
        }
        if s.sequences_removed > 0 {
            writeln!(f, "  Sequences removed: {}", s.sequences_removed)?;
        }
        if s.enums_added > 0 {
            writeln!(f, "  Enums added:     {}", s.enums_added)?;
        }
        if s.enums_removed > 0 {
            writeln!(f, "  Enums removed:   {}", s.enums_removed)?;
        }

        writeln!(f)?;

        if self.dry_run {
            writeln!(f, "This was a dry run. No migration was recorded.")?;
            writeln!(f, "Run without --dry-run to import these changes.")?;
        } else if let Some(id) = &self.migration_id {
            writeln!(f, "Migration recorded: {}...", &id[..16.min(id.len())])?;
            writeln!(f)?;
            writeln!(f, "Run 'tern history' to view migration history.")?;
        }

        Ok(())
    }
}

impl Import {
    /// Dispatch the import command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Resolve database URL (positional > --database-url > env)
        let db_url = self
            .url
            .or(self.database_url)
            .ok_or_else(|| miette::miette!("Database URL required. Provide as argument, --database-url, or set DATABASE_URL environment variable."))?;

        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        info!("Connecting to database...");

        // Connect to the database
        let client = db::connect(&db_url)
            .await
            .into_diagnostic()
            .wrap_err("Failed to connect to database")?;

        // Load the live database schema
        let catalog = PostgresCatalog::new(&client);
        let live_schema = load_namespace(&catalog, &self.schema)
            .await
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to load schema '{}' from database", self.schema))?;

        // Load the expected schema from migrations
        let expected_schema = backend.get_current_state().await.into_diagnostic()?;

        // Compute diff (expected=source, live=target)
        // This shows what operations would transform expected -> live
        let diff = diff_namespaces(&expected_schema, &live_schema);

        // Check if there are any changes
        if diff.is_empty() {
            let import_output = ImportOutput {
                has_changes: false,
                migration_id: None,
                description: self.description.clone(),
                operation_count: 0,
                dry_run: self.dry_run,
                summary: ImportSummary {
                    tables_added: 0,
                    tables_removed: 0,
                    tables_modified: 0,
                    columns_added: 0,
                    views_added: 0,
                    views_removed: 0,
                    sequences_added: 0,
                    sequences_removed: 0,
                    enums_added: 0,
                    enums_removed: 0,
                },
            };

            match self.format {
                OutputFormat::Text => output!("{}", import_output),
                OutputFormat::Json => print_json(&import_output),
                OutputFormat::Sql => output!("-- No changes to import."),
            }

            return Ok(());
        }

        // Count columns added (sum across all modified tables)
        let columns_added: usize = diff
            .tables
            .modified
            .iter()
            .map(|m| m.columns.added.len())
            .sum();

        let summary = ImportSummary {
            tables_added: diff.tables.added.len(),
            tables_removed: diff.tables.removed.len(),
            tables_modified: diff.tables.modified.len(),
            columns_added,
            views_added: diff.views.added.len(),
            views_removed: diff.views.removed.len(),
            sequences_added: diff.sequences.added.len(),
            sequences_removed: diff.sequences.removed.len(),
            enums_added: diff.enums.added.len(),
            enums_removed: diff.enums.removed.len(),
        };

        // Analyze for breaking changes
        let analysis = analyze_breaking_changes(&diff);
        let breaking_changes = analysis.into_changes();

        // Generate migration plan
        let plan = MigrationPlan::from_diff(&diff);

        // Create the migration
        let source_hash = StateHash::from_namespace(&expected_schema);
        let target_hash = StateHash::from_namespace(&live_schema);

        // Compute inverse operations for the down migration
        let inverse_result = compute_inverse_operations(&plan.operations);
        let down_operations = inverse_result.operations;

        let migration = Migration::new(
            &self.description,
            plan.operations.clone(),
            down_operations,
            source_hash,
            target_hash,
            breaking_changes,
        );

        let migration_id = if !self.dry_run {
            // Record the migration
            backend
                .record_migration(&migration, &live_schema)
                .await
                .into_diagnostic()?;

            Some(migration.id.to_hex())
        } else {
            None
        };

        let import_output = ImportOutput {
            has_changes: true,
            migration_id,
            description: self.description,
            operation_count: migration.up_operations.len(),
            dry_run: self.dry_run,
            summary,
        };

        // Output based on format
        match self.format {
            OutputFormat::Text => output!("{}", import_output),
            OutputFormat::Json => print_json(&import_output),
            OutputFormat::Sql => {
                let renderer = PostgresRenderer::new(RenderConfig::default());
                let script = plan.render(&renderer);
                output!("-- Migration SQL to transform expected state to live database state:");
                output!("{}", script.to_sql());
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_output_no_changes_display() {
        let output = ImportOutput {
            has_changes: false,
            migration_id: None,
            description: "Import from database".to_string(),
            operation_count: 0,
            dry_run: false,
            summary: ImportSummary {
                tables_added: 0,
                tables_removed: 0,
                tables_modified: 0,
                columns_added: 0,
                views_added: 0,
                views_removed: 0,
                sequences_added: 0,
                sequences_removed: 0,
                enums_added: 0,
                enums_removed: 0,
            },
        };
        let display = format!("{}", output);
        assert!(display.contains("Database matches migrations"));
    }

    #[test]
    fn import_output_with_changes_display() {
        let output = ImportOutput {
            has_changes: true,
            migration_id: Some("abc123def456".repeat(5)),
            description: "Import from database".to_string(),
            operation_count: 5,
            dry_run: false,
            summary: ImportSummary {
                tables_added: 2,
                tables_removed: 0,
                tables_modified: 1,
                columns_added: 5,
                views_added: 0,
                views_removed: 0,
                sequences_added: 0,
                sequences_removed: 0,
                enums_added: 0,
                enums_removed: 0,
            },
        };
        let display = format!("{}", output);
        assert!(display.contains("Import Summary"));
        assert!(display.contains("Tables added:    2"));
        assert!(display.contains("Tables modified: 1"));
        assert!(display.contains("Columns added:   5"));
        assert!(display.contains("Migration recorded:"));
    }

    #[test]
    fn import_output_dry_run_display() {
        let output = ImportOutput {
            has_changes: true,
            migration_id: None,
            description: "Import from database".to_string(),
            operation_count: 1,
            dry_run: true,
            summary: ImportSummary {
                tables_added: 1,
                tables_removed: 0,
                tables_modified: 0,
                columns_added: 0,
                views_added: 0,
                views_removed: 0,
                sequences_added: 0,
                sequences_removed: 0,
                enums_added: 0,
                enums_removed: 0,
            },
        };
        let display = format!("{}", output);
        assert!(display.contains("Import Preview (Dry Run)"));
        assert!(display.contains("This was a dry run"));
    }

    #[test]
    fn import_output_serializes() {
        let output = ImportOutput {
            has_changes: true,
            migration_id: Some("abc".to_string()),
            description: "Test".to_string(),
            operation_count: 1,
            dry_run: false,
            summary: ImportSummary {
                tables_added: 1,
                tables_removed: 0,
                tables_modified: 0,
                columns_added: 0,
                views_added: 0,
                views_removed: 0,
                sequences_added: 0,
                sequences_removed: 0,
                enums_added: 0,
                enums_removed: 0,
            },
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"has_changes\":true"));
        assert!(json.contains("\"migration_id\":\"abc\""));
    }
}
