//! Schema diff command.
//!
//! This command shows the diff between current state and edited schema.sql.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::IntoDiagnostic;
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::diff::breaking::{BreakingChange, MitigationStrategy, analyze_breaking_changes};
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::pglite::SchemaLoader;
use crate::db::state::StateBackend;

/// Show diff between current state and edited schema.sql
///
/// Compares the current migration state to the edited schema.sql file
/// and displays what operations would be needed to transform the schema.
/// This is the preview step before generating a migration.
#[derive(Debug, Clone, Args)]
pub struct Diff {
    /// Path to the edited schema file (default: .tern/schema.sql)
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Output format (text shows summary, sql shows migration SQL, json includes metadata)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,
}

impl Diff {
    /// Dispatch the schema diff command.
    pub async fn dispatch(self) -> miette::Result<()> {
        anstream::eprintln!(
            "WARNING: 'schema diff' is deprecated. Use 'tern check schema' instead."
        );

        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        let schema_name = source.name.as_ref().to_string();

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

        // Generate diff
        let diff = diff_namespaces(&source, &target);

        // Analyze for breaking changes
        let analysis = analyze_breaking_changes(&diff);

        // Generate migration plan
        let plan = MigrationPlan::from_diff(&diff);

        // Render to SQL for display
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let script = plan.render(&renderer);

        // Build output
        let breaking_changes: Vec<BreakingChangeOutput> =
            analysis.iter().map(BreakingChangeOutput::from).collect();

        let destructive_count = analysis.count_by_mitigation(MitigationStrategy::Destructive);

        let operations: Vec<String> = script
            .descriptions()
            .into_iter()
            .map(String::from)
            .collect();

        let output = SchemaDiffOutput {
            schema: schema_name,
            operation_count: plan.len(),
            breaking_change_count: analysis.len(),
            destructive_change_count: destructive_count,
            is_empty: plan.is_empty(),
            operations,
            breaking_changes,
            sql: Some(script.to_sql()),
        };

        // Output based on format
        match self.format {
            OutputFormat::Text => println!("{}", output),
            OutputFormat::Json => print_json(&output),
            OutputFormat::Sql => {
                if plan.is_empty() {
                    println!("-- No changes detected.");
                } else {
                    println!("{}", script.to_sql());
                }
            }
        }

        Ok(())
    }
}

/// Output structure for schema diff JSON format.
#[derive(Debug, Serialize)]
pub struct SchemaDiffOutput {
    /// The schema name (e.g., "public").
    pub schema: String,
    /// Number of operations in the migration.
    pub operation_count: usize,
    /// Number of breaking changes detected.
    pub breaking_change_count: usize,
    /// Number of destructive changes detected.
    pub destructive_change_count: usize,
    /// Whether the diff is empty (no changes).
    pub is_empty: bool,
    /// Descriptions of each operation.
    pub operations: Vec<String>,
    /// Breaking changes with details.
    pub breaking_changes: Vec<BreakingChangeOutput>,
    /// The SQL migration (if requested).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
}

/// Output structure for breaking changes in JSON format.
#[derive(Debug, Serialize)]
pub struct BreakingChangeOutput {
    /// Description of the breaking change.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: String,
}

impl From<&BreakingChange> for BreakingChangeOutput {
    fn from(bc: &BreakingChange) -> Self {
        Self {
            description: bc.description.clone(),
            mitigation: bc.mitigation.as_str().to_string(),
        }
    }
}

impl std::fmt::Display for SchemaDiffOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty {
            writeln!(f, "No changes detected.")?;
            writeln!(f)?;
            writeln!(f, "The schema file matches the current state.")?;
            return Ok(());
        }

        writeln!(f, "Schema Diff")?;
        writeln!(f, "===========")?;
        writeln!(f)?;
        writeln!(f, "  Schema:     {}", self.schema)?;
        writeln!(f, "  Operations: {}", self.operation_count)?;

        if self.breaking_change_count > 0 {
            writeln!(f)?;
            writeln!(
                f,
                "WARNING: {} Breaking Change(s) Detected",
                self.breaking_change_count
            )?;
            writeln!(f, "----------------------------------------")?;
            for bc in &self.breaking_changes {
                writeln!(f, "  [{}] {}", bc.mitigation, bc.description)?;
            }
        }

        if !self.operations.is_empty() {
            writeln!(f)?;
            writeln!(f, "Operations:")?;
            for (i, op) in self.operations.iter().enumerate() {
                writeln!(f, "  {}. {}", i + 1, op)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diff_display() {
        let output = SchemaDiffOutput {
            schema: "public".to_string(),
            operation_count: 0,
            breaking_change_count: 0,
            destructive_change_count: 0,
            is_empty: true,
            operations: vec![],
            breaking_changes: vec![],
            sql: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("No changes detected"));
        assert!(display.contains("schema file matches"));
    }

    #[test]
    fn non_empty_diff_display() {
        let output = SchemaDiffOutput {
            schema: "public".to_string(),
            operation_count: 2,
            breaking_change_count: 0,
            destructive_change_count: 0,
            is_empty: false,
            operations: vec![
                "Create table users".to_string(),
                "Create index on users".to_string(),
            ],
            breaking_changes: vec![],
            sql: Some("CREATE TABLE users...".to_string()),
        };
        let display = format!("{}", output);
        assert!(display.contains("Schema Diff"));
        assert!(display.contains("public"));
        assert!(display.contains("Operations: 2"));
        assert!(display.contains("1. Create table users"));
        assert!(display.contains("2. Create index on users"));
    }

    #[test]
    fn diff_with_breaking_changes_display() {
        let output = SchemaDiffOutput {
            schema: "public".to_string(),
            operation_count: 1,
            breaking_change_count: 2,
            destructive_change_count: 1,
            is_empty: false,
            operations: vec!["Drop table users".to_string()],
            breaking_changes: vec![
                BreakingChangeOutput {
                    description: "Table 'users' was dropped".to_string(),
                    mitigation: "destructive".to_string(),
                },
                BreakingChangeOutput {
                    description: "Column 'posts.author_id' was dropped".to_string(),
                    mitigation: "destructive".to_string(),
                },
            ],
            sql: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("WARNING: 2 Breaking Change(s)"));
        assert!(display.contains("[destructive]"));
        assert!(display.contains("users"));
    }

    #[test]
    fn schema_diff_output_serializes() {
        let output = SchemaDiffOutput {
            schema: "public".to_string(),
            operation_count: 1,
            breaking_change_count: 0,
            destructive_change_count: 0,
            is_empty: false,
            operations: vec!["Create table".to_string()],
            breaking_changes: vec![],
            sql: Some("SQL".to_string()),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"schema\":\"public\""));
        assert!(json.contains("\"operation_count\":1"));
        assert!(json.contains("\"is_empty\":false"));
    }
}
