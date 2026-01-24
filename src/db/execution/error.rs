//! Execution error types.
//!
//! This module defines error types for migration execution against live databases.

// Note: The unused_assignments warning is a false positive from thiserror macro expansion.
// The struct fields in HistoryDiverged are used in the #[error] format string.
#![allow(unused_assignments)]

use crate::db::state::MigrationId;

/// Error type for migration execution operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum ExecutionError {
    /// Database connection failed.
    #[error("failed to connect to database: {0}")]
    #[diagnostic(code(tern::execution::connection))]
    Connection(#[from] tokio_postgres::Error),

    /// Failed to create the tern tracking schema.
    #[error("failed to create tern tracking schema: {0}")]
    #[diagnostic(code(tern::execution::create_schema))]
    CreateSchema(String),

    /// Migration hash mismatch - history has diverged.
    #[error(
        "migration history has diverged: local migration {migration_id} has different content than recorded in database (expected: {expected_hash}, actual: {actual_hash})"
    )]
    #[diagnostic(
        code(tern::execution::history_diverged),
        help(
            "The local migration file has been modified since it was applied. Investigate why the migration changed."
        )
    )]
    HistoryDiverged {
        migration_id: String,
        expected_hash: String,
        actual_hash: String,
    },

    /// Migration execution failed.
    #[error("failed to execute migration {migration_id}: {message}")]
    #[diagnostic(code(tern::execution::migration_failed))]
    MigrationFailed {
        migration_id: String,
        message: String,
    },

    /// Migration not found in local state.
    #[error("migration {0} not found in local state")]
    #[diagnostic(code(tern::execution::migration_not_found))]
    MigrationNotFound(String),

    /// Failed to render migration to SQL.
    #[error("failed to render migration to SQL: {0}")]
    #[diagnostic(code(tern::execution::render_failed))]
    RenderFailed(String),

    /// Failed to compute schema checksum.
    #[error("failed to compute schema checksum: {0}")]
    #[diagnostic(code(tern::execution::checksum_failed))]
    ChecksumFailed(String),

    /// Transaction error.
    #[error("transaction error: {0}")]
    #[diagnostic(code(tern::execution::transaction))]
    Transaction(String),

    /// Invalid migration state.
    #[error("invalid migration state: {0}")]
    #[diagnostic(code(tern::execution::invalid_state))]
    InvalidState(String),

    /// Query error.
    #[error("query error: {0}")]
    #[diagnostic(code(tern::execution::query))]
    Query(String),

    /// No migrations to revert.
    #[error("no migrations have been applied to the database")]
    #[diagnostic(
        code(tern::execution::no_migrations_to_revert),
        help("Run 'tern up' to apply migrations first before attempting to revert.")
    )]
    NoMigrationsToRevert,

    /// Revert operation failed.
    #[error("failed to revert migration {migration_id}: {message}")]
    #[diagnostic(code(tern::execution::revert_failed))]
    RevertFailed {
        migration_id: String,
        message: String,
    },

    /// Cannot revert baseline migration.
    #[error("cannot revert baseline migration {0}")]
    #[diagnostic(
        code(tern::execution::cannot_revert_baseline),
        help("The baseline migration represents the initial state and cannot be reverted.")
    )]
    CannotRevertBaseline(String),
}

/// Represents the result of applying a single migration.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    /// The migration ID that was applied.
    pub migration_id: MigrationId,
    /// The description of the migration.
    pub description: String,
    /// The number of SQL statements executed.
    pub statement_count: usize,
    /// The schema hash after applying the migration.
    pub schema_hash: String,
}

/// Represents the overall result of a migration execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// The migrations that were applied.
    pub applied: Vec<MigrationResult>,
    /// Whether all migrations were successful.
    pub success: bool,
    /// Error message if any migration failed.
    pub error: Option<String>,
}

impl ExecutionResult {
    /// Creates a successful execution result.
    pub fn success(applied: Vec<MigrationResult>) -> Self {
        Self {
            applied,
            success: true,
            error: None,
        }
    }

    /// Creates a failed execution result.
    pub fn failure(applied: Vec<MigrationResult>, error: String) -> Self {
        Self {
            applied,
            success: false,
            error: Some(error),
        }
    }

    /// Returns true if no migrations were applied.
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }

    /// Returns the number of migrations applied.
    pub fn count(&self) -> usize {
        self.applied.len()
    }
}
