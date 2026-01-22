//! MCP server error types.

use thiserror::Error;

/// Errors that can occur in the MCP server.
#[derive(Debug, Error)]
pub enum McpError {
    /// No active session exists.
    #[error("no active session; call session_start first")]
    NoSession,

    /// Session already exists.
    #[error("session already exists; call session_reset to start fresh")]
    SessionExists,

    /// Table not found.
    #[error("table '{name}' not found")]
    TableNotFound {
        name: String,
        available: Vec<String>,
    },

    /// Column not found.
    #[error("column '{column}' not found in table '{table}'")]
    ColumnNotFound { table: String, column: String },

    /// Constraint not found.
    #[error("constraint '{constraint}' not found on table '{table}'")]
    ConstraintNotFound { table: String, constraint: String },

    /// Index not found.
    #[error("index '{name}' not found")]
    IndexNotFound { name: String },

    /// Enum not found.
    #[error("enum type '{name}' not found")]
    EnumNotFound { name: String },

    /// Object already exists.
    #[error("{kind} '{name}' already exists")]
    DuplicateObject { kind: String, name: String },

    /// Invalid PostgreSQL type.
    #[error("invalid PostgreSQL type: {type_name}")]
    InvalidType {
        type_name: String,
        suggestion: Option<String>,
    },

    /// Constraint violation.
    #[error("constraint violation: {message}")]
    ConstraintViolation { message: String },

    /// PGLite execution error.
    #[error("PGLite error: {message}")]
    PgLiteError { message: String },

    /// State backend error.
    #[error("state error: {message}")]
    StateError { message: String },

    /// Invalid input.
    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    /// Internal error.
    #[error("internal error: {message}")]
    Internal { message: String },

    /// No changes to migrate.
    #[error("no pending changes to migrate")]
    NoChanges,

    /// Breaking changes present.
    #[error("migration has breaking changes; use force=true to proceed")]
    BreakingChanges { changes: Vec<String> },
}

impl McpError {
    /// Returns an error code string for the error.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoSession => "NO_SESSION",
            Self::SessionExists => "SESSION_EXISTS",
            Self::TableNotFound { .. } => "TABLE_NOT_FOUND",
            Self::ColumnNotFound { .. } => "COLUMN_NOT_FOUND",
            Self::ConstraintNotFound { .. } => "CONSTRAINT_NOT_FOUND",
            Self::IndexNotFound { .. } => "INDEX_NOT_FOUND",
            Self::EnumNotFound { .. } => "ENUM_NOT_FOUND",
            Self::DuplicateObject { .. } => "DUPLICATE_OBJECT",
            Self::InvalidType { .. } => "INVALID_TYPE",
            Self::ConstraintViolation { .. } => "CONSTRAINT_VIOLATION",
            Self::PgLiteError { .. } => "PGLITE_ERROR",
            Self::StateError { .. } => "STATE_ERROR",
            Self::InvalidInput { .. } => "INVALID_INPUT",
            Self::Internal { .. } => "INTERNAL_ERROR",
            Self::NoChanges => "NO_CHANGES",
            Self::BreakingChanges { .. } => "BREAKING_CHANGES",
        }
    }

    /// Returns a suggestion for fixing the error, if available.
    #[must_use]
    pub fn suggestion(&self) -> Option<String> {
        match self {
            Self::NoSession => Some("Call session_start first to initialize a session".to_string()),
            Self::TableNotFound { available, .. } => {
                if !available.is_empty() {
                    Some(format!("Available tables: {}", available.join(", ")))
                } else {
                    None
                }
            }
            Self::InvalidType { suggestion, .. } => suggestion.clone(),
            _ => None,
        }
    }
}

/// Result type for MCP operations.
pub type McpResult<T> = Result<T, McpError>;

impl From<crate::db::pglite::PgLiteError> for McpError {
    fn from(e: crate::db::pglite::PgLiteError) -> Self {
        Self::PgLiteError {
            message: e.to_string(),
        }
    }
}

impl From<crate::db::state::StateError> for McpError {
    fn from(e: crate::db::state::StateError) -> Self {
        Self::StateError {
            message: e.to_string(),
        }
    }
}

impl From<crate::db::query::QueryError> for McpError {
    fn from(e: crate::db::query::QueryError) -> Self {
        Self::Internal {
            message: e.to_string(),
        }
    }
}
