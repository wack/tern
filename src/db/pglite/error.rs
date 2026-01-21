//! Error types for PGLite operations.
//!
//! This module defines error types for:
//! - PGLite runtime management
//! - SQL execution
//! - Worklist execution with dependency resolution

use std::path::PathBuf;

/// Error returned by PGLite operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum PgLiteError {
    /// Failed to initialize PGLite runtime.
    #[error("failed to initialize PGLite runtime: {message}")]
    RuntimeInit { message: String },

    /// Failed to start PGLite instance.
    #[error("failed to start PGLite instance: {message}")]
    RuntimeStart { message: String },

    /// Failed to connect to PGLite instance.
    #[error("failed to connect to PGLite: {source}")]
    Connection {
        #[source]
        source: tokio_postgres::Error,
    },

    /// Failed to read SQL file.
    #[error("failed to read SQL file {}: {source}", path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to execute SQL.
    #[error("failed to execute SQL: {source}")]
    ExecuteSql {
        #[source]
        source: tokio_postgres::Error,
    },

    /// Invalid glob pattern for finding SQL files.
    #[error("invalid glob pattern '{pattern}': {source}")]
    InvalidGlobPattern {
        pattern: String,
        #[source]
        source: glob::PatternError,
    },

    /// No SQL files found in directory.
    #[error("no SQL files found in directory: {}", path.display())]
    NoSqlFiles { path: PathBuf },

    /// Worklist execution error.
    #[error(transparent)]
    Worklist(#[from] WorklistError),

    /// Query error during introspection.
    #[error("schema introspection failed: {source}")]
    Introspection {
        #[from]
        source: crate::db::query::QueryError,
    },

    /// PGLite feature not enabled.
    #[error(
        "PGLite feature is not enabled; rebuild with `--features pglite` or use a real PostgreSQL connection"
    )]
    FeatureNotEnabled,
}

/// Error returned by the worklist executor.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum WorklistError {
    /// Circular dependency detected among SQL files.
    ///
    /// This error occurs when the worklist algorithm cannot make progress
    /// because all remaining files have unresolved dependencies on each other.
    #[error("circular dependency detected among schema files")]
    #[diagnostic(help(
        "The following files could not be executed in any order:\n{}\n\n\
         For circular foreign key references, use ALTER TABLE ADD CONSTRAINT \
         in a separate file that runs after both tables are created.",
        format_pending_files(.pending_files)
    ))]
    CircularDependency {
        /// Files that could not be executed due to circular dependencies.
        pending_files: Vec<PathBuf>,
    },

    /// SQL execution failed with a non-retryable error.
    #[error("SQL execution failed for {}: {source}", path.display())]
    ExecutionFailed {
        path: PathBuf,
        #[source]
        source: tokio_postgres::Error,
    },

    /// Failed to read SQL file.
    #[error("failed to read SQL file {}: {source}", path.display())]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Formats a list of pending files for display in error messages.
fn format_pending_files(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// PostgreSQL error codes that indicate missing dependencies.
///
/// These errors are retryable because the dependency might be created
/// by a file that hasn't been executed yet.
pub mod error_codes {
    /// Undefined table (42P01)
    ///
    /// Occurs when referencing a table that doesn't exist yet.
    pub const UNDEFINED_TABLE: &str = "42P01";

    /// Undefined function (42883)
    ///
    /// Occurs when referencing a function that doesn't exist yet.
    pub const UNDEFINED_FUNCTION: &str = "42883";

    /// Undefined object (42704)
    ///
    /// Occurs when referencing any object that doesn't exist yet.
    pub const UNDEFINED_OBJECT: &str = "42704";

    /// Undefined column (42703)
    ///
    /// Occurs when referencing a column that doesn't exist yet.
    pub const UNDEFINED_COLUMN: &str = "42703";

    /// Invalid schema name (3F000)
    ///
    /// Occurs when referencing a schema that doesn't exist yet.
    pub const INVALID_SCHEMA_NAME: &str = "3F000";

    /// Duplicate schema (42P06)
    ///
    /// Occurs when trying to create a schema that already exists.
    /// This is skippable, not retryable.
    pub const DUPLICATE_SCHEMA: &str = "42P06";

    /// Duplicate table (42P07)
    ///
    /// Occurs when trying to create a table that already exists.
    /// This is skippable, not retryable.
    pub const DUPLICATE_TABLE: &str = "42P07";

    /// Duplicate object (42710)
    ///
    /// Occurs when trying to create an object that already exists.
    /// This is skippable, not retryable.
    pub const DUPLICATE_OBJECT: &str = "42710";

    /// Returns true if the error code indicates a missing dependency
    /// that might be resolved by executing other files first.
    pub fn is_dependency_error(code: &str) -> bool {
        matches!(
            code,
            UNDEFINED_TABLE
                | UNDEFINED_FUNCTION
                | UNDEFINED_OBJECT
                | UNDEFINED_COLUMN
                | INVALID_SCHEMA_NAME
        )
    }

    /// Returns true if the error code indicates the object already exists
    /// and the file can be skipped.
    pub fn is_duplicate_error(code: &str) -> bool {
        matches!(code, DUPLICATE_SCHEMA | DUPLICATE_TABLE | DUPLICATE_OBJECT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_classification() {
        assert!(error_codes::is_dependency_error(
            error_codes::UNDEFINED_TABLE
        ));
        assert!(error_codes::is_dependency_error(
            error_codes::UNDEFINED_FUNCTION
        ));
        assert!(error_codes::is_dependency_error(
            error_codes::UNDEFINED_OBJECT
        ));

        assert!(!error_codes::is_dependency_error(
            error_codes::DUPLICATE_TABLE
        ));
        assert!(!error_codes::is_dependency_error("42000")); // syntax error

        assert!(error_codes::is_duplicate_error(
            error_codes::DUPLICATE_TABLE
        ));
        assert!(error_codes::is_duplicate_error(
            error_codes::DUPLICATE_SCHEMA
        ));
        assert!(!error_codes::is_duplicate_error(
            error_codes::UNDEFINED_TABLE
        ));
    }
}
