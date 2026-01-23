//! State backend initialization from an existing database.
//!
//! This module provides functions for initializing a state backend from
//! an existing database, creating the baseline migration that captures
//! the current schema state.

// Allow false positives from thiserror derive macro - these fields ARE used
// in the generated Display implementations via the #[error("...")] attribute.
#![allow(unused_assignments)]

use crate::db::diff::diff_namespaces;
use crate::db::migrate::MigrationPlan;
use crate::db::model::Namespace;
use crate::db::query::{Catalog, QueryError, load_namespace};

use super::StateBackend;
use super::error::StateError;
use super::types::{Migration, StateHash};

/// Error type for initialization operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum InitError {
    /// Failed to load schema from database.
    #[error("failed to load schema '{schema}': {source}")]
    #[diagnostic(code(tern::init::load_schema))]
    LoadSchema {
        schema: String,
        #[source]
        source: QueryError,
    },

    /// State backend error.
    #[error(transparent)]
    #[diagnostic(transparent)]
    State(#[from] StateError),

    /// State backend is already initialized.
    #[error("state backend is already initialized at {path}")]
    #[diagnostic(
        code(tern::init::already_initialized),
        help(
            "Use 'tern status' to view the current state or remove the directory to reinitialize"
        )
    )]
    AlreadyInitialized { path: String },
}

/// Initialize a state backend from an existing database schema.
///
/// This function:
/// 1. Introspects the current database schema
/// 2. Initializes the state backend (creates directories, etc.)
/// 3. Creates a baseline migration with full DDL operations
/// 4. Saves the baseline and current state to the backend
///
/// The baseline migration includes operations that would recreate the
/// schema from scratch. This allows existing database schemas to be imported
/// into Tern's migration history.
///
/// # Arguments
///
/// * `backend` - The state backend to initialize
/// * `catalog` - Database catalog adapter for introspection
/// * `schema_name` - Name of the database schema to capture
///
/// # Returns
///
/// The baseline migration that was created.
///
/// # Errors
///
/// Returns an error if:
/// - The backend is already initialized
/// - Database introspection fails
/// - Saving to the backend fails
///
/// # Example
///
/// ```ignore
/// use tern::db::state::{LocalFileBackend, init_from_database};
/// use tern::db::query::PostgresCatalog;
///
/// let backend = LocalFileBackend::default_location();
/// let catalog = PostgresCatalog::new(&client);
///
/// let baseline = init_from_database(&backend, &catalog, "public").await?;
/// println!("Initialized with baseline migration: {}", baseline.id.to_short_hex());
/// ```
pub async fn init_from_database<B, C>(
    backend: &B,
    catalog: &C,
    schema_name: &str,
) -> Result<Migration, InitError>
where
    B: StateBackend,
    C: Catalog,
{
    // Check if already initialized
    if backend.is_initialized().await? {
        return Err(InitError::AlreadyInitialized {
            path: "state backend".to_string(),
        });
    }

    // Load current database schema
    let namespace = load_namespace(catalog, schema_name)
        .await
        .map_err(|source| InitError::LoadSchema {
            schema: schema_name.to_string(),
            source,
        })?;

    // Initialize the backend
    backend.initialize().await?;

    // Generate operations by diffing empty -> current state
    let empty = Namespace::empty(schema_name);
    let diff = diff_namespaces(&empty, &namespace);
    let plan = MigrationPlan::from_diff(&diff);

    // Create and save baseline migration with operations
    let baseline = Migration::baseline_with_operations(namespace.clone(), plan.operations);
    backend.save_migration(&baseline).await?;

    // Save current state
    backend.save_current_state(&namespace).await?;

    Ok(baseline)
}

/// Initialize a state backend with an empty schema.
///
/// This creates a baseline migration for an empty schema, useful when
/// starting a new project without an existing database.
///
/// # Arguments
///
/// * `backend` - The state backend to initialize
/// * `schema_name` - Name of the schema (typically "public")
///
/// # Returns
///
/// The baseline migration that was created.
///
/// # Errors
///
/// Returns an error if the backend is already initialized or saving fails.
pub async fn init_empty<B>(backend: &B, schema_name: &str) -> Result<Migration, InitError>
where
    B: StateBackend,
{
    // Check if already initialized
    if backend.is_initialized().await? {
        return Err(InitError::AlreadyInitialized {
            path: "state backend".to_string(),
        });
    }

    // Create empty namespace
    let namespace = Namespace::empty(schema_name);

    // Initialize the backend
    backend.initialize().await?;

    // Create and save baseline migration
    let baseline = Migration::baseline(namespace.clone());
    backend.save_migration(&baseline).await?;

    // Save current state
    backend.save_current_state(&namespace).await?;

    Ok(baseline)
}

