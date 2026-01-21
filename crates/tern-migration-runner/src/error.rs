//! Error types for the migration runner.
//!
//! This module defines all error types that can occur during migration
//! component loading, instantiation, and execution.

// False positives from thiserror/miette macro expansion
#![allow(unused_assignments)]

use miette::Diagnostic;
use thiserror::Error;

/// Errors that can occur during migration runtime operations.
#[derive(Debug, Error, Diagnostic)]
pub enum RuntimeError {
    /// Failed to create the WebAssembly engine.
    #[error("failed to create WebAssembly engine: {message}")]
    #[diagnostic(code(tern::runner::engine_creation))]
    EngineCreation {
        /// The error message.
        message: String,
    },

    /// Failed to load the WebAssembly component.
    #[error("failed to load WebAssembly component: {message}")]
    #[diagnostic(code(tern::runner::component_load))]
    ComponentLoad {
        /// The error message.
        message: String,
    },

    /// Failed to instantiate the WebAssembly component.
    #[error("failed to instantiate WebAssembly component: {message}")]
    #[diagnostic(code(tern::runner::instantiation))]
    Instantiation {
        /// The error message.
        message: String,
    },

    /// Failed to link host functions to the component.
    #[error("failed to link host functions: {message}")]
    #[diagnostic(code(tern::runner::linking))]
    Linking {
        /// The error message.
        message: String,
    },

    /// Failed to call a component function.
    #[error("failed to call component function '{function}': {message}")]
    #[diagnostic(code(tern::runner::function_call))]
    FunctionCall {
        /// The function that was called.
        function: String,
        /// The error message.
        message: String,
    },

    /// The migration execution failed.
    #[error("migration execution failed: {message}")]
    #[diagnostic(
        code(tern::runner::migration_failed),
        help("Check the error message for details about what went wrong")
    )]
    MigrationFailed {
        /// The error message from the migration component.
        message: String,
    },

    /// Invalid component: missing required exports.
    #[error("invalid component: {message}")]
    #[diagnostic(
        code(tern::runner::invalid_component),
        help("Ensure the component was compiled with a compatible version of tern")
    )]
    InvalidComponent {
        /// The error message.
        message: String,
    },
}

impl RuntimeError {
    /// Create an engine creation error.
    pub fn engine_creation(err: impl std::fmt::Display) -> Self {
        Self::EngineCreation {
            message: err.to_string(),
        }
    }

    /// Create a component load error.
    pub fn component_load(err: impl std::fmt::Display) -> Self {
        Self::ComponentLoad {
            message: err.to_string(),
        }
    }

    /// Create an instantiation error.
    pub fn instantiation(err: impl std::fmt::Display) -> Self {
        Self::Instantiation {
            message: err.to_string(),
        }
    }

    /// Create a linking error.
    pub fn linking(err: impl std::fmt::Display) -> Self {
        Self::Linking {
            message: err.to_string(),
        }
    }

    /// Create a function call error.
    pub fn function_call(function: impl Into<String>, err: impl std::fmt::Display) -> Self {
        Self::FunctionCall {
            function: function.into(),
            message: err.to_string(),
        }
    }

    /// Create a migration failed error.
    pub fn migration_failed(message: impl Into<String>) -> Self {
        Self::MigrationFailed {
            message: message.into(),
        }
    }

    /// Create an invalid component error.
    pub fn invalid_component(message: impl Into<String>) -> Self {
        Self::InvalidComponent {
            message: message.into(),
        }
    }
}

