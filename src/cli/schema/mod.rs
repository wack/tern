//! Schema-related CLI commands.
//!
//! This module implements commands for working with the schema DDL file,
//! which is the foundation of the model-first migration workflow.

use anstream::println;
use std::path::PathBuf;

use miette::IntoDiagnostic;

use super::{OutputFormat, ensure_backend_initialized, load_backend};
use crate::db::state::{SchemaExporter, StateBackend};

/// Export the current schema as SQL DDL.
///
/// This command generates a `.tern/schema.sql` file containing SQL DDL
/// statements that would recreate the current schema from scratch. The
/// exported schema is useful for:
///
/// - Viewing the current schema in a human-readable format
/// - Model-first migration workflows (editing schema.sql to define changes)
/// - Documentation and code review
/// - Understanding the complete database structure
///
/// # Arguments
///
/// * `output` - Optional path to write the schema (default: `.tern/schema.sql`)
/// * `state_path` - Optional path to the state directory (default: `.tern/`)
/// * `format` - Output format (text or json for metadata)
///
/// # Example
///
/// ```bash
/// # Export to default location (.tern/schema.sql)
/// tern schema export
///
/// # Export to a custom location
/// tern schema export --output schema.sql
///
/// # Export from a specific state directory
/// tern schema export --path /path/to/project
/// ```
pub async fn run_schema_export(
    output: Option<PathBuf>,
    state_path: Option<&std::path::Path>,
    format: OutputFormat,
) -> miette::Result<()> {
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Load the current schema state
    let namespace = backend.get_current_state().await.into_diagnostic()?;

    // Generate the SQL DDL
    let sql = SchemaExporter::export(&namespace);

    // Determine output location
    let output_path = output.unwrap_or_else(|| backend.schema_path());

    // Write to file or stdout
    if output_path == std::path::Path::new("-") {
        // Write to stdout
        match format {
            OutputFormat::Sql | OutputFormat::Text => {
                println!("{sql}");
            }
            OutputFormat::Json => {
                // For JSON format, wrap in a structured output
                let output = SchemaExportOutput {
                    schema: namespace.name.as_ref().to_string(),
                    sql: sql.clone(),
                    path: None,
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).into_diagnostic()?
                );
            }
        }
    } else {
        // Write to file
        std::fs::write(&output_path, &sql)
            .into_diagnostic()
            .map_err(|e| miette::miette!("Failed to write schema file: {}", e))?;

        match format {
            OutputFormat::Text | OutputFormat::Sql => {
                println!("Schema exported to: {}", output_path.display());
                println!();
                println!("Schema: {}", namespace.name.as_ref());
                println!("Tables: {}", namespace.tables.len());
                println!("Views: {}", namespace.views.len());
                println!("Sequences: {}", namespace.sequences.len());
                println!("Enums: {}", namespace.enums.len());
            }
            OutputFormat::Json => {
                let output = SchemaExportOutput {
                    schema: namespace.name.as_ref().to_string(),
                    sql,
                    path: Some(output_path.to_string_lossy().to_string()),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).into_diagnostic()?
                );
            }
        }
    }

    Ok(())
}

/// Output structure for JSON format.
#[derive(Debug, serde::Serialize)]
struct SchemaExportOutput {
    /// The schema name (e.g., "public").
    schema: String,
    /// The generated SQL DDL.
    sql: String,
    /// The path where the schema was written (None if stdout).
    path: Option<String>,
}

// =============================================================================
// Schema Diff (PGLite-dependent)
// =============================================================================

#[cfg(feature = "pglite")]
mod pglite_commands {
    use anstream::{eprintln, println};
    use std::io::{self, Write};
    use std::path::PathBuf;

    use miette::IntoDiagnostic;
    use serde::Serialize;

    use super::super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
    use crate::db::diff::breaking::{BreakingChange, MitigationStrategy, analyze_breaking_changes};
    use crate::db::diff::diff_namespaces;
    use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
    use crate::db::pglite::SchemaLoader;
    use crate::db::state::{Migration, StateBackend, StateHash};

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

