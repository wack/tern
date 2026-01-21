//! Verify command for checking state backend consistency.
//!
//! This command verifies that the state backend matches the current
//! database schema, detecting any drift from manual changes.

use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json, LocalFileBackend};
use crate::db::query::PostgresCatalog;
use crate::db::state::{StateBackend, StateHash, verify_state};
use crate::db::{self};

/// Verify output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutput {
    /// Whether the verification passed.
    pub verified: bool,
    /// State backend state hash.
    pub backend_state_hash: String,
    /// Database state hash.
    pub database_state_hash: String,
    /// Detailed message.
    pub message: String,
    /// Drift details (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift_details: Option<DriftDetails>,
}

/// Details about schema drift.
#[derive(Debug, Clone, Serialize)]
pub struct DriftDetails {
    /// Tables in database but not in state.
    pub added_tables: Vec<String>,
    /// Tables in state but not in database.
    pub removed_tables: Vec<String>,
    /// Tables with modifications.
    pub modified_tables: Vec<String>,
}

impl std::fmt::Display for VerifyOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.verified {
            writeln!(f, "Verification Passed")?;
            writeln!(f, "===================")?;
            writeln!(f)?;
            writeln!(f, "State backend matches database schema.")?;
            writeln!(f)?;
            writeln!(f, "  State hash: {}", &self.backend_state_hash[..16])?;
        } else {
            writeln!(f, "Verification Failed")?;
            writeln!(f, "===================")?;
            writeln!(f)?;
            writeln!(f, "WARNING: Schema drift detected!")?;
            writeln!(f)?;
            writeln!(
                f,
                "  Backend state: {}",
                &self.backend_state_hash[..std::cmp::min(16, self.backend_state_hash.len())]
            )?;
            writeln!(
                f,
                "  Database state: {}",
                &self.database_state_hash[..std::cmp::min(16, self.database_state_hash.len())]
            )?;
            writeln!(f)?;
            writeln!(f, "{}", self.message)?;

            if let Some(ref drift) = self.drift_details {
                if !drift.added_tables.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Tables added to database:")?;
                    for table in &drift.added_tables {
                        writeln!(f, "  + {}", table)?;
                    }
                }
                if !drift.removed_tables.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Tables removed from database:")?;
                    for table in &drift.removed_tables {
                        writeln!(f, "  - {}", table)?;
                    }
                }
                if !drift.modified_tables.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Tables modified:")?;
                    for table in &drift.modified_tables {
                        writeln!(f, "  ~ {}", table)?;
                    }
                }
            }

            writeln!(f)?;
            writeln!(f, "To resolve, either:")?;
            writeln!(
                f,
                "  1. Run 'tern compile' to capture the changes as a new migration"
            )?;
            writeln!(f, "  2. Revert the database to match the state backend")?;
        }

        Ok(())
    }
}

/// Runs the verify command.
///
/// Verifies that the state backend matches the current database schema.
///
/// # Arguments
///
/// * `database_url` - PostgreSQL connection string
/// * `schema` - Database schema name
/// * `format` - Output format
/// * `state_path` - Optional path to the state directory
pub async fn run_verify(
    database_url: &str,
    schema: &str,
    format: OutputFormat,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Connect to database
    println!("Connecting to database...");
    let client = db::connect(database_url)
        .await
        .into_diagnostic()
        .wrap_err("Failed to connect to database")?;

    let catalog = PostgresCatalog::new(&client);

    println!("Verifying schema '{}'...", schema);

    // Verify state
    let matches = verify_state(&backend, &catalog, schema)
        .await
        .into_diagnostic()?;

    // Get hashes for output
    let backend_hash = backend.get_current_state_hash().await.into_diagnostic()?;
    let database_state = crate::db::query::load_namespace(&catalog, schema)
        .await
        .into_diagnostic()?;
    let database_hash = StateHash::from_namespace(&database_state);

    // Compute drift details if not matching
    let drift_details = if !matches {
        let backend_state = backend.get_current_state().await.into_diagnostic()?;
        Some(compute_drift(&backend_state, &database_state))
    } else {
        None
    };

    let output = VerifyOutput {
        verified: matches,
        backend_state_hash: backend_hash.to_hex(),
        database_state_hash: database_hash.to_hex(),
        message: if matches {
            "State backend is in sync with database.".to_string()
        } else {
            "Database has been modified outside of Tern migrations.".to_string()
        },
        drift_details,
    };

    match format {
        OutputFormat::Text | OutputFormat::Sql => println!("{}", output),
        OutputFormat::Json => print_json(&output),
    }

    // Return error if verification failed (for CI usage)
    if !matches {
        std::process::exit(1);
    }

    Ok(())
}