/// Errors that can occur during database operations.
#[derive(Debug, Error, Diagnostic)]
pub enum DatabaseError {
    /// Failed to connect to the database.
    #[error("failed to connect to database: {message}")]
    #[diagnostic(
        code(tern::runner::connection),
        help("Check that the database URL is correct and the database is running")
    )]
    Connection {
        /// The error message.
        message: String,
    },

    /// Failed to execute a SQL statement.
    #[error("SQL execution failed: {message}")]
    #[diagnostic(code(tern::runner::sql_execution))]
    SqlExecution {
        /// The error message.
        message: String,
        /// Optional PostgreSQL error code.
        code: Option<String>,
        /// Optional constraint name.
        constraint: Option<String>,
        /// Optional table name.
        table: Option<String>,
    },

    /// Failed to begin a transaction.
    #[error("failed to begin transaction: {message}")]
    #[diagnostic(code(tern::runner::transaction))]
    TransactionBegin {
        /// The error message.
        message: String,
    },

    /// Failed to commit a transaction.
    #[error("failed to commit transaction: {message}")]
    #[diagnostic(code(tern::runner::transaction))]
    TransactionCommit {
        /// The error message.
        message: String,
    },

    /// Failed to rollback a transaction.
    #[error("failed to rollback transaction: {message}")]
    #[diagnostic(code(tern::runner::transaction))]
    TransactionRollback {
        /// The error message.
        message: String,
    },

    /// Invalid database URL.
    #[error("invalid database URL: {message}")]
    #[diagnostic(
        code(tern::runner::invalid_url),
        help("The URL should be in the format: postgres://user:password@host:port/database")
    )]
    InvalidUrl {
        /// The error message.
        message: String,
    },
}

impl DatabaseError {
    /// Create a connection error.
    pub fn connection(err: impl std::fmt::Display) -> Self {
        Self::Connection {
            message: err.to_string(),
        }
    }

    /// Create a SQL execution error.
    pub fn sql_execution(err: impl std::fmt::Display) -> Self {
        Self::SqlExecution {
            message: err.to_string(),
            code: None,
            constraint: None,
            table: None,
        }
    }

    /// Create a SQL execution error with PostgreSQL error details.
    pub fn sql_execution_with_details(
        message: impl Into<String>,
        code: Option<String>,
        constraint: Option<String>,
        table: Option<String>,
    ) -> Self {
        Self::SqlExecution {
            message: message.into(),
            code,
            constraint,
            table,
        }
    }

    /// Create a transaction begin error.
    pub fn transaction_begin(err: impl std::fmt::Display) -> Self {
        Self::TransactionBegin {
            message: err.to_string(),
        }
    }

    /// Create a transaction commit error.
    pub fn transaction_commit(err: impl std::fmt::Display) -> Self {
        Self::TransactionCommit {
            message: err.to_string(),
        }
    }

    /// Create a transaction rollback error.
    pub fn transaction_rollback(err: impl std::fmt::Display) -> Self {
        Self::TransactionRollback {
            message: err.to_string(),
        }
    }

    /// Create an invalid URL error.
    pub fn invalid_url(err: impl std::fmt::Display) -> Self {
        Self::InvalidUrl {
            message: err.to_string(),
        }
    }
}

