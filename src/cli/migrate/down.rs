//! Migrate down command.
//!
//! This command reverts the most recently applied migration using the
//! pre-computed down_operations stored in the migration file.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::execution::{ExecutionError, MigrationTracker, compute_migration_hash};
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::state::{Migration, StateBackend};
use crate::db::{self};

/// Revert the most recently applied migration.
///
/// Connects to a database and reverts the last applied migration.
/// Only one migration is reverted at a time for safety.
#[derive(Debug, Clone, Args)]
pub struct Down {
    /// PostgreSQL connection string
    #[arg(env = "DATABASE_URL")]
    pub url: Option<String>,

    /// PostgreSQL connection string (alternative to positional)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// The database schema to migrate
    #[arg(long, default_value = "public")]
    pub schema: String,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Show what would be reverted without actually reverting
    #[arg(long)]
    pub dry_run: bool,

    /// Skip integrity verification (dangerous)
    ///
    /// This skips both schema checksum verification and migration hash
    /// verification. Use only in emergency situations when you understand
    /// the risks of schema corruption.
    #[arg(long)]
    pub force: bool,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

/// Output structure for down command JSON format.
#[derive(Debug, Serialize)]
pub struct DownOutput {
    /// Whether the operation was successful.
    pub success: bool,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// The migration that was reverted (or would be reverted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migration: Option<RevertedMigrationInfo>,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Information about a reverted migration.
#[derive(Debug, Serialize)]
pub struct RevertedMigrationInfo {
    /// Migration ID (hex).
    pub id: String,
    /// Migration description.
    pub description: String,
    /// Number of up operations in the migration.
    pub operation_count: usize,
    /// Number of SQL statements executed to revert.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_count: Option<usize>,
}

impl std::fmt::Display for DownOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(migration) = &self.migration {
            if self.dry_run {
                writeln!(f, "Would revert migration (dry run):")?;
                writeln!(f)?;
                writeln!(
                    f,
                    "  [{}...] {}",
                    &migration.id[..12.min(migration.id.len())],
                    migration.description
                )?;
                writeln!(f, "  Operations to revert: {}", migration.operation_count)?;
                writeln!(f)?;
                writeln!(f, "Run without --dry-run to revert this migration.")?;
            } else if self.success {
                writeln!(f, "Reverted migration:")?;
                writeln!(f)?;
                writeln!(
                    f,
                    "  [{}...] {}",
                    &migration.id[..12.min(migration.id.len())],
                    migration.description
                )?;
                if let Some(stmt_count) = migration.statement_count {
                    writeln!(f, "  SQL statements executed: {}", stmt_count)?;
                }
                writeln!(f)?;
                writeln!(f, "Migration reverted successfully.")?;
            } else {
                writeln!(f, "Migration revert failed!")?;
                writeln!(f)?;
                if let Some(err) = &self.error {
                    writeln!(f, "Error: {}", err)?;
                }
            }
        } else if let Some(err) = &self.error {
            writeln!(f, "Error: {}", err)?;
        } else {
            writeln!(f, "No migrations to revert.")?;
        }

        Ok(())
    }
}

impl Down {
    /// Dispatch the down command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Resolve database URL (positional > --database-url > env)
        let db_url = self
            .url
            .or(self.database_url)
            .ok_or_else(|| miette::miette!("Database URL required. Provide as argument, --database-url, or set DATABASE_URL environment variable."))?;

        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        if !matches!(self.format, OutputFormat::Json) {
            println!("Connecting to database...");
        }

        // Connect to the database
        let client = db::connect(&db_url)
            .await
            .into_diagnostic()
            .wrap_err("Failed to connect to database")?;

        // Create tracker
        let tracker = MigrationTracker::new(&client, &self.schema);

        // Ensure tracking infrastructure exists
        tracker.ensure_schema().await.into_diagnostic()?;

        // Get the current migration ID from the database
        let current_id = match tracker.get_current_migration_id().await.into_diagnostic()? {
            Some(id) => id,
            None => {
                let output = DownOutput {
                    success: false,
                    dry_run: self.dry_run,
                    migration: None,
                    error: Some("No migrations have been applied to revert.".to_string()),
                };
                match self.format {
                    OutputFormat::Text => println!("{}", output),
                    OutputFormat::Json => print_json(&output),
                    OutputFormat::Sql => println!("-- No migrations to revert"),
                }
                std::process::exit(1);
            }
        };

        // Get the current migration record (for verification)
        let current_record = tracker
            .get_migration(&current_id)
            .await
            .into_diagnostic()?
            .ok_or_else(|| {
                miette::miette!(
                    "Migration {} is recorded as current but not found in tern.migrations table",
                    current_id
                )
            })?;

        // Find the migration in the local backend (needed for both verification and execution)
        let migration = find_migration_by_hex_id(&backend, &current_id)
            .await
            .into_diagnostic()
            .wrap_err("Failed to find migration in local state")?;

