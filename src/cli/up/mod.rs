//! Run migrations command.
//!
//! This command runs pending migrations against a live database.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::execution::MigrationExecutor;
use crate::db::state::StateBackend;
use crate::db::{self};

/// Run pending migrations against a live database.
///
/// Connects to a database and applies all migrations that haven't been
/// applied yet. Each migration runs in its own transaction.
#[derive(Debug, Clone, Args)]
pub struct Up {
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

    /// Show pending migrations without applying
    #[arg(long)]
    pub dry_run: bool,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

/// Output structure for up command JSON format.
#[derive(Debug, Serialize)]
pub struct UpOutput {
    /// Whether the operation was successful.
    pub success: bool,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Number of migrations applied or pending.
    pub migration_count: usize,
    /// Details of each migration.
    pub migrations: Vec<MigrationInfo>,
    /// Error message if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Information about a single migration.
#[derive(Debug, Serialize)]
pub struct MigrationInfo {
    /// Migration ID (hex).
    pub id: String,
    /// Migration description.
    pub description: String,
    /// Number of SQL statements (if applied).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_count: Option<usize>,
    /// Schema hash after applying (if applied).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_hash: Option<String>,
    /// Whether this migration was applied.
    pub applied: bool,
}

impl std::fmt::Display for UpOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.migration_count == 0 {
            writeln!(f, "Database is up to date. No pending migrations.")?;
            return Ok(());
        }

        if self.dry_run {
            writeln!(f, "Pending migrations (dry run):")?;
            writeln!(f)?;
            for (i, m) in self.migrations.iter().enumerate() {
                writeln!(
                    f,
                    "  {}. [{}...] {}",
                    i + 1,
                    &m.id[..12.min(m.id.len())],
                    m.description
                )?;
            }
            writeln!(f)?;
            writeln!(f, "Run without --dry-run to apply these migrations.")?;
        } else if self.success {
            writeln!(f, "Applying {} pending migration(s):", self.migration_count)?;
            writeln!(f)?;
            for (i, m) in self.migrations.iter().enumerate() {
                let status = if m.applied { "OK" } else { "SKIPPED" };
                // Pad description to align status
                let desc = if m.description.len() > 40 {
                    format!("{}...", &m.description[..37])
                } else {
                    m.description.clone()
                };
                writeln!(
                    f,
                    "  [{}/{}] {:.<50} {}",
                    i + 1,
                    self.migration_count,
                    desc,
                    status
                )?;
            }
            writeln!(f)?;
            writeln!(f, "All migrations applied successfully.")?;
        } else {
            writeln!(f, "Migration failed!")?;
            writeln!(f)?;
            if let Some(err) = &self.error {
                writeln!(f, "Error: {}", err)?;
            }
            writeln!(f)?;
            writeln!(
                f,
                "Applied {} of {} migration(s) before failure.",
                self.migrations.iter().filter(|m| m.applied).count(),
                self.migration_count
            )?;
        }

        Ok(())
    }
}

impl Up {
    /// Dispatch the up command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Resolve database URL (positional > --database-url > env)
        let db_url = self
            .url
            .or(self.database_url)
            .ok_or_else(|| miette::miette!("Database URL required. Provide as argument, --database-url, or set DATABASE_URL environment variable."))?;

        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Check that we have migrations to apply
        let index = backend.get_migration_index().await.into_diagnostic()?;
        if index.is_empty() {
            return Err(miette::miette!(
                "No migrations found in state backend.\n\nRun 'tern generate' to create a migration first."
            ));
        }

        if !matches!(self.format, OutputFormat::Json) {
            println!("Connecting to database...");
        }

        // Connect to the database
        let client = db::connect(&db_url)
            .await
            .into_diagnostic()
            .wrap_err("Failed to connect to database")?;

        // Create executor
        let executor = MigrationExecutor::new(&client, &backend, &self.schema);

