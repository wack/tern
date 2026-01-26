//! Session management for migration authoring.
//!
//! This module provides the session management functionality for the MCP server,
//! allowing users to create isolated editing sessions where they can make schema
//! changes and eventually generate migrations.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::RwLock;

use crate::db::migrate::Operation;
use crate::db::model::Namespace;
#[cfg(feature = "pglite")]
use crate::db::pglite::PgLiteRuntime;
use crate::db::state::LocalFileBackend;
#[cfg(feature = "pglite")]
use crate::db::state::SchemaExporter;
use crate::mcp::error::{McpError, SessionId};

/// Default maximum number of concurrent sessions.
pub const DEFAULT_MAX_SESSIONS: usize = 5;

/// Record of an operation applied in a session.
#[derive(Debug, Clone)]
pub struct OperationRecord {
    /// The structured operation, if this was applied via `apply_operation`.
    pub operation: Option<Operation>,
    /// The SQL that was executed.
    pub sql: String,
    /// When the operation was applied.
    pub applied_at: Timestamp,
}

/// An active migration authoring session.
pub struct Session {
    /// Unique identifier for this session.
    pub id: SessionId,
    /// User-provided description for the migration being created.
    pub description: String,
    /// When the session was started.
    pub started_at: Timestamp,
    /// Schema state at session start (immutable, used for diffing).
    pub base_state: Namespace,
    /// History of SQL statements executed in this session.
    pub sql_history: Vec<String>,
    /// History of operations applied in this session.
    pub operation_history: Vec<OperationRecord>,
    /// The PGLite runtime for this session's database.
    #[cfg(feature = "pglite")]
    pub runtime: Option<PgLiteRuntime>,
}

impl Session {
    /// Creates a new session.
    #[cfg(feature = "pglite")]
    pub fn new(id: SessionId, description: String, base_state: Namespace) -> Self {
        Self {
            id,
            description,
            started_at: Timestamp::now(),
            base_state,
            sql_history: Vec::new(),
            operation_history: Vec::new(),
            runtime: None,
        }
    }

    /// Creates a new session (non-pglite version).
    #[cfg(not(feature = "pglite"))]
    pub fn new(id: SessionId, description: String, base_state: Namespace) -> Self {
        Self {
            id,
            description,
            started_at: Timestamp::now(),
            base_state,
            sql_history: Vec::new(),
            operation_history: Vec::new(),
        }
    }

    /// Gets a PGLite client connection for this session.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime is not initialized or the connection fails.
    #[cfg(feature = "pglite")]
    pub async fn get_client(&self) -> Result<tokio_postgres::Client, McpError> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| McpError::PgLiteError("session database not initialized".into()))?;
        runtime
            .client()
            .await
            .map_err(|e| McpError::PgLiteError(e.to_string()))
    }

    /// Returns the number of operations in this session.
    #[must_use]
    pub fn operation_count(&self) -> usize {
        self.operation_history.len()
    }

    /// Returns true if this session has any changes.
    #[must_use]
    pub fn has_changes(&self) -> bool {
        !self.operation_history.is_empty()
    }

    /// Records an SQL execution in the session history.
    pub fn record_sql(&mut self, sql: String) {
        self.sql_history.push(sql.clone());
        self.operation_history.push(OperationRecord {
            operation: None,
            sql,
            applied_at: Timestamp::now(),
        });
    }

    /// Records a structured operation in the session history.
    pub fn record_operation(&mut self, operation: Operation, sql: String) {
        self.sql_history.push(sql.clone());
        self.operation_history.push(OperationRecord {
            operation: Some(operation),
            sql,
            applied_at: Timestamp::now(),
        });
    }
}

/// Summary of a session for listing.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// Session ID.
    pub session_id: String,
    /// Migration description.
    pub description: String,
    /// When the session was started.
    pub started_at: String,
    /// Number of operations applied.
    pub operation_count: usize,
    /// Whether the session has any changes.
    pub has_changes: bool,
}

impl From<&Session> for SessionSummary {
    fn from(session: &Session) -> Self {
        Self {
            session_id: session.id.to_string(),
            description: session.description.clone(),
            started_at: session.started_at.to_string(),
            operation_count: session.operation_count(),
            has_changes: session.has_changes(),
        }
    }
}

/// Manages all active migration authoring sessions.
pub struct SessionManager {
    /// Active sessions indexed by ID.
    sessions: HashMap<SessionId, Session>,
    /// Base schema state (shared across all sessions).
    base_state: Arc<Namespace>,
    /// State backend for saving migrations.
    backend: Arc<LocalFileBackend>,
    /// Maximum number of concurrent sessions.
    max_sessions: usize,
    /// PGLite runtime for session databases.
    #[cfg(feature = "pglite")]
    _pglite_runtime: Option<crate::db::pglite::PgLiteRuntime>,
}

impl SessionManager {
    /// Creates a new session manager.
    pub fn new(base_state: Namespace, backend: LocalFileBackend) -> Self {
        Self {
            sessions: HashMap::new(),
            base_state: Arc::new(base_state),
            backend: Arc::new(backend),
            max_sessions: DEFAULT_MAX_SESSIONS,
            #[cfg(feature = "pglite")]
            _pglite_runtime: None,
        }
    }

    /// Sets the maximum number of concurrent sessions.
    pub fn with_max_sessions(mut self, max: usize) -> Self {
        self.max_sessions = max;
        self
    }

    /// Returns the base schema state.
    #[must_use]
    pub fn base_state(&self) -> &Namespace {
        &self.base_state
    }

    /// Returns a reference to the state backend.
    #[must_use]
    pub fn backend(&self) -> &LocalFileBackend {
        &self.backend
    }

