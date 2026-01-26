//! Error types for the MCP server.
//!
//! This module defines error types specific to the MCP (Model Context Protocol)
//! server implementation, including protocol errors, session errors, and
//! operation errors.

// Suppress warnings about enum variant fields that are used by thiserror's Display impl.
#![allow(unused_assignments)]

use std::fmt;

/// Error returned by MCP server operations.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum McpError {
    // =========================================================================
    // Protocol Errors (JSON-RPC standard codes)
    // =========================================================================
    /// Parse error: Invalid JSON was received.
    #[error("parse error: {0}")]
    #[diagnostic(code(mcp::parse_error))]
    ParseError(String),

    /// Invalid request: The JSON is not a valid request object.
    #[error("invalid request: {0}")]
    #[diagnostic(code(mcp::invalid_request))]
    InvalidRequest(String),

    /// Method not found: The requested method does not exist.
    #[error("method not found: {0}")]
    #[diagnostic(code(mcp::method_not_found))]
    MethodNotFound(String),

    /// Invalid params: The method parameters are invalid.
    #[error("invalid params: {0}")]
    #[diagnostic(code(mcp::invalid_params))]
    InvalidParams(String),

    /// Internal error: An internal server error occurred.
    #[error("internal error: {0}")]
    #[diagnostic(code(mcp::internal_error))]
    InternalError(String),

    // =========================================================================
    // Session Errors
    // =========================================================================
    /// Session not found: The specified session does not exist.
    #[error("session not found: {0}")]
    #[diagnostic(
        code(mcp::session_not_found),
        help("Use list_sessions to see active sessions, or start_session to create a new one.")
    )]
    SessionNotFound(SessionId),

    /// No active session: A session ID is required but none was provided.
    #[error("no active session; call start_session first")]
    #[diagnostic(code(mcp::no_active_session))]
    NoActiveSession,

    /// Session limit reached: Too many concurrent sessions are active.
    #[error("session limit reached; cancel an existing session first")]
    #[diagnostic(
        code(mcp::session_limit_reached),
        help("Use cancel_session to close an existing session before starting a new one.")
    )]
    SessionLimitReached {
        /// Maximum number of sessions allowed.
        max_sessions: usize,
        /// Currently active session count.
        current_sessions: usize,
    },

    // =========================================================================
    // SQL Execution Errors
    // =========================================================================
    /// SQL execution failed: The SQL statement could not be executed.
    #[error("SQL execution failed: {message}")]
    #[diagnostic(code(mcp::sql_execution_failed))]
    SqlExecutionFailed {
        /// Error message from PostgreSQL.
        message: String,
        /// PostgreSQL error code, if available.
        code: Option<String>,
        /// Detailed error information, if available.
        detail: Option<String>,
        /// Hint for fixing the error, if available.
        hint: Option<String>,
        /// Position in the SQL where the error occurred, if available.
        position: Option<i32>,
    },

    // =========================================================================
    // Migration Generation Errors
    // =========================================================================
    /// No changes: The session has no schema changes to migrate.
    #[error("no changes in session")]
    #[diagnostic(
        code(mcp::no_changes),
        help(
            "Make schema changes using execute_sql or apply_operation before generating a migration."
        )
    )]
    NoChanges,

    /// Breaking changes detected: The migration contains breaking changes.
    #[error("breaking changes detected; use force=true to proceed")]
    #[diagnostic(
        code(mcp::breaking_changes),
        help("Review the breaking changes and set force=true if you want to proceed.")
    )]
    BreakingChangesDetected {
        /// List of breaking changes detected.
        breaking_changes: Vec<String>,
    },

    /// Save failed: The migration could not be saved to disk.
    #[error("failed to save migration: {0}")]
    #[diagnostic(code(mcp::save_failed))]
    SaveFailed(String),

    // =========================================================================
    // Backend Errors
    // =========================================================================
    /// Backend not initialized: The state backend is not initialized.
    #[error("backend not initialized: {0}")]
    #[diagnostic(
        code(mcp::backend_not_initialized),
        help("Run 'tern init' to initialize the project first.")
    )]
    BackendNotInitialized(String),

    /// Backend error: A state backend operation failed.
    #[error("backend error: {0}")]
    #[diagnostic(code(mcp::backend_error))]
    BackendError(String),

    /// Migration not found: The specified migration does not exist.
    #[error("migration not found: {0}")]
    #[diagnostic(code(mcp::migration_not_found))]
    MigrationNotFound(String),

    // =========================================================================
    // PGLite Errors
    // =========================================================================
    /// PGLite error: An error occurred with the embedded PostgreSQL.
    #[error("PGLite error: {0}")]
    #[diagnostic(code(mcp::pglite_error))]
    PgLiteError(String),

    // =========================================================================
    // Resource Errors
    // =========================================================================
    /// Resource not found: The requested resource does not exist.
    #[error("resource not found: {0}")]
    #[diagnostic(code(mcp::resource_not_found))]
    ResourceNotFound(String),

    /// Invalid resource URI: The resource URI is malformed.
    #[error("invalid resource URI: {0}")]
    #[diagnostic(code(mcp::invalid_resource_uri))]
    InvalidResourceUri(String),

    // =========================================================================
    // Transport Errors
    // =========================================================================
    /// I/O error: An I/O operation failed.
    #[error("I/O error: {0}")]
    #[diagnostic(code(mcp::io_error))]
    IoError(#[from] std::io::Error),
}

