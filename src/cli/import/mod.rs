//! Import schema changes command.
//!
//! This command imports schema changes from a live database into the migration history.

use std::path::PathBuf;

use anstream::println;
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

    /// Skip integrity verification (dangerous)
    #[arg(long)]
    pub force: bool,

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
        // Extract all values at the start to avoid partial move issues
        let force = self.force;
        let dry_run = self.dry_run;
        let format = self.format;
        let description = self.description;
        let schema = self.schema;
        let path = self.path;

        // Resolve database URL (positional > --database-url > env)
        let db_url = self
            .url
            .or(self.database_url)
            .ok_or_else(|| miette::miette!("Database URL required. Provide as argument, --database-url, or set DATABASE_URL environment variable."))?;

        let backend = load_backend(path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Verify local migration chain integrity (unless --force)
        if !force {
            verify_chain_integrity(&backend).await?;
        }

        println!("Connecting to database...");

        // Connect to the database
        let client = db::connect(&db_url)
            .await
            .into_diagnostic()
            .wrap_err("Failed to connect to database")?;

        // Verify migration hashes against database (unless --force)
        if !force {
            verify_migration_hashes(&backend, &client, &schema).await?;
        }

        // Load the live database schema
        let catalog = PostgresCatalog::new(&client);
        let live_schema = load_namespace(&catalog, &schema)
            .await
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to load schema '{}' from database", schema))?;

        // Load the expected schema from migrations
        let expected_schema = backend.get_current_state().await.into_diagnostic()?;

        // Compute diff (expected=source, live=target)
        // This shows what operations would transform expected -> live
        let diff = diff_namespaces(&expected_schema, &live_schema);

        // Check if there are any changes
        if diff.is_empty() {
            let output = ImportOutput {
                has_changes: false,
                migration_id: None,
                description: description.clone(),
                operation_count: 0,
                dry_run,
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

            match format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => println!("-- No changes to import."),
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
            &description,
            plan.operations.clone(),
            down_operations,
            source_hash,
            target_hash,
            breaking_changes,
        );

        let migration_id = if !dry_run {
            // Record the migration
            backend
                .record_migration(&migration, &live_schema)
                .await
                .into_diagnostic()?;

            Some(migration.id.to_hex())
        } else {
            None
        };

        let output = ImportOutput {
            has_changes: true,
            migration_id,
            description,
            operation_count: migration.up_operations.len(),
            dry_run,
            summary,
        };

        // Output based on format
        match format {
            OutputFormat::Text => println!("{}", output),
            OutputFormat::Json => print_json(&output),
            OutputFormat::Sql => {
                let renderer = PostgresRenderer::new(RenderConfig::default());
                let script = plan.render(&renderer);
                println!("-- Migration SQL to transform expected state to live database state:");
                println!("{}", script.to_sql());
            }
        }

        Ok(())
    }
}

/// Verifies the integrity of the local migration chain.
///
/// Checks that each migration's parent_state_hash matches the previous
/// migration's resulting_state_hash.
async fn verify_chain_integrity<B: StateBackend>(backend: &B) -> miette::Result<()> {
    use crate::db::state::StateError;

    match backend.verify_chain().await {
        Ok(()) => Ok(()),
        Err(StateError::BrokenChain {
            id,
            parent,
            expected,
        }) => Err(miette::miette!(
            "Migration chain integrity check failed\n\n\
             Migration {} has an invalid parent hash.\n\n\
             Expected parent: {}...\n\
             Actual parent:   {}...\n\n\
             This indicates the migration file was modified or corrupted.\n\n\
             To resolve:\n  \
             Option 1: Restore the migration from version control\n  \
             Option 2: Run 'tern verify chain' for detailed diagnostics\n  \
             Option 3: Use --force to skip verification (dangerous)",
            &id.to_hex()[..16.min(id.to_hex().len())],
            &expected.to_hex()[..16.min(expected.to_hex().len())],
            &parent.to_hex()[..16.min(parent.to_hex().len())],
        )),
        Err(e) => Err(miette::miette!("Failed to verify migration chain: {}", e)),
    }
}