/// Compute drift details between backend state and database state.
fn compute_drift(
    backend: &crate::db::model::Namespace,
    database: &crate::db::model::Namespace,
) -> DriftDetails {
    use std::collections::HashSet;

    let backend_tables: HashSet<_> = backend.tables.iter().map(|t| t.name.to_string()).collect();
    let database_tables: HashSet<_> = database.tables.iter().map(|t| t.name.to_string()).collect();

    let added_tables: Vec<_> = database_tables
        .difference(&backend_tables)
        .cloned()
        .collect();
    let removed_tables: Vec<_> = backend_tables
        .difference(&database_tables)
        .cloned()
        .collect();

    // Check for modifications (tables in both but with different structure)
    let common_tables: HashSet<_> = backend_tables.intersection(&database_tables).collect();
    let mut modified_tables = Vec::new();

    for table_name in common_tables {
        let backend_table = backend
            .tables
            .iter()
            .find(|t| &t.name.to_string() == table_name);
        let database_table = database
            .tables
            .iter()
            .find(|t| &t.name.to_string() == table_name);

        if let (Some(bt), Some(dt)) = (backend_table, database_table) {
            // Compare column counts as a simple heuristic
            if bt.columns.len() != dt.columns.len()
                || bt.constraints.len() != dt.constraints.len()
                || bt.indexes.len() != dt.indexes.len()
            {
                modified_tables.push(table_name.clone());
            }
        }
    }

    DriftDetails {
        added_tables,
        removed_tables,
        modified_tables,
    }
}

/// Runs the verify chain command.
///
/// Verifies the integrity of the migration chain in the state backend.
///
/// # Arguments
///
/// * `format` - Output format
/// * `state_path` - Optional path to the state directory
pub async fn run_verify_chain(
    format: OutputFormat,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    println!("Verifying migration chain...");

    match backend.verify_chain().await {
        Ok(()) => {
            let index = backend.get_migration_index().await.into_diagnostic()?;
            let output = ChainVerifyOutput {
                verified: true,
                migration_count: index.len(),
                message: format!(
                    "Migration chain verified: {} migrations with valid chain integrity.",
                    index.len()
                ),
            };

            match format {
                OutputFormat::Text | OutputFormat::Sql => println!("{}", output),
                OutputFormat::Json => print_json(&output),
            }
            Ok(())
        }
        Err(e) => {
            let output = ChainVerifyOutput {
                verified: false,
                migration_count: 0,
                message: format!("Chain verification failed: {}", e),
            };

            match format {
                OutputFormat::Text | OutputFormat::Sql => println!("{}", output),
                OutputFormat::Json => print_json(&output),
            }

            std::process::exit(1);
        }
    }
}

/// Chain verification output.
#[derive(Debug, Clone, Serialize)]
pub struct ChainVerifyOutput {
    /// Whether the verification passed.
    pub verified: bool,
    /// Number of migrations in chain.
    pub migration_count: usize,
    /// Detailed message.
    pub message: String,
}

impl std::fmt::Display for ChainVerifyOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.verified {
            writeln!(f, "Chain Verification Passed")?;
            writeln!(f, "=========================")?;
            writeln!(f)?;
            writeln!(f, "{}", self.message)?;
        } else {
            writeln!(f, "Chain Verification Failed")?;
            writeln!(f, "=========================")?;
            writeln!(f)?;
            writeln!(f, "{}", self.message)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::Namespace;
    use crate::db::state::init_empty;
    use tempfile::TempDir;

    #[tokio::test]
    async fn verify_chain_empty() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());
        init_empty(&backend, "public").await.unwrap();

        // Chain verification should pass
        run_verify_chain(OutputFormat::Text, Some(temp_dir.path()))
            .await
            .unwrap();
    }

    #[test]
    fn verify_output_display_success() {
        let output = VerifyOutput {
            verified: true,
            backend_state_hash: "a".repeat(64),
            database_state_hash: "a".repeat(64),
            message: "All good".to_string(),
            drift_details: None,
        };

        let display = format!("{}", output);
        assert!(display.contains("Verification Passed"));
    }

    #[test]
    fn verify_output_display_failure() {
        let output = VerifyOutput {
            verified: false,
            backend_state_hash: "a".repeat(64),
            database_state_hash: "b".repeat(64),
            message: "Drift detected".to_string(),
            drift_details: Some(DriftDetails {
                added_tables: vec!["new_table".to_string()],
                removed_tables: vec!["old_table".to_string()],
                modified_tables: vec!["changed_table".to_string()],
            }),
        };

        let display = format!("{}", output);
        assert!(display.contains("Verification Failed"));
        assert!(display.contains("new_table"));
        assert!(display.contains("old_table"));
        assert!(display.contains("changed_table"));
    }

    #[test]
    fn compute_drift_empty_states() {
        let backend = Namespace::empty("public");
        let database = Namespace::empty("public");

        let drift = compute_drift(&backend, &database);
        assert!(drift.added_tables.is_empty());
        assert!(drift.removed_tables.is_empty());
        assert!(drift.modified_tables.is_empty());
    }
}
