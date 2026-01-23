//! Verify command for checking state backend consistency.
//!
//! This command verifies that the state backend matches the current
//! database schema, detecting any drift from manual changes.

use miette::{Context, IntoDiagnostic};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::checksum::compute_schema_checksum;
use crate::db::diff::{NamespaceDiff, diff_namespaces};
use crate::db::model::Namespace;
use crate::db::query::PostgresCatalog;
use crate::db::schema::{ColumnName, ConstraintName, IndexName, SequenceName, TableName, TypeName};
use crate::db::state::{StateBackend, StateHash, verify_state};
use crate::db::{self};

/// Verify output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutput {
    /// Whether the verification passed.
    pub verified: bool,
    /// State backend state hash (BLAKE3).
    pub backend_state_hash: String,
    /// Database state hash (BLAKE3).
    pub database_state_hash: String,
    /// Schema checksum (xxhash3) for the backend state.
    pub backend_schema_checksum: String,
    /// Schema checksum (xxhash3) for the database state.
    pub database_schema_checksum: String,
    /// Detailed message.
    pub message: String,
    /// Drift details (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift_details: Option<DriftDetails>,
}

/// Details about schema drift.
#[derive(Debug, Clone, Serialize)]
pub struct DriftDetails {
    /// Summary of changes.
    pub summary: DriftSummary,
    /// Tables in database but not in state.
    pub added_tables: Vec<String>,
    /// Tables in state but not in database.
    pub removed_tables: Vec<String>,
    /// Tables with modifications.
    pub modified_tables: Vec<ModifiedTableSummary>,
    /// Potential table renames detected.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub potential_renames: Vec<PotentialRenameSummary>,
    /// Views in database but not in state.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_views: Vec<String>,
    /// Views in state but not in database.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_views: Vec<String>,
    /// Views with modifications.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_views: Vec<String>,
    /// Sequences in database but not in state.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_sequences: Vec<String>,
    /// Sequences in state but not in database.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_sequences: Vec<String>,
    /// Sequences with modifications.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_sequences: Vec<String>,
    /// Enums in database but not in state.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_enums: Vec<String>,
    /// Enums in state but not in database.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_enums: Vec<String>,
    /// Enums with modifications.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_enums: Vec<String>,
}

/// Summary of drift changes.
#[derive(Debug, Clone, Serialize)]
pub struct DriftSummary {
    /// Total number of tables affected.
    pub tables_changed: usize,
    /// Total number of views affected.
    pub views_changed: usize,
    /// Total number of sequences affected.
    pub sequences_changed: usize,
    /// Total number of enums affected.
    pub enums_changed: usize,
    /// Total number of all changes.
    pub total_changes: usize,
}

/// Summary of a modified table.
#[derive(Debug, Clone, Serialize)]
pub struct ModifiedTableSummary {
    /// The table name.
    pub name: String,
    /// Columns added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_columns: Vec<String>,
    /// Columns removed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_columns: Vec<String>,
    /// Columns modified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_columns: Vec<String>,
    /// Constraints added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_constraints: Vec<String>,
    /// Constraints removed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_constraints: Vec<String>,
    /// Constraints modified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_constraints: Vec<String>,
    /// Indexes added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub added_indexes: Vec<String>,
    /// Indexes removed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed_indexes: Vec<String>,
    /// Indexes modified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modified_indexes: Vec<String>,
}

/// Summary of a potential rename.
#[derive(Debug, Clone, Serialize)]
pub struct PotentialRenameSummary {
    /// The old name (in state).
    pub from: String,
    /// The new name (in database).
    pub to: String,
    /// Similarity score (0.0 - 1.0).
    pub similarity: f64,
}