/// Verifies that local migration files match what was applied to the database.
///
/// Compares the BLAKE3 hash of each local migration's up_operations against
/// the migration_hash stored in the tern.migrations table.
async fn verify_migration_hashes<B: StateBackend>(
    backend: &B,
    client: &tokio_postgres::Client,
    target_schema: &str,
) -> miette::Result<()> {
    use crate::db::execution::{MigrationTracker, compute_migration_hash};

    let tracker = MigrationTracker::new(client, target_schema);

    // Check if tracking tables exist (fresh database)
    if tracker.ensure_schema().await.is_err() {
        // Fresh database, nothing to verify against
        return Ok(());
    }

    // Get all recorded migrations from the database
    let recorded = match tracker.get_all_migrations().await {
        Ok(migrations) => migrations,
        Err(_) => {
            // No tern.migrations table, nothing to verify
            return Ok(());
        }
    };

    // If no migrations recorded, nothing to verify
    if recorded.is_empty() {
        return Ok(());
    }

    // Load all local migrations
    let local_migrations = backend.get_all_migrations().await.into_diagnostic()?;

    // Check if database has more migrations than local
    let local_count = local_migrations.len();
    let recorded_count = recorded.len();

    if recorded_count > local_count {
        return Err(miette::miette!(
            "Database has migrations not present locally\n\n\
             The database has {} migrations applied, but only {} are present locally.\n\
             Your local migration history is behind the database.\n\n\
             To resolve:\n  \
             Pull the latest migrations from your team's repository.",
            recorded_count,
            local_count
        ));
    }

    // Build hash map of local migrations
    let local_hashes: Vec<(crate::db::state::MigrationId, String)> = local_migrations
        .iter()
        .map(|m| (m.id, compute_migration_hash(m)))
        .collect();

    // Verify each recorded migration
    let diverged = tracker
        .verify_history(&local_hashes)
        .await
        .map_err(|e| miette::miette!("Failed to verify migration history: {}", e))?;

    if let Some((id, expected, actual)) = diverged.first() {
        return Err(miette::miette!(
            "Local migration files don't match applied migrations\n\n\
             Migration {} has been modified since it was applied to this database.\n\n\
             Applied hash:  {}...\n\
             Local hash:    {}...\n\n\
             The local migration file differs from what was applied to the database.\n\
             Running 'import' would generate an incorrect migration.\n\n\
             To resolve:\n  \
             Option 1: Restore the migration from version control\n  \
             Option 2: Pull the correct migration files from the team\n  \
             Option 3: Use --force to skip verification (dangerous)\n\n\
             Warning: Proceeding with mismatched migrations can cause schema\n\
             inconsistencies between environments.",
            &id[..16.min(id.len())],
            &expected[..16.min(expected.len())],
            &actual[..16.min(actual.len())],
        ));
    }

    Ok(())
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

    mod chain_verification_tests {
        use super::*;
        use crate::db::model::Namespace;
        use crate::db::state::{InMemoryBackend, Migration, StateHash};

        #[tokio::test]
        async fn verify_chain_integrity_passes_for_valid_chain() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                vec![],
                m1.resulting_state_hash,
                StateHash::from_bytes([22u8; 32]),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            // Should pass without error
            let result = verify_chain_integrity(&backend).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn verify_chain_integrity_fails_for_broken_chain() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            // Create a migration with wrong parent hash
            let m2 = Migration::new(
                "Second",
                vec![],
                vec![],
                StateHash::from_bytes([99u8; 32]), // Wrong parent hash
                StateHash::from_bytes([22u8; 32]),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            // Should fail with error message
            let result = verify_chain_integrity(&backend).await;
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Migration chain integrity check failed"));
            assert!(err.contains("invalid parent hash"));
        }

        #[tokio::test]
        async fn verify_chain_integrity_passes_for_empty_history() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            // Empty history should pass (nothing to verify)
            let result = verify_chain_integrity(&backend).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn verify_chain_integrity_passes_for_single_migration() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            // Single migration should pass
            let result = verify_chain_integrity(&backend).await;
            assert!(result.is_ok());
        }
    }
}