        let migration = match migration {
            Some(m) => m,
            None => {
                return Err(miette::miette!(
                    "Migration {} is recorded in the database but not found in local state.\n\nThis may indicate state corruption or that migrations were applied from a different source.",
                    current_id
                ));
            }
        };

        // Always perform verification, but handle results based on --force flag
        let schema_verification = tracker
            .verify_schema_checksum(&current_id, &current_record.schema_hash)
            .await;

        let local_hash = compute_migration_hash(&migration);
        let hash_matches = local_hash == current_record.migration_hash;

        // Determine if verification passed
        let schema_ok = schema_verification.is_ok();
        let all_ok = schema_ok && hash_matches;

        if self.force && !matches!(self.format, OutputFormat::Json) {
            if all_ok {
                println!();
                println!("Note: --force was unnecessary, all integrity checks passed.");
                println!();
            } else {
                println!();
                println!("WARNING: Integrity checks failed, but proceeding due to --force flag.");
                println!();
                if let Err(ExecutionError::SchemaDrift {
                    expected, actual, ..
                }) = &schema_verification
                {
                    println!(
                        "  Schema drift detected for migration {}...:",
                        &current_id[..12.min(current_id.len())]
                    );
                    println!("    Expected checksum: {}", expected);
                    println!("    Actual checksum:   {}", actual);
                    println!();
                }
                if !hash_matches {
                    println!(
                        "  Migration file modified for {}...:",
                        &current_id[..12.min(current_id.len())]
                    );
                    println!(
                        "    Expected hash: {}",
                        &current_record.migration_hash
                            [..16.min(current_record.migration_hash.len())]
                    );
                    println!(
                        "    Actual hash:   {}",
                        &local_hash[..16.min(local_hash.len())]
                    );
                    println!();
                }
                println!("Proceeding anyway. This can result in schema corruption or data loss.");
                println!();
            }
        } else if !self.force {
            // When not forcing, error on verification failure
            if let Err(ExecutionError::SchemaDrift {
                expected, actual, ..
            }) = schema_verification
            {
                return Err(miette::miette!(
                    "Schema drift detected!\n\nThe live database schema does not match the expected state after migration {}.\n\n  Expected checksum: {}\n  Actual checksum:   {}\n\nThis indicates the database was modified outside of Tern migrations.\n\nTo investigate: tern verify --database-url <URL>\nTo resolve:\n  Option 1: Revert manual changes to match expected state\n  Option 2: Run `tern compile` to capture changes as a new migration\n  Option 3: Use `--force` to skip verification (dangerous)",
                    &current_id[..12.min(current_id.len())],
                    expected,
                    actual
                ));
            }
            if !hash_matches {
                return Err(miette::miette!(
                    "Migration file has been modified since it was applied!\n\nMigration {} has different content than what was recorded in the database.\n\n  Expected hash: {}\n  Actual hash:   {}\n\nThis migration was applied with different operations than what's on disk.\n\nTo resolve:\n  Option 1: Restore the original migration file from version control\n  Option 2: Use `--force` to skip verification (dangerous)\n\nWarning: Reverting with mismatched history can cause schema inconsistencies between environments.",
                    &current_id[..12.min(current_id.len())],
                    current_record.migration_hash,
                    local_hash
                ));
            }
        }

        // Check if this is a baseline migration
        if migration.is_baseline() {
            return Err(miette::miette!(
                "Cannot revert the baseline migration.\n\nThe baseline migration represents the initial database state and cannot be reverted."
            ));
        }

