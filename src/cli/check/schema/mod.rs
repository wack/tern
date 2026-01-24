//! Schema check command.
//!
//! This command verifies that `.tern/schema.sql` is consistent with the
//! migrations in `.tern/migrations/`.

use std::path::PathBuf;

use clap::Args;
use miette::IntoDiagnostic;
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::checksum::compute_schema_checksum;
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::pglite::SchemaLoader;
use crate::db::state::StateBackend;
use crate::output;

/// Verify that schema.sql is consistent with migrations.
///
/// Compares the schema defined in `.tern/schema.sql` with the schema
/// that would result from replaying all migrations. Reports any drift
/// between the two.
#[derive(Debug, Clone, Args)]
pub struct CheckSchema {
    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,
}

impl CheckSchema {
    /// Dispatch the check schema command.
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Get the schema file path
        let schema_file = backend.schema_path();

        // Verify the schema file exists
        if !schema_file.exists() {
            return Err(miette::miette!(
                "Schema file not found: {}\n\nRun 'tern schema export' to generate the schema file first.",
                schema_file.display()
            ));
        }

        // Load the expected schema by replaying migrations
        // This uses the current state from the backend which represents
        // the schema after all migrations have been applied
        let migration_schema = backend.get_current_state().await.into_diagnostic()?;
        let migration_checksum = compute_schema_checksum(&migration_schema);

        // Load the schema from the schema.sql file
        let schema_file_schema = SchemaLoader::load_file(&schema_file)
            .await
            .map_err(|e| miette::miette!("Failed to load schema file: {}", e))?;
        let schema_file_checksum = compute_schema_checksum(&schema_file_schema);

        // Compare checksums
        let consistent = migration_checksum == schema_file_checksum;

        // If not consistent, compute the diff for detailed output
        let diff_summary = if !consistent {
            let diff = diff_namespaces(&migration_schema, &schema_file_schema);
            Some(DiffSummary {
                tables_added: diff.tables.added.len(),
                tables_removed: diff.tables.removed.len(),
                tables_modified: diff.tables.modified.len(),
                views_added: diff.views.added.len(),
                views_removed: diff.views.removed.len(),
                sequences_added: diff.sequences.added.len(),
                sequences_removed: diff.sequences.removed.len(),
                enums_added: diff.enums.added.len(),
                enums_removed: diff.enums.removed.len(),
            })
        } else {
            None
        };

        let check_output = CheckSchemaOutput {
            consistent,
            migration_checksum,
            schema_file_checksum,
            diff_summary,
        };

        // Output based on format
        match self.format {
            OutputFormat::Text => output!("{}", check_output),
            OutputFormat::Json => print_json(&check_output),
            OutputFormat::Sql => {
                if !consistent {
                    // Show the SQL that would bring schema.sql in line with migrations
                    let diff = diff_namespaces(&migration_schema, &schema_file_schema);
                    let plan = MigrationPlan::from_diff(&diff);
                    let renderer = PostgresRenderer::new(RenderConfig::default());
                    let script = plan.render(&renderer);
                    output!("-- SQL to transform migrations state to schema.sql state:");
                    output!("{}", script.to_sql());
                } else {
                    output!("-- No changes needed. Schema is consistent.");
                }
            }
        }

        // Exit with error code if not consistent
        if !consistent {
            std::process::exit(1);
        }

        Ok(())
    }
}

/// Output structure for check schema JSON format.
#[derive(Debug, Serialize)]
pub struct CheckSchemaOutput {
    /// Whether the schema is consistent.
    pub consistent: bool,
    /// Checksum of schema from migrations.
    pub migration_checksum: String,
    /// Checksum of schema from schema.sql.
    pub schema_file_checksum: String,
    /// Summary of differences (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<DiffSummary>,
}

/// Summary of schema differences.
#[derive(Debug, Serialize)]
pub struct DiffSummary {
    /// Number of tables added in schema.sql.
    pub tables_added: usize,
    /// Number of tables removed in schema.sql.
    pub tables_removed: usize,
    /// Number of tables modified in schema.sql.
    pub tables_modified: usize,
    /// Number of views added in schema.sql.
    pub views_added: usize,
    /// Number of views removed in schema.sql.
    pub views_removed: usize,
    /// Number of sequences added in schema.sql.
    pub sequences_added: usize,
    /// Number of sequences removed in schema.sql.
    pub sequences_removed: usize,
    /// Number of enums added in schema.sql.
    pub enums_added: usize,
    /// Number of enums removed in schema.sql.
    pub enums_removed: usize,
}

impl std::fmt::Display for CheckSchemaOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.consistent {
            writeln!(f, "Schema is consistent with migrations.")?;
        } else {
            writeln!(f, "Schema drift detected!")?;
            writeln!(f)?;
            writeln!(f, "The schema.sql file does not match the migrations.")?;

            if let Some(diff) = &self.diff_summary {
                writeln!(f)?;
                writeln!(f, "Changes in schema.sql not reflected in migrations:")?;

                if diff.tables_added > 0 {
                    writeln!(f, "  + {} table(s) added", diff.tables_added)?;
                }
                if diff.tables_removed > 0 {
                    writeln!(f, "  - {} table(s) removed", diff.tables_removed)?;
                }
                if diff.tables_modified > 0 {
                    writeln!(f, "  ~ {} table(s) modified", diff.tables_modified)?;
                }
                if diff.views_added > 0 {
                    writeln!(f, "  + {} view(s) added", diff.views_added)?;
                }
                if diff.views_removed > 0 {
                    writeln!(f, "  - {} view(s) removed", diff.views_removed)?;
                }
                if diff.sequences_added > 0 {
                    writeln!(f, "  + {} sequence(s) added", diff.sequences_added)?;
                }
                if diff.sequences_removed > 0 {
                    writeln!(f, "  - {} sequence(s) removed", diff.sequences_removed)?;
                }
                if diff.enums_added > 0 {
                    writeln!(f, "  + {} enum(s) added", diff.enums_added)?;
                }
                if diff.enums_removed > 0 {
                    writeln!(f, "  - {} enum(s) removed", diff.enums_removed)?;
                }
            }

            writeln!(f)?;
            writeln!(
                f,
                "Run 'tern generate' to create a migration for these changes,"
            )?;
            writeln!(
                f,
                "or run 'tern schema export' to regenerate schema.sql from migrations."
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_schema_output_consistent_display() {
        let output = CheckSchemaOutput {
            consistent: true,
            migration_checksum: "abc123".to_string(),
            schema_file_checksum: "abc123".to_string(),
            diff_summary: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Schema is consistent with migrations"));
    }

    #[test]
    fn check_schema_output_inconsistent_display() {
        let output = CheckSchemaOutput {
            consistent: false,
            migration_checksum: "abc123".to_string(),
            schema_file_checksum: "def456".to_string(),
            diff_summary: Some(DiffSummary {
                tables_added: 1,
                tables_removed: 0,
                tables_modified: 2,
                views_added: 0,
                views_removed: 0,
                sequences_added: 0,
                sequences_removed: 0,
                enums_added: 0,
                enums_removed: 0,
            }),
        };
        let display = format!("{}", output);
        assert!(display.contains("Schema drift detected"));
        assert!(display.contains("1 table(s) added"));
        assert!(display.contains("2 table(s) modified"));
    }

    #[test]
    fn check_schema_output_serializes() {
        let output = CheckSchemaOutput {
            consistent: true,
            migration_checksum: "abc".to_string(),
            schema_file_checksum: "abc".to_string(),
            diff_summary: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"consistent\":true"));
    }
}