    /// Show diff between current state and edited schema.sql.
    ///
    /// This command compares the current migration state to the edited schema.sql
    /// file and displays what operations would be needed to transform the schema.
    ///
    /// # Arguments
    ///
    /// * `schema_path` - Optional path to the edited schema file
    /// * `state_path` - Optional path to the state directory
    /// * `format` - Output format
    pub async fn run_schema_diff(
        schema_path: Option<PathBuf>,
        state_path: Option<&std::path::Path>,
        format: OutputFormat,
    ) -> miette::Result<()> {
        let backend = load_backend(state_path);
        ensure_backend_initialized(&backend).await?;

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        let schema_name = source.name.as_ref().to_string();

        // Determine schema file path
        let schema_file = schema_path.unwrap_or_else(|| backend.schema_path());

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
        match format {
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

    /// Generate migration from schema changes.
    ///
    /// This command compares the current migration state to the edited schema.sql
    /// file and generates a migration that would transform the schema.
    ///
    /// # Arguments
    ///
    /// * `schema_path` - Optional path to the edited schema file
    /// * `description` - Migration description
    /// * `state_path` - Optional path to the state directory
    /// * `format` - Output format
    /// * `dry_run` - If true, preview without recording the migration
    /// * `force` - If true, skip confirmation prompt for destructive changes
    pub async fn run_schema_migrate(
        schema_path: Option<PathBuf>,
        description: &str,
        state_path: Option<&std::path::Path>,
        format: OutputFormat,
        dry_run: bool,
        force: bool,
    ) -> miette::Result<()> {
        let backend = load_backend(state_path);
        ensure_backend_initialized(&backend).await?;

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        let source_hash = StateHash::from_namespace(&source);

        // Determine schema file path
        let schema_file = schema_path.unwrap_or_else(|| backend.schema_path());

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
            match format {
                OutputFormat::Text => {
                    println!("No changes detected.");
                    println!();
                    println!("The schema file matches the current state.");
                }
                OutputFormat::Json => {
                    let output = SchemaMigrateOutput {
                        migration_id: "".to_string(),
                        description: description.to_string(),
                        operation_count: 0,
                        has_breaking_changes: false,
                        has_destructive_changes: false,
                        source_state_hash: source_hash.to_hex(),
                        target_state_hash: target_hash.to_hex(),
                        dry_run,
                        breaking_changes: vec![],
                    };
                    print_json(&output);
                }
                OutputFormat::Sql => {
                    println!("-- No changes detected.");
                }
            }
            return Ok(());
        }

        // Handle destructive changes
        let destructive_count = analysis.count_by_mitigation(MitigationStrategy::Destructive);
        if destructive_count > 0 && !force && !dry_run {
            // Show warning
            eprintln!();
            eprintln!(
                "WARNING: {} destructive change(s) detected:",
                destructive_count
            );
            eprintln!();
            for change in analysis.by_mitigation(MitigationStrategy::Destructive) {
                eprintln!("  - {}", change.description);
            }
            eprintln!();
            eprintln!("These changes will result in DATA LOSS and cannot be undone.");
            eprintln!();

            // Prompt for confirmation
            if !confirm_destructive_changes()? {
                eprintln!("Migration cancelled.");
                return Ok(());
            }
        }

        // Get the breaking changes for the migration
        let breaking_changes = analysis.into_changes();

        // Create the migration
        let migration = Migration::new(
            description,
            plan.operations.clone(),
            source_hash,
            target_hash,
            breaking_changes.clone(),
        );

        // Build output
        let breaking_change_outputs: Vec<BreakingChangeOutput> = breaking_changes
            .iter()
            .map(BreakingChangeOutput::from)
            .collect();

        let output = SchemaMigrateOutput {
            migration_id: migration.id.to_hex(),
            description: description.to_string(),
            operation_count: migration.operations.len(),
            has_breaking_changes: !breaking_changes.is_empty(),
            has_destructive_changes: destructive_count > 0,
            source_state_hash: source_hash.to_hex(),
            target_state_hash: target_hash.to_hex(),
            dry_run,
            breaking_changes: breaking_change_outputs,
        };

        // Record the migration (unless dry run)
        if !dry_run {
            backend
                .record_migration(&migration, &target)
                .await
                .into_diagnostic()?;
        }

        // Output based on format
        match format {
            OutputFormat::Text => println!("{}", output),
            OutputFormat::Json => print_json(&output),
            OutputFormat::Sql => {
                let renderer = PostgresRenderer::new(RenderConfig::default());
                let script = plan.render(&renderer);
                println!("{}", script.to_sql());
            }
        }

        Ok(())
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
}

#[cfg(feature = "pglite")]
pub use pglite_commands::{run_schema_diff, run_schema_migrate};

#[cfg(test)]
mod tests {
    use super::*;

    mod schema_export_output_tests {
        use super::*;

        #[test]
        fn schema_export_output_serializes() {
            let output = SchemaExportOutput {
                schema: "public".to_string(),
                sql: "CREATE TABLE foo (id int);".to_string(),
                path: Some(".tern/schema.sql".to_string()),
            };
            let json = serde_json::to_string(&output).unwrap();
            assert!(json.contains("\"schema\":\"public\""));
            assert!(json.contains("\"sql\":\"CREATE TABLE foo"));
        }

        #[test]
        fn schema_export_output_path_optional() {
            let output = SchemaExportOutput {
                schema: "public".to_string(),
                sql: "".to_string(),
                path: None,
            };
            let json = serde_json::to_string(&output).unwrap();
            assert!(json.contains("\"path\":null"));
        }
    }

    #[cfg(feature = "pglite")]
    mod pglite_command_tests {
        use super::pglite_commands::*;

        mod schema_diff_output_tests {
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

        mod schema_migrate_output_tests {
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

        mod breaking_change_output_tests {
            use super::*;
            use crate::db::diff::breaking::{BreakingChange, BreakingChangeKind};
            use crate::db::schema::TableName;

            #[test]
            fn from_breaking_change() {
                let table = TableName::try_new("users".to_string()).unwrap();
                let bc = BreakingChange::new(BreakingChangeKind::TableDropped { table });
                let output = BreakingChangeOutput::from(&bc);

                assert!(output.description.contains("users"));
                assert_eq!(output.mitigation, "destructive");
            }
        }
    }
}