/// Verify that the state backend matches the current database schema.
///
/// This compares the stored state hash with a fresh introspection of
/// the database to detect any drift (manual changes not tracked by migrations).
///
/// # Arguments
///
/// * `backend` - The state backend to verify
/// * `catalog` - Database catalog adapter for introspection
/// * `schema_name` - Name of the database schema to verify
///
/// # Returns
///
/// `Ok(true)` if the state matches, `Ok(false)` if there is drift.
///
/// # Errors
///
/// Returns an error if the backend is not initialized or database
/// introspection fails.
pub async fn verify_state<B, C>(
    backend: &B,
    catalog: &C,
    schema_name: &str,
) -> Result<bool, InitError>
where
    B: StateBackend,
    C: Catalog,
{
    // Get stored state hash
    let stored_hash = backend.get_current_state_hash().await?;

    // Load current database schema
    let namespace = load_namespace(catalog, schema_name)
        .await
        .map_err(|source| InitError::LoadSchema {
            schema: schema_name.to_string(),
            source,
        })?;

    // Compute current hash
    let current_hash = StateHash::from_namespace(&namespace);

    Ok(stored_hash == current_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::query::FakeCatalog;
    use crate::db::query::catalog::{ColumnRow, NamespaceRow, TableRow};
    use crate::db::state::InMemoryBackend;

    /// Helper function to create a basic column row.
    fn make_column(position: i16, name: &str, type_name: &str) -> ColumnRow {
        ColumnRow {
            position,
            name: name.to_string(),
            type_name: type_name.to_string(),
            type_schema: "pg_catalog".to_string(),
            formatted_type: type_name.to_string(),
            is_array: false,
            is_nullable: true,
            default_expr: None,
            generated_kind: None,
            identity_kind: None,
            collation_schema: "pg_catalog".to_string(),
            collation_name: "default".to_string(),
            comment: None,
        }
    }

    #[tokio::test]
    async fn init_empty_creates_baseline() {
        let backend = InMemoryBackend::new();

        let baseline = init_empty(&backend, "public").await.unwrap();

        assert!(baseline.is_baseline());
        assert!(baseline.is_checkpoint());
        assert!(baseline.parent_state_hash.is_zero());
        assert_eq!(baseline.operations.len(), 0);

        // Verify backend state
        assert!(backend.is_initialized().await.unwrap());
        let index = backend.get_migration_index().await.unwrap();
        assert_eq!(index.len(), 1);

        // Verify current state was saved
        let state = backend.get_current_state().await.unwrap();
        assert_eq!(state.name.as_ref(), "public");
    }

    #[tokio::test]
    async fn init_empty_fails_if_already_initialized() {
        let backend = InMemoryBackend::new();

        // First init should succeed
        init_empty(&backend, "public").await.unwrap();

        // Second init should fail
        let result = init_empty(&backend, "public").await;
        assert!(matches!(result, Err(InitError::AlreadyInitialized { .. })));
    }

    #[tokio::test]
    async fn init_from_database_creates_baseline_with_schema() {
        let backend = InMemoryBackend::new();

        // Create a fake catalog with some tables
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(200, make_column(1, "id", "integer"))
            .with_column(200, make_column(2, "email", "text"));

        let baseline = init_from_database(&backend, &catalog, "public")
            .await
            .unwrap();

        // Baseline still has parent_state_hash of zero
        assert!(baseline.parent_state_hash.is_zero());
        assert!(baseline.is_checkpoint());

        // NEW: Baseline now has operations to create the schema from scratch
        assert!(!baseline.operations.is_empty());
        // Should have at least one operation to create the table
        assert!(baseline.operation_count() >= 1);

        // Verify the checkpoint state contains the table
        let checkpoint_state = baseline.checkpoint_state.as_ref().unwrap();
        assert_eq!(checkpoint_state.tables.len(), 1);
        assert_eq!(checkpoint_state.tables[0].name.as_ref(), "users");
        assert_eq!(checkpoint_state.tables[0].columns.len(), 2);

        // Verify current state was saved
        let state = backend.get_current_state().await.unwrap();
        assert_eq!(state.tables.len(), 1);
    }

    #[tokio::test]
    async fn verify_state_detects_match() {
        let backend = InMemoryBackend::new();

        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(200, make_column(1, "id", "integer"));

        init_from_database(&backend, &catalog, "public")
            .await
            .unwrap();

        // Same catalog should verify as matching
        let matches = verify_state(&backend, &catalog, "public").await.unwrap();
        assert!(matches);
    }

    #[tokio::test]
    async fn verify_state_detects_drift() {
        let backend = InMemoryBackend::new();

        let catalog1 = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(200, make_column(1, "id", "integer"));

        init_from_database(&backend, &catalog1, "public")
            .await
            .unwrap();

        // Different catalog should detect drift (different column)
        let catalog2 = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(200, make_column(1, "id", "integer"))
            .with_column(200, make_column(2, "email", "text")); // Added column

        let matches = verify_state(&backend, &catalog2, "public").await.unwrap();
        assert!(!matches);
    }
}