        if self.dry_run {
            // Just show pending migrations
            if !matches!(self.format, OutputFormat::Json) {
                println!("Checking migration status...");
            }

            let pending = executor
                .get_pending(false)
                .await
                .into_diagnostic()
                .wrap_err("Failed to get pending migrations")?;

            let migrations: Vec<MigrationInfo> = pending
                .iter()
                .map(|m| MigrationInfo {
                    id: m.id.to_hex(),
                    description: m.description.clone(),
                    statement_count: None,
                    schema_hash: None,
                    applied: false,
                })
                .collect();

            let output = UpOutput {
                success: true,
                dry_run: true,
                migration_count: migrations.len(),
                migrations,
                error: None,
            };

            match self.format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => {
                    println!("-- Dry run: showing pending migrations");
                    for m in pending {
                        println!("-- Migration: {} - {}", m.id.to_short_hex(), m.description);
                    }
                }
            }
        } else {
            // Execute pending migrations
            if !matches!(self.format, OutputFormat::Json) {
                println!("Checking migration status...");
            }

            let result = executor
                .execute_pending(false)
                .await
                .into_diagnostic()
                .wrap_err("Failed to execute migrations")?;

            let migrations: Vec<MigrationInfo> = result
                .applied
                .iter()
                .map(|r| MigrationInfo {
                    id: r.migration_id.to_hex(),
                    description: r.description.clone(),
                    statement_count: Some(r.statement_count),
                    schema_hash: Some(r.schema_hash.clone()),
                    applied: true,
                })
                .collect();

            let output = UpOutput {
                success: result.success,
                dry_run: false,
                migration_count: migrations.len(),
                migrations,
                error: result.error.clone(),
            };

            match self.format {
                OutputFormat::Text => println!("{}", output),
                OutputFormat::Json => print_json(&output),
                OutputFormat::Sql => {
                    if result.success {
                        println!("-- All migrations applied successfully");
                    } else if let Some(err) = &result.error {
                        println!("-- Migration failed: {}", err);
                    }
                }
            }

            // Exit with error if migrations failed
            if !result.success {
                std::process::exit(1);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_output_no_migrations_display() {
        let output = UpOutput {
            success: true,
            dry_run: false,
            migration_count: 0,
            migrations: vec![],
            error: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Database is up to date"));
    }

    #[test]
    fn up_output_dry_run_display() {
        let output = UpOutput {
            success: true,
            dry_run: true,
            migration_count: 2,
            migrations: vec![
                MigrationInfo {
                    id: "abc123def456".to_string(),
                    description: "Add users table".to_string(),
                    statement_count: None,
                    schema_hash: None,
                    applied: false,
                },
                MigrationInfo {
                    id: "789xyz123456".to_string(),
                    description: "Add orders table".to_string(),
                    statement_count: None,
                    schema_hash: None,
                    applied: false,
                },
            ],
            error: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Pending migrations (dry run)"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("Add orders table"));
        assert!(display.contains("Run without --dry-run"));
    }

    #[test]
    fn up_output_success_display() {
        let output = UpOutput {
            success: true,
            dry_run: false,
            migration_count: 2,
            migrations: vec![
                MigrationInfo {
                    id: "abc123def456".to_string(),
                    description: "Add users table".to_string(),
                    statement_count: Some(3),
                    schema_hash: Some("hash1".to_string()),
                    applied: true,
                },
                MigrationInfo {
                    id: "789xyz123456".to_string(),
                    description: "Add orders table".to_string(),
                    statement_count: Some(2),
                    schema_hash: Some("hash2".to_string()),
                    applied: true,
                },
            ],
            error: None,
        };
        let display = format!("{}", output);
        assert!(display.contains("Applying 2 pending migration(s)"));
        assert!(display.contains("All migrations applied successfully"));
    }

    #[test]
    fn up_output_failure_display() {
        let output = UpOutput {
            success: false,
            dry_run: false,
            migration_count: 2,
            migrations: vec![MigrationInfo {
                id: "abc123def456".to_string(),
                description: "Add users table".to_string(),
                statement_count: Some(3),
                schema_hash: Some("hash1".to_string()),
                applied: true,
            }],
            error: Some("relation 'foo' does not exist".to_string()),
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration failed"));
        assert!(display.contains("relation 'foo' does not exist"));
    }

    #[test]
    fn up_output_serializes() {
        let output = UpOutput {
            success: true,
            dry_run: false,
            migration_count: 1,
            migrations: vec![MigrationInfo {
                id: "abc".to_string(),
                description: "Test".to_string(),
                statement_count: Some(1),
                schema_hash: Some("hash".to_string()),
                applied: true,
            }],
            error: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"success\":true"));
        assert!(json.contains("\"migration_count\":1"));
    }
}