impl std::fmt::Display for VerifyOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.verified {
            writeln!(f, "Verification Passed")?;
            writeln!(f, "===================")?;
            writeln!(f)?;
            writeln!(f, "State backend matches database schema.")?;
            writeln!(f)?;
            writeln!(f, "  State hash:     {}", &self.backend_state_hash[..16])?;
            writeln!(f, "  Schema checksum: {}", &self.backend_schema_checksum)?;
        } else {
            writeln!(f, "Verification Failed")?;
            writeln!(f, "===================")?;
            writeln!(f)?;
            writeln!(f, "WARNING: Schema drift detected!")?;
            writeln!(f)?;
            writeln!(f, "State Hashes (BLAKE3):")?;
            writeln!(
                f,
                "  Backend:  {}",
                &self.backend_state_hash[..std::cmp::min(16, self.backend_state_hash.len())]
            )?;
            writeln!(
                f,
                "  Database: {}",
                &self.database_state_hash[..std::cmp::min(16, self.database_state_hash.len())]
            )?;
            writeln!(f)?;
            writeln!(f, "Schema Checksums (xxhash3):")?;
            writeln!(f, "  Backend:  {}", &self.backend_schema_checksum)?;
            writeln!(f, "  Database: {}", &self.database_schema_checksum)?;
            writeln!(f)?;
            writeln!(f, "{}", self.message)?;

            if let Some(ref drift) = self.drift_details {
                // Summary
                writeln!(f)?;
                writeln!(f, "Summary: {} total changes", drift.summary.total_changes)?;
                if drift.summary.tables_changed > 0 {
                    writeln!(f, "  - {} table(s) affected", drift.summary.tables_changed)?;
                }
                if drift.summary.views_changed > 0 {
                    writeln!(f, "  - {} view(s) affected", drift.summary.views_changed)?;
                }
                if drift.summary.sequences_changed > 0 {
                    writeln!(
                        f,
                        "  - {} sequence(s) affected",
                        drift.summary.sequences_changed
                    )?;
                }
                if drift.summary.enums_changed > 0 {
                    writeln!(f, "  - {} enum(s) affected", drift.summary.enums_changed)?;
                }

                // Table changes
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
                        write!(f, "  ~ {}", table.name)?;
                        let changes = count_table_changes(table);
                        if changes > 0 {
                            write!(f, " ({} changes)", changes)?;
                        }
                        writeln!(f)?;

                        // Show column changes
                        for col in &table.added_columns {
                            writeln!(f, "      + column: {}", col)?;
                        }
                        for col in &table.removed_columns {
                            writeln!(f, "      - column: {}", col)?;
                        }
                        for col in &table.modified_columns {
                            writeln!(f, "      ~ column: {}", col)?;
                        }

                        // Show constraint changes
                        for con in &table.added_constraints {
                            writeln!(f, "      + constraint: {}", con)?;
                        }
                        for con in &table.removed_constraints {
                            writeln!(f, "      - constraint: {}", con)?;
                        }
                        for con in &table.modified_constraints {
                            writeln!(f, "      ~ constraint: {}", con)?;
                        }

                        // Show index changes
                        for idx in &table.added_indexes {
                            writeln!(f, "      + index: {}", idx)?;
                        }
                        for idx in &table.removed_indexes {
                            writeln!(f, "      - index: {}", idx)?;
                        }
                        for idx in &table.modified_indexes {
                            writeln!(f, "      ~ index: {}", idx)?;
                        }
                    }
                }
                if !drift.potential_renames.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Potential table renames detected:")?;
                    for rename in &drift.potential_renames {
                        writeln!(
                            f,
                            "  ? {} -> {} (similarity: {:.0}%)",
                            rename.from,
                            rename.to,
                            rename.similarity * 100.0
                        )?;
                    }
                }

                // View changes
                if !drift.added_views.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Views added to database:")?;
                    for view in &drift.added_views {
                        writeln!(f, "  + {}", view)?;
                    }
                }
                if !drift.removed_views.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Views removed from database:")?;
                    for view in &drift.removed_views {
                        writeln!(f, "  - {}", view)?;
                    }
                }
                if !drift.modified_views.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Views modified:")?;
                    for view in &drift.modified_views {
                        writeln!(f, "  ~ {}", view)?;
                    }
                }

                // Sequence changes
                if !drift.added_sequences.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Sequences added to database:")?;
                    for seq in &drift.added_sequences {
                        writeln!(f, "  + {}", seq)?;
                    }
                }
                if !drift.removed_sequences.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Sequences removed from database:")?;
                    for seq in &drift.removed_sequences {
                        writeln!(f, "  - {}", seq)?;
                    }
                }
                if !drift.modified_sequences.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Sequences modified:")?;
                    for seq in &drift.modified_sequences {
                        writeln!(f, "  ~ {}", seq)?;
                    }
                }

                // Enum changes
                if !drift.added_enums.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Enums added to database:")?;
                    for e in &drift.added_enums {
                        writeln!(f, "  + {}", e)?;
                    }
                }
                if !drift.removed_enums.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Enums removed from database:")?;
                    for e in &drift.removed_enums {
                        writeln!(f, "  - {}", e)?;
                    }
                }
                if !drift.modified_enums.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "Enums modified:")?;
                    for e in &drift.modified_enums {
                        writeln!(f, "  ~ {}", e)?;
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

/// Count the total number of changes in a modified table.
fn count_table_changes(table: &ModifiedTableSummary) -> usize {
    table.added_columns.len()
        + table.removed_columns.len()
        + table.modified_columns.len()
        + table.added_constraints.len()
        + table.removed_constraints.len()
        + table.modified_constraints.len()
        + table.added_indexes.len()
        + table.removed_indexes.len()
        + table.modified_indexes.len()
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

    // Get state and compute hashes
    let backend_state = backend.get_current_state().await.into_diagnostic()?;
    let database_state = crate::db::query::load_namespace(&catalog, schema)
        .await
        .into_diagnostic()?;

    let backend_hash = StateHash::from_namespace(&backend_state);
    let database_hash = StateHash::from_namespace(&database_state);

    // Compute xxhash3 checksums
    let backend_checksum = compute_schema_checksum(&backend_state);
    let database_checksum = compute_schema_checksum(&database_state);

    // Compute drift details if not matching
    let drift_details = if !matches {
        Some(compute_drift(&backend_state, &database_state))
    } else {
        None
    };

    let output = VerifyOutput {
        verified: matches,
        backend_state_hash: backend_hash.to_hex(),
        database_state_hash: database_hash.to_hex(),
        backend_schema_checksum: backend_checksum,
        database_schema_checksum: database_checksum,
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
///
/// Uses the comprehensive diff_namespaces function for accurate change detection.
fn compute_drift(backend: &Namespace, database: &Namespace) -> DriftDetails {
    // Use the full diff system for comprehensive comparison
    let diff = diff_namespaces(backend, database);

    // Build detailed drift information from the diff
    build_drift_details(&diff)
}

/// Build DriftDetails from a NamespaceDiff.
fn build_drift_details(diff: &NamespaceDiff) -> DriftDetails {
    // Tables
    let added_tables: Vec<String> = diff
        .tables
        .added
        .iter()
        .map(|t| t.name.as_ref().to_string())
        .collect();
    let removed_tables: Vec<String> = diff
        .tables
        .removed
        .iter()
        .map(|t: &TableName| t.as_ref().to_string())
        .collect();
    let modified_tables: Vec<ModifiedTableSummary> = diff
        .tables
        .modified
        .iter()
        .map(|m| ModifiedTableSummary {
            name: m.name.as_ref().to_string(),
            added_columns: m
                .columns
                .added
                .iter()
                .map(|c| c.name.as_ref().to_string())
                .collect(),
            removed_columns: m
                .columns
                .removed
                .iter()
                .map(|c: &ColumnName| c.as_ref().to_string())
                .collect(),
            modified_columns: m
                .columns
                .modified
                .iter()
                .map(|c| c.name.as_ref().to_string())
                .collect(),
            added_constraints: m
                .constraints
                .added
                .iter()
                .map(|c| c.name.as_ref().to_string())
                .collect(),
            removed_constraints: m
                .constraints
                .removed
                .iter()
                .map(|c: &ConstraintName| c.as_ref().to_string())
                .collect(),
            modified_constraints: m
                .constraints
                .modified
                .iter()
                .map(|c| c.name.as_ref().to_string())
                .collect(),
            added_indexes: m
                .indexes
                .added
                .iter()
                .map(|i| i.name.as_ref().to_string())
                .collect(),
            removed_indexes: m
                .indexes
                .removed
                .iter()
                .map(|i: &IndexName| i.as_ref().to_string())
                .collect(),
            modified_indexes: m
                .indexes
                .modified
                .iter()
                .map(|i| i.name.as_ref().to_string())
                .collect(),
        })
        .collect();
    let potential_renames: Vec<PotentialRenameSummary> = diff
        .tables
        .potential_renames
        .iter()
        .map(|r| PotentialRenameSummary {
            from: r.source_key.as_ref().to_string(),
            to: r.target.name.as_ref().to_string(),
            similarity: r.similarity,
        })
        .collect();

    // Views
    let added_views: Vec<String> = diff
        .views
        .added
        .iter()
        .map(|v| v.name.as_ref().to_string())
        .collect();
    let removed_views: Vec<String> = diff
        .views
        .removed
        .iter()
        .map(|v: &TableName| v.as_ref().to_string())
        .collect();
    let modified_views: Vec<String> = diff
        .views
        .modified
        .iter()
        .map(|v| v.name.as_ref().to_string())
        .collect();

    // Sequences
    let added_sequences: Vec<String> = diff
        .sequences
        .added
        .iter()
        .map(|s| s.name.as_ref().to_string())
        .collect();
    let removed_sequences: Vec<String> = diff
        .sequences
        .removed
        .iter()
        .map(|s: &SequenceName| s.as_ref().to_string())
        .collect();
    let modified_sequences: Vec<String> = diff
        .sequences
        .modified
        .iter()
        .map(|s| s.name.as_ref().to_string())
        .collect();

    // Enums
    let added_enums: Vec<String> = diff
        .enums
        .added
        .iter()
        .map(|e| e.name.as_ref().to_string())
        .collect();
    let removed_enums: Vec<String> = diff
        .enums
        .removed
        .iter()
        .map(|e: &TypeName| e.as_ref().to_string())
        .collect();
    let modified_enums: Vec<String> = diff
        .enums
        .modified
        .iter()
        .map(|e| e.name.as_ref().to_string())
        .collect();

    // Compute summary
    let tables_changed =
        added_tables.len() + removed_tables.len() + modified_tables.len() + potential_renames.len();
    let views_changed = added_views.len() + removed_views.len() + modified_views.len();
    let sequences_changed =
        added_sequences.len() + removed_sequences.len() + modified_sequences.len();
    let enums_changed = added_enums.len() + removed_enums.len() + modified_enums.len();
    let total_changes = tables_changed + views_changed + sequences_changed + enums_changed;

    DriftDetails {
        summary: DriftSummary {
            tables_changed,
            views_changed,
            sequences_changed,
            enums_changed,
            total_changes,
        },
        added_tables,
        removed_tables,
        modified_tables,
        potential_renames,
        added_views,
        removed_views,
        modified_views,
        added_sequences,
        removed_sequences,
        modified_sequences,
        added_enums,
        removed_enums,
        modified_enums,
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
    use crate::db::state::{LocalFileBackend, init_empty};
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
            backend_schema_checksum: "abc123def456".to_string(),
            database_schema_checksum: "abc123def456".to_string(),
            message: "All good".to_string(),
            drift_details: None,
        };

        let display = format!("{}", output);
        assert!(display.contains("Verification Passed"));
        assert!(display.contains("abc123def456"));
    }

    #[test]
    fn verify_output_display_failure() {
        let output = VerifyOutput {
            verified: false,
            backend_state_hash: "a".repeat(64),
            database_state_hash: "b".repeat(64),
            backend_schema_checksum: "abc123def456".to_string(),
            database_schema_checksum: "def456abc123".to_string(),
            message: "Drift detected".to_string(),
            drift_details: Some(DriftDetails {
                summary: DriftSummary {
                    tables_changed: 3,
                    views_changed: 0,
                    sequences_changed: 0,
                    enums_changed: 0,
                    total_changes: 3,
                },
                added_tables: vec!["new_table".to_string()],
                removed_tables: vec!["old_table".to_string()],
                modified_tables: vec![ModifiedTableSummary {
                    name: "changed_table".to_string(),
                    added_columns: vec!["new_col".to_string()],
                    removed_columns: vec![],
                    modified_columns: vec![],
                    added_constraints: vec![],
                    removed_constraints: vec![],
                    modified_constraints: vec![],
                    added_indexes: vec![],
                    removed_indexes: vec![],
                    modified_indexes: vec![],
                }],
                potential_renames: vec![],
                added_views: vec![],
                removed_views: vec![],
                modified_views: vec![],
                added_sequences: vec![],
                removed_sequences: vec![],
                modified_sequences: vec![],
                added_enums: vec![],
                removed_enums: vec![],
                modified_enums: vec![],
            }),
        };

        let display = format!("{}", output);
        assert!(display.contains("Verification Failed"));
        assert!(display.contains("new_table"));
        assert!(display.contains("old_table"));
        assert!(display.contains("changed_table"));
        assert!(display.contains("new_col"));
        assert!(display.contains("3 total changes"));
    }

    #[test]
    fn compute_drift_empty_states() {
        let backend = Namespace::empty("public");
        let database = Namespace::empty("public");

        let drift = compute_drift(&backend, &database);
        assert!(drift.added_tables.is_empty());
        assert!(drift.removed_tables.is_empty());
        assert!(drift.modified_tables.is_empty());
        assert_eq!(drift.summary.total_changes, 0);
    }

    #[test]
    fn compute_drift_with_added_table() {
        use crate::db::model::table::{Table, TableKind};
        use crate::db::schema::{Oid, TableName};

        let backend = Namespace::empty("public");
        let mut database = Namespace::empty("public");
        database.tables.push(Table {
            oid: Oid::new(1),
            name: TableName::try_new("users".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        });

        let drift = compute_drift(&backend, &database);
        assert_eq!(drift.added_tables, vec!["users"]);
        assert!(drift.removed_tables.is_empty());
        assert!(drift.modified_tables.is_empty());
        assert_eq!(drift.summary.tables_changed, 1);
        assert_eq!(drift.summary.total_changes, 1);
    }

    #[test]
    fn verify_output_json_serialization() {
        let output = VerifyOutput {
            verified: true,
            backend_state_hash: "a".repeat(64),
            database_state_hash: "a".repeat(64),
            backend_schema_checksum: "abc123def456".to_string(),
            database_schema_checksum: "abc123def456".to_string(),
            message: "All good".to_string(),
            drift_details: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("backend_schema_checksum"));
        assert!(json.contains("database_schema_checksum"));
        assert!(json.contains("abc123def456"));
    }
}
