//! Migrate down command.
//!
//! This command reverts the most recently applied migration.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::execution::MigrationTracker;
use crate::db::migrate::{MigrationPlan, Operation, PostgresRenderer, RenderConfig};
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
    /// Number of operations that were reverted.
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

/// Computes the inverse operations needed to revert a migration.
///
/// Returns the operations in reverse order (last applied first reverted).
fn compute_inverse_operations(operations: &[Operation]) -> Result<Vec<Operation>, String> {
    let mut inverse_ops = Vec::new();

    // Process operations in reverse order
    for op in operations.iter().rev() {
        let inverse = compute_single_inverse(op)?;
        inverse_ops.push(inverse);
    }

    Ok(inverse_ops)
}

/// Computes the inverse of a single operation.
fn compute_single_inverse(op: &Operation) -> Result<Operation, String> {
    match op {
        // Enum operations
        Operation::CreateEnum { schema, enum_type } => Ok(Operation::DropEnum {
            schema: schema.clone(),
            name: enum_type.name.clone(),
        }),
        Operation::DropEnum { .. } => {
            Err("Cannot revert DropEnum: enum definition is not preserved".to_string())
        }
        Operation::RenameEnum { schema, from, to } => Ok(Operation::RenameEnum {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AddEnumValue { .. } => Err(
            "Cannot revert AddEnumValue: PostgreSQL does not support removing enum values"
                .to_string(),
        ),

        // Sequence operations
        Operation::CreateSequence { schema, sequence } => Ok(Operation::DropSequence {
            schema: schema.clone(),
            name: sequence.name.clone(),
        }),
        Operation::DropSequence { .. } => {
            Err("Cannot revert DropSequence: sequence definition is not preserved".to_string())
        }
        Operation::RenameSequence { schema, from, to } => Ok(Operation::RenameSequence {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AlterSequence { .. } => {
            Err("Cannot revert AlterSequence: previous values are not preserved".to_string())
        }

        // Table operations
        Operation::CreateTable { schema, table } => Ok(Operation::DropTable {
            schema: schema.clone(),
            name: table.name.clone(),
        }),
        Operation::DropTable { .. } => {
            Err("Cannot revert DropTable: table definition and data are not preserved".to_string())
        }
        Operation::RenameTable { schema, from, to } => Ok(Operation::RenameTable {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // Column operations
        Operation::AddColumn {
            schema,
            table,
            column,
        } => Ok(Operation::DropColumn {
            schema: schema.clone(),
            table: table.clone(),
            name: column.name.clone(),
        }),
        Operation::DropColumn { .. } => Err(
            "Cannot revert DropColumn: column definition and data are not preserved".to_string(),
        ),
        Operation::RenameColumn {
            schema,
            table,
            from,
            to,
        } => Ok(Operation::RenameColumn {
            schema: schema.clone(),
            table: table.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AlterColumn { .. } => {
            Err("Cannot revert AlterColumn: previous values are not preserved".to_string())
        }

        // Constraint operations
        Operation::AddConstraint {
            schema,
            table,
            constraint,
        } => Ok(Operation::DropConstraint {
            schema: schema.clone(),
            table: table.clone(),
            name: constraint.name.clone(),
        }),
        Operation::DropConstraint { .. } => {
            Err("Cannot revert DropConstraint: constraint definition is not preserved".to_string())
        }
        Operation::RenameConstraint {
            schema,
            table,
            from,
            to,
        } => Ok(Operation::RenameConstraint {
            schema: schema.clone(),
            table: table.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // Index operations
        Operation::CreateIndex {
            schema,
            index,
            concurrently,
            ..
        } => Ok(Operation::DropIndex {
            schema: schema.clone(),
            name: index.name.clone(),
            concurrently: *concurrently,
        }),
        Operation::DropIndex { .. } => {
            Err("Cannot revert DropIndex: index definition is not preserved".to_string())
        }
        Operation::RenameIndex { schema, from, to } => Ok(Operation::RenameIndex {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // View operations
        Operation::CreateView { schema, view } => Ok(Operation::DropView {
            schema: schema.clone(),
            name: view.name.clone(),
            is_materialized: view.is_materialized,
        }),
        Operation::DropView { .. } => {
            Err("Cannot revert DropView: view definition is not preserved".to_string())
        }
        Operation::RenameView {
            schema,
            from,
            to,
            is_materialized,
        } => Ok(Operation::RenameView {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
            is_materialized: *is_materialized,
        }),
        Operation::ReplaceView { .. } => {
            Err("Cannot revert ReplaceView: previous view definition is not preserved".to_string())
        }
        Operation::RefreshMaterializedView { .. } => {
            // Refreshing a materialized view is idempotent, so we can just skip it
            // Return a no-op by creating a comment that won't change anything
            // Actually, there's no true no-op, so we'll just error
            Err(
                "Cannot revert RefreshMaterializedView: this operation cannot be undone"
                    .to_string(),
            )
        }

        // Comment operations
        Operation::SetComment { .. } => {
            Err("Cannot revert SetComment: previous comment is not preserved".to_string())
        }
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

        // Find the migration in the local backend
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

        // Check if this is a baseline migration
        if migration.is_baseline() {
            return Err(miette::miette!(
                "Cannot revert the baseline migration.\n\nThe baseline migration represents the initial database state and cannot be reverted."
            ));
        }

        // Compute inverse operations
        let inverse_ops = match compute_inverse_operations(&migration.operations) {
            Ok(ops) => ops,
            Err(msg) => {
                let output = DownOutput {
                    success: false,
                    dry_run: self.dry_run,
                    migration: Some(RevertedMigrationInfo {
                        id: migration.id.to_hex(),
                        description: migration.description.clone(),
                        operation_count: migration.operations.len(),
                        statement_count: None,
                    }),
                    error: Some(msg),
                };
                match self.format {
                    OutputFormat::Text => println!("{}", output),
                    OutputFormat::Json => print_json(&output),
                    OutputFormat::Sql => println!("-- Cannot compute inverse operations"),
                }
                std::process::exit(1);
            }
        };

        if self.dry_run {
            // Just show what would be reverted
            let output = DownOutput {
                success: true,
                dry_run: true,
                migration: Some(RevertedMigrationInfo {
                    id: migration.id.to_hex(),
                    description: migration.description.clone(),
                    operation_count: migration.operations.len(),
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
                    // Render the inverse operations to show what SQL would be executed
                    let renderer = PostgresRenderer::new(RenderConfig::default());
                    let plan = MigrationPlan::from_operations(inverse_ops);
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
            let plan = MigrationPlan::from_operations(inverse_ops);
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

            // Execute the inverse operations
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
                        operation_count: migration.operations.len(),
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
                    operation_count: migration.operations.len(),
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