        // Check if the migration is reversible (has pre-computed down_operations)
        if !migration.is_reversible() {
            let output = DownOutput {
                success: false,
                dry_run: self.dry_run,
                migration: Some(RevertedMigrationInfo {
                    id: migration.id.to_hex(),
                    description: migration.description.clone(),
                    operation_count: migration.up_operations.len(),
                    statement_count: None,
                }),
                error: Some(
                    "This migration contains irreversible operations and cannot be reverted."
                        .to_string(),
                ),
            };
            match self.format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => println!("-- Migration is not reversible"),
            }
            std::process::exit(1);
        }

        // Use the pre-computed down_operations
        let down_ops = migration.down_operations.clone();

        if self.dry_run {
            // Just show what would be reverted
            let output = DownOutput {
                success: true,
                dry_run: true,
                migration: Some(RevertedMigrationInfo {
                    id: migration.id.to_hex(),
                    description: migration.description.clone(),
                    operation_count: migration.up_operations.len(),
                    statement_count: None,
                }),
                error: None,
            };

            match self.format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => {
                    println!(
                        "-- Dry run: would revert migration {}",
                        migration.id.to_short_hex()
                    );
                    // Render the down operations to show what SQL would be executed
                    let renderer = PostgresRenderer::new(RenderConfig::default());
                    let plan = MigrationPlan::from_operations(down_ops);
                    let script = plan.render(&renderer);
                    println!("{}", script.to_sql());
                }
            }
        } else {
            // Execute the revert
            if !matches!(self.format, OutputFormat::Json) {
                println!("Reverting migration {}...", migration.id.to_short_hex());
            }

            // Render migration to SQL
            let renderer = PostgresRenderer::new(RenderConfig::default());
            let plan = MigrationPlan::from_operations(down_ops);
            let script = plan.render(&renderer);
            let sql = script.to_sql();
            let statement_count = script.all_statements().len();

            // Begin transaction
            client
                .execute("BEGIN", &[])
                .await
                .into_diagnostic()
                .wrap_err("Failed to begin transaction")?;

            // Set search path to target schema
            let set_search_path = format!("SET search_path TO {}", self.schema);
            if let Err(e) = client.batch_execute(&set_search_path).await {
                let _ = client.execute("ROLLBACK", &[]).await;
                return Err(miette::miette!("Failed to set search_path: {}", e));
            }

            // Execute the down operations
            if !sql.is_empty()
                && let Err(e) = client.batch_execute(&sql).await
            {
                let _ = client.execute("ROLLBACK", &[]).await;
                let output = DownOutput {
                    success: false,
                    dry_run: false,
                    migration: Some(RevertedMigrationInfo {
                        id: migration.id.to_hex(),
                        description: migration.description.clone(),
                        operation_count: migration.up_operations.len(),
                        statement_count: Some(statement_count),
                    }),
                    error: Some(e.to_string()),
                };
                match self.format {
                    OutputFormat::Text => println!("{}", output),
                    OutputFormat::Json => print_json(&output),
                    OutputFormat::Sql => println!("-- Revert failed: {}", e),
                }
                std::process::exit(1);
            }

            // Remove migration record from tracking tables
            if let Err(e) = tracker.unrecord_migration().await {
                let _ = client.execute("ROLLBACK", &[]).await;
                return Err(miette::miette!("Failed to unrecord migration: {}", e));
            }

            // Commit transaction
            client
                .execute("COMMIT", &[])
                .await
                .into_diagnostic()
                .wrap_err("Failed to commit transaction")?;

            let output = DownOutput {
                success: true,
                dry_run: false,
                migration: Some(RevertedMigrationInfo {
                    id: migration.id.to_hex(),
                    description: migration.description.clone(),
                    operation_count: migration.up_operations.len(),
                    statement_count: Some(statement_count),
                }),
                error: None,
            };

            match self.format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => {
                    println!("-- Migration reverted successfully");
                }
            }
        }

        Ok(())
    }
}

/// Find a migration by its hex ID string.
async fn find_migration_by_hex_id(
    backend: &crate::db::state::LocalFileBackend,
    hex_id: &str,
) -> Result<Option<Migration>, crate::db::state::StateError> {
    let all_migrations = backend.get_all_migrations().await?;
    Ok(all_migrations.into_iter().find(|m| m.id.to_hex() == hex_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_output_no_migrations_display() {
        let output = DownOutput {
            success: false,
            dry_run: false,
            migration: None,
            error: Some("No migrations to revert".to_string()),
        };
        let display = format!("{}", output);
        assert!(display.contains("No migrations to revert"));
    }

    #[test]
    fn down_output_dry_run_display() {
        let output = DownOutput {
            success: true,
            dry_run: true,
            migration: Some(RevertedMigrationInfo {
                id: "abc123def456".to_string(),
                description: "Add users table".to_string(),
                operation_count: 3,
                statement_count: None,
            }),
            error: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Would revert migration (dry run)"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("Run without --dry-run"));
    }

    #[test]
    fn down_output_success_display() {
        let output = DownOutput {
            success: true,
            dry_run: false,
            migration: Some(RevertedMigrationInfo {
                id: "abc123def456".to_string(),
                description: "Add users table".to_string(),
                operation_count: 3,
                statement_count: Some(5),
            }),
            error: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Reverted migration"));
        assert!(display.contains("Migration reverted successfully"));
    }

    #[test]
    fn down_output_failure_display() {
        let output = DownOutput {
            success: false,
            dry_run: false,
            migration: Some(RevertedMigrationInfo {
                id: "abc123def456".to_string(),
                description: "Add users table".to_string(),
                operation_count: 3,
                statement_count: None,
            }),
            error: Some("relation 'users' does not exist".to_string()),
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration revert failed"));
        assert!(display.contains("relation 'users' does not exist"));
    }

    #[test]
    fn down_output_serializes() {
        let output = DownOutput {
            success: true,
            dry_run: false,
            migration: Some(RevertedMigrationInfo {
                id: "abc".to_string(),
                description: "Test".to_string(),
                operation_count: 1,
                statement_count: Some(2),
            }),
            error: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"operation_count\":1"));
    }
}