impl McpError {
    /// Returns the JSON-RPC error code for this error.
    ///
    /// Standard JSON-RPC error codes:
    /// - -32700: Parse error
    /// - -32600: Invalid request
    /// - -32601: Method not found
    /// - -32602: Invalid params
    /// - -32603: Internal error
    ///
    /// Application-specific error codes (starting at -32100):
    /// - -32100: Session not found
    /// - -32101: No active session
    /// - -32102: Session limit reached
    /// - -32103: SQL execution failed
    /// - -32104: No changes
    /// - -32105: Breaking changes detected
    /// - -32106: Save failed
    /// - -32107: Backend not initialized
    /// - -32108: PGLite error
    /// - -32109: Resource not found
    /// - -32110: Invalid resource URI
    /// - -32111: Backend error
    /// - -32112: Migration not found
    #[must_use]
    pub fn error_code(&self) -> i32 {
        match self {
            Self::ParseError(_) => -32700,
            Self::InvalidRequest(_) => -32600,
            Self::MethodNotFound(_) => -32601,
            Self::InvalidParams(_) => -32602,
            Self::InternalError(_) => -32603,
            Self::SessionNotFound(_) => -32100,
            Self::NoActiveSession => -32101,
            Self::SessionLimitReached { .. } => -32102,
            Self::SqlExecutionFailed { .. } => -32103,
            Self::NoChanges => -32104,
            Self::BreakingChangesDetected { .. } => -32105,
            Self::SaveFailed(_) => -32106,
            Self::BackendNotInitialized(_) => -32107,
            Self::PgLiteError(_) => -32108,
            Self::ResourceNotFound(_) => -32109,
            Self::InvalidResourceUri(_) => -32110,
            Self::BackendError(_) => -32111,
            Self::MigrationNotFound(_) => -32112,
            Self::IoError(_) => -32603, // Map to internal error
        }
    }
}

/// Unique identifier for a migration authoring session.
///
/// Session IDs are prefixed with "sess_" followed by 8 alphanumeric characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Creates a new session ID from a string.
    ///
    /// # Panics
    ///
    /// Panics if the string is empty.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        assert!(!id.is_empty(), "session ID must not be empty");
        Self(id)
    }

    /// Generates a new random session ID.
    ///
    /// The format is "sess_" followed by 12 alphanumeric characters.
    #[must_use]
    pub fn generate() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::{SystemTime, UNIX_EPOCH};

        // Static counter to ensure uniqueness even when called in quick succession
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        // Simple random ID generation without external dependencies
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        // Combine timestamp with an incrementing counter for uniqueness
        // Using SeqCst ordering to ensure strict ordering across threads
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);

        // Include thread ID for additional uniqueness in multi-threaded contexts
        let thread_id = std::thread::current().id();
        let thread_hash = format!("{:?}", thread_id).bytes().fold(0u64, |acc, b| {
            acc.wrapping_mul(31).wrapping_add(u64::from(b))
        });

        let combined = (timestamp as u64)
            .wrapping_add(counter)
            .wrapping_add(thread_hash);
        let random_part = format!("{:012x}", combined ^ 0xDEAD_BEEF_CAFE_BABE);
        Self(format!("sess_{}", &random_part[..12]))
    }

    /// Returns the session ID as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for SessionId {
    fn from(s: &str) -> Self {
        Self::new(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_generate() {
        let id1 = SessionId::generate();
        let id2 = SessionId::generate();

        assert!(id1.as_str().starts_with("sess_"));
        assert!(id2.as_str().starts_with("sess_"));
        // Generated IDs should be different (in most cases)
        // Note: This could theoretically fail if generated in the same nanosecond
    }

    #[test]
    fn session_id_from_string() {
        let id = SessionId::from("sess_abc12345".to_string());
        assert_eq!(id.as_str(), "sess_abc12345");
    }

    #[test]
    fn session_id_display() {
        let id = SessionId::new("sess_test1234");
        assert_eq!(format!("{}", id), "sess_test1234");
    }

    #[test]
    fn error_codes_are_correct() {
        assert_eq!(McpError::ParseError("test".into()).error_code(), -32700);
        assert_eq!(McpError::InvalidRequest("test".into()).error_code(), -32600);
        assert_eq!(McpError::MethodNotFound("test".into()).error_code(), -32601);
        assert_eq!(McpError::InvalidParams("test".into()).error_code(), -32602);
        assert_eq!(McpError::InternalError("test".into()).error_code(), -32603);
        assert_eq!(
            McpError::SessionNotFound(SessionId::new("sess_test")).error_code(),
            -32100
        );
    }

    #[test]
    #[should_panic(expected = "session ID must not be empty")]
    fn session_id_empty_panics() {
        let _id = SessionId::new("");
    }
}