    /// Starts a new migration authoring session.
    ///
    /// Note: PGLite initialization is deferred until the first SQL operation
    /// to keep session creation fast for tests and quick operations.
    ///
    /// # Errors
    ///
    /// Returns an error if the session limit has been reached.
    pub async fn start_session(&mut self, description: String) -> Result<SessionId, McpError> {
        // Check session limit
        if self.sessions.len() >= self.max_sessions {
            return Err(McpError::SessionLimitReached {
                max_sessions: self.max_sessions,
                current_sessions: self.sessions.len(),
            });
        }

        // Generate session ID
        let id = SessionId::generate();

        // Create session with base state (PGLite is initialized lazily)
        let session = Session::new(id.clone(), description, (*self.base_state).clone());

        // Store session
        self.sessions.insert(id.clone(), session);

        Ok(id)
    }

    /// Ensures a session's PGLite database is initialized.
    ///
    /// This is called lazily when the first SQL operation is performed.
    ///
    /// # Errors
    ///
    /// Returns an error if PGLite initialization fails.
    #[cfg(feature = "pglite")]
    pub async fn ensure_session_initialized(&mut self, id: &SessionId) -> Result<(), McpError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| McpError::SessionNotFound(id.clone()))?;

        // Already initialized?
        if session.runtime.is_some() {
            return Ok(());
        }

        // Initialize PGLite database for this session
        let mut runtime = PgLiteRuntime::new().map_err(|e| McpError::PgLiteError(e.to_string()))?;
        runtime
            .start()
            .await
            .map_err(|e| McpError::PgLiteError(e.to_string()))?;

        // Generate DDL from base state and execute it
        let ddl = SchemaExporter::export(&session.base_state);
        let client = runtime
            .client()
            .await
            .map_err(|e| McpError::PgLiteError(e.to_string()))?;
        client.batch_execute(&ddl).await.map_err(|e| {
            McpError::PgLiteError(format!("failed to initialize session database: {e}"))
        })?;

        session.runtime = Some(runtime);

        Ok(())
    }

    /// Gets a session by ID.
    #[must_use]
    pub fn get_session(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.get(id)
    }

    /// Gets a mutable session by ID.
    #[must_use]
    pub fn get_session_mut(&mut self, id: &SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// Cancels and removes a session.
    ///
    /// # Errors
    ///
    /// Returns an error if the session is not found.
    pub async fn cancel_session(&mut self, id: &SessionId) -> Result<usize, McpError> {
        let session = self
            .sessions
            .remove(id)
            .ok_or_else(|| McpError::SessionNotFound(id.clone()))?;

        let discarded = session.operation_count();
        Ok(discarded)
    }

    /// Lists all active sessions.
    #[must_use]
    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        self.sessions.values().map(SessionSummary::from).collect()
    }

    /// Returns the number of active sessions.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Removes a session from the manager (used after generating a migration).
    pub fn remove_session(&mut self, id: &SessionId) -> Option<Session> {
        self.sessions.remove(id)
    }
}

/// Thread-safe wrapper around `SessionManager`.
#[allow(dead_code)]
pub type SharedSessionManager = Arc<RwLock<SessionManager>>;

/// Creates a new shared session manager.
#[must_use]
#[allow(dead_code)]
pub fn shared_session_manager(
    base_state: Namespace,
    backend: LocalFileBackend,
) -> SharedSessionManager {
    Arc::new(RwLock::new(SessionManager::new(base_state, backend)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_backend() -> (TempDir, LocalFileBackend) {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));
        (temp_dir, backend)
    }

    #[tokio::test]
    async fn start_session_generates_id() {
        let (_temp_dir, backend) = create_test_backend();
        let base_state = Namespace::empty("public");
        let mut manager = SessionManager::new(base_state, backend);

        let id = manager
            .start_session("Test migration".into())
            .await
            .unwrap();

        assert!(id.as_str().starts_with("sess_"));
        assert_eq!(manager.session_count(), 1);
    }

    #[tokio::test]
    async fn get_session_returns_session() {
        let (_temp_dir, backend) = create_test_backend();
        let base_state = Namespace::empty("public");
        let mut manager = SessionManager::new(base_state, backend);

        let id = manager
            .start_session("Test migration".into())
            .await
            .unwrap();
        let session = manager.get_session(&id);

        assert!(session.is_some());
        assert_eq!(session.unwrap().description, "Test migration");
    }

    #[tokio::test]
    async fn cancel_session_removes_session() {
        let (_temp_dir, backend) = create_test_backend();
        let base_state = Namespace::empty("public");
        let mut manager = SessionManager::new(base_state, backend);

        let id = manager
            .start_session("Test migration".into())
            .await
            .unwrap();
        assert_eq!(manager.session_count(), 1);

        let discarded = manager.cancel_session(&id).await.unwrap();
        assert_eq!(discarded, 0);
        assert_eq!(manager.session_count(), 0);
    }

    #[tokio::test]
    async fn session_limit_enforced() {
        let (_temp_dir, backend) = create_test_backend();
        let base_state = Namespace::empty("public");
        let mut manager = SessionManager::new(base_state, backend).with_max_sessions(2);

        manager.start_session("Session 1".into()).await.unwrap();
        manager.start_session("Session 2".into()).await.unwrap();

        let result = manager.start_session("Session 3".into()).await;
        assert!(matches!(result, Err(McpError::SessionLimitReached { .. })));
    }

    #[tokio::test]
    async fn list_sessions_returns_summaries() {
        let (_temp_dir, backend) = create_test_backend();
        let base_state = Namespace::empty("public");
        let mut manager = SessionManager::new(base_state, backend);

        manager.start_session("First".into()).await.unwrap();
        manager.start_session("Second".into()).await.unwrap();

        let sessions = manager.list_sessions();
        assert_eq!(sessions.len(), 2);
    }
}