/// Errors that can occur during CLI operations.
#[derive(Debug, Error, Diagnostic)]
pub enum CliError {
    /// Failed to read component file.
    #[error("failed to read component file: {path}")]
    #[diagnostic(code(tern::runner::file_read))]
    FileRead {
        /// The file path.
        path: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// User cancelled the operation.
    #[error("operation cancelled by user")]
    #[diagnostic(code(tern::runner::cancelled))]
    Cancelled,

    /// Missing required argument.
    #[error("missing required argument: {argument}")]
    #[diagnostic(code(tern::runner::missing_argument))]
    MissingArgument {
        /// The missing argument name.
        argument: String,
    },

    /// Runtime error wrapper.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Runtime(#[from] RuntimeError),

    /// Database error wrapper.
    #[error(transparent)]
    #[diagnostic(transparent)]
    Database(#[from] DatabaseError),
}

impl CliError {
    /// Create a file read error.
    pub fn file_read(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::FileRead {
            path: path.into(),
            source,
        }
    }

    /// Create a cancelled error.
    pub fn cancelled() -> Self {
        Self::Cancelled
    }

    /// Create a missing argument error.
    pub fn missing_argument(argument: impl Into<String>) -> Self {
        Self::MissingArgument {
            argument: argument.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod runtime_error_tests {
        use super::*;

        #[test]
        fn engine_creation_error_display() {
            let err = RuntimeError::engine_creation("test error");
            assert_eq!(
                format!("{}", err),
                "failed to create WebAssembly engine: test error"
            );
        }

        #[test]
        fn component_load_error_display() {
            let err = RuntimeError::component_load("invalid magic bytes");
            assert!(format!("{}", err).contains("invalid magic bytes"));
        }

        #[test]
        fn instantiation_error_display() {
            let err = RuntimeError::instantiation("missing import");
            assert!(format!("{}", err).contains("missing import"));
        }

        #[test]
        fn linking_error_display() {
            let err = RuntimeError::linking("unknown function");
            assert!(format!("{}", err).contains("unknown function"));
        }

        #[test]
        fn function_call_error_display() {
            let err = RuntimeError::function_call("describe", "trap occurred");
            let msg = format!("{}", err);
            assert!(msg.contains("describe"));
            assert!(msg.contains("trap occurred"));
        }

        #[test]
        fn migration_failed_error_display() {
            let err = RuntimeError::migration_failed("constraint violation");
            assert!(format!("{}", err).contains("constraint violation"));
        }

        #[test]
        fn invalid_component_error_display() {
            let err = RuntimeError::invalid_component("missing migration export");
            assert!(format!("{}", err).contains("missing migration export"));
        }
    }

    mod database_error_tests {
        use super::*;

        #[test]
        fn connection_error_display() {
            let err = DatabaseError::connection("connection refused");
            assert!(format!("{}", err).contains("connection refused"));
        }

        #[test]
        fn sql_execution_error_display() {
            let err = DatabaseError::sql_execution("syntax error");
            assert!(format!("{}", err).contains("syntax error"));
        }

        #[test]
        fn sql_execution_with_details_display() {
            let err = DatabaseError::sql_execution_with_details(
                "unique violation",
                Some("23505".to_string()),
                Some("users_email_key".to_string()),
                Some("users".to_string()),
            );
            let msg = format!("{:?}", err);
            assert!(msg.contains("unique violation"));
            assert!(msg.contains("23505"));
            assert!(msg.contains("users_email_key"));
        }

        #[test]
        fn transaction_begin_error_display() {
            let err = DatabaseError::transaction_begin("connection lost");
            assert!(format!("{}", err).contains("connection lost"));
        }

        #[test]
        fn transaction_commit_error_display() {
            let err = DatabaseError::transaction_commit("serialization failure");
            assert!(format!("{}", err).contains("serialization failure"));
        }

        #[test]
        fn transaction_rollback_error_display() {
            let err = DatabaseError::transaction_rollback("connection lost");
            assert!(format!("{}", err).contains("connection lost"));
        }

        #[test]
        fn invalid_url_error_display() {
            let err = DatabaseError::invalid_url("missing host");
            assert!(format!("{}", err).contains("missing host"));
        }
    }

    mod cli_error_tests {
        use super::*;

        #[test]
        fn file_read_error_display() {
            let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
            let err = CliError::file_read("/path/to/file.wasm", io_err);
            assert!(format!("{}", err).contains("/path/to/file.wasm"));
        }

        #[test]
        fn cancelled_error_display() {
            let err = CliError::cancelled();
            assert!(format!("{}", err).contains("cancelled"));
        }

        #[test]
        fn missing_argument_error_display() {
            let err = CliError::missing_argument("--database-url");
            assert!(format!("{}", err).contains("--database-url"));
        }

        #[test]
        fn from_runtime_error() {
            let runtime_err = RuntimeError::migration_failed("test");
            let cli_err: CliError = runtime_err.into();
            assert!(matches!(cli_err, CliError::Runtime(_)));
        }

        #[test]
        fn from_database_error() {
            let db_err = DatabaseError::connection("test");
            let cli_err: CliError = db_err.into();
            assert!(matches!(cli_err, CliError::Database(_)));
        }
    }
}
