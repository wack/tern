//! Session state management for the MCP server.
//!
//! The session manages the working state during an MCP interaction, including
//! the PGLite runtime, base namespace, and operation history.

use std::path::PathBuf;
use std::time::Instant;

use tokio_postgres::Client;

use crate::db::migrate::Operation;
use crate::db::model::Namespace;
use crate::db::pglite::PgLiteRuntime;
use crate::db::query::{PostgresCatalog, load_namespace};
use crate::db::state::{LocalFileBackend, StateBackend};

use super::error::{McpError, McpResult};

/// A unique session identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(u64);

impl SessionId {
    /// Creates a new session ID.
    #[must_use]
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// An operation that was applied during the session.
#[derive(Debug, Clone)]
pub struct AppliedOperation {
    /// The operation that was applied.
    pub operation: Operation,
    /// SQL that was executed.
    pub sql: String,
    /// Timestamp when applied.
    pub applied_at: Instant,
}

/// Manages the working state during an MCP interaction.
///
/// The session tracks:
/// - The PGLite runtime for executing SQL
/// - The base namespace loaded from state.json
/// - Operations applied during this session
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    /// PGLite runtime for this session.
    runtime: PgLiteRuntime,
    /// PostgreSQL client connection.
    client: Option<Client>,
    /// Base namespace loaded from state (immutable reference point).
    base_namespace: Namespace,
    /// Operations applied in this session.
    operations: Vec<AppliedOperation>,
    /// Session creation time.
    created_at: Instant,
    /// Working directory (where .tern/ lives).
    working_dir: PathBuf,
    /// Schema name being worked on.
    schema_name: String,
}

impl Session {
    /// Creates a new session by loading state from the Tern project.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - Path to the directory containing `.tern/`
    /// * `schema_name` - PostgreSQL schema name to work with (e.g., "public")
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The `.tern/` directory cannot be found
    /// - The state backend cannot be loaded
    /// - PGLite cannot be started
    pub async fn new(working_dir: PathBuf, schema_name: String) -> McpResult<Self> {
        // Load state from the backend
        let backend = LocalFileBackend::new(&working_dir);

        if !backend.is_initialized().await? {
            return Err(McpError::StateError {
                message: format!(
                    ".tern/ directory not found or not initialized in {}",
                    working_dir.display()
                ),
            });
        }

        // Get the current schema state
        let base_namespace = backend.get_current_state().await?;

        // Start PGLite runtime
        let mut runtime = PgLiteRuntime::new()?;
        runtime.start().await?;

        // Get a client connection
        let client = runtime.client().await?;

        // Render the base schema to SQL and execute it in PGLite
        let base_sql = render_namespace_to_sql(&base_namespace);
        if !base_sql.is_empty() {
            client
                .batch_execute(&base_sql)
                .await
                .map_err(|e| McpError::PgLiteError {
                    message: format!("Failed to execute base schema: {}", e),
                })?;
        }

        Ok(Self {
            id: SessionId::new(),
            runtime,
            client: Some(client),
            base_namespace,
            operations: Vec::new(),
            created_at: Instant::now(),
            working_dir,
            schema_name,
        })
    }

    /// Returns the session ID.
    #[must_use]
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Returns the base namespace.
    #[must_use]
    pub fn base_namespace(&self) -> &Namespace {
        &self.base_namespace
    }

    /// Returns the working directory.
    #[must_use]
    pub fn working_dir(&self) -> &PathBuf {
        &self.working_dir
    }

    /// Returns the schema name.
    #[must_use]
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// Returns the number of operations applied.
    #[must_use]
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }

    /// Returns the operations applied in this session.
    #[must_use]
    pub fn operations(&self) -> &[AppliedOperation] {
        &self.operations
    }

    /// Returns the session creation time.
    #[must_use]
    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    /// Gets a PostgreSQL client connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the client is not available.
    pub fn client(&self) -> McpResult<&Client> {
        self.client.as_ref().ok_or_else(|| McpError::Internal {
            message: "Client not available".to_string(),
        })
    }

    /// Executes SQL in the PGLite runtime.
    ///
    /// # Arguments
    ///
    /// * `sql` - The SQL to execute
    ///
    /// # Errors
    ///
    /// Returns an error if execution fails.
    pub async fn execute_sql(&self, sql: &str) -> McpResult<()> {
        let client = self.client()?;
        client
            .batch_execute(sql)
            .await
            .map_err(|e| McpError::PgLiteError {
                message: e.to_string(),
            })
    }

    /// Executes SQL and records the operation.
    ///
    /// # Arguments
    ///
    /// * `sql` - The SQL to execute
    /// * `operation` - The operation being applied
    ///
    /// # Errors
    ///
    /// Returns an error if execution fails.
    pub async fn apply_operation(&mut self, sql: &str, operation: Operation) -> McpResult<()> {
        self.execute_sql(sql).await?;

        self.operations.push(AppliedOperation {
            operation,
            sql: sql.to_string(),
            applied_at: Instant::now(),
        });

        Ok(())
    }

    /// Loads the current namespace from PGLite.
    ///
    /// # Errors
    ///
    /// Returns an error if the namespace cannot be loaded.
    pub async fn current_namespace(&self) -> McpResult<Namespace> {
        let client = self.client()?;
        let catalog = PostgresCatalog::new(client);
        let namespace = load_namespace(&catalog, &self.schema_name).await?;
        Ok(namespace)
    }

    /// Resets the session to the base state.
    ///
    /// This discards all operations and restores PGLite to the base schema.
    ///
    /// # Errors
    ///
    /// Returns an error if the reset fails.
    pub async fn reset(&mut self) -> McpResult<usize> {
        let discarded = self.operations.len();

        // Reset PGLite to clean state
        self.runtime.reset().await?;

        // Get a new client
        self.client = Some(self.runtime.client().await?);

        // Re-execute the base schema
        let base_sql = render_namespace_to_sql(&self.base_namespace);
        if !base_sql.is_empty() {
            self.execute_sql(&base_sql).await?;
        }

        // Clear operations
        self.operations.clear();

        Ok(discarded)
    }

    /// Returns a summary of pending changes.
    #[must_use]
    pub fn pending_changes_summary(&self) -> PendingChangesSummary {
        let mut summary = PendingChangesSummary::default();

        for applied in &self.operations {
            match &applied.operation {
                Operation::CreateTable { table, .. } => {
                    summary.tables_added.push(table.name.as_ref().to_string());
                }
                Operation::DropTable { name, .. } => {
                    summary.tables_dropped.push(name.as_ref().to_string());
                }
                Operation::AddColumn { table, column, .. } => {
                    summary.columns_added.push(format!(
                        "{}.{}",
                        table.as_ref(),
                        column.name.as_ref()
                    ));
                }
                Operation::DropColumn { table, name, .. } => {
                    summary
                        .columns_dropped
                        .push(format!("{}.{}", table.as_ref(), name.as_ref()));
                }
                Operation::CreateIndex { index, .. } => {
                    summary
                        .indexes_created
                        .push(index.name.as_ref().to_string());
                }
                Operation::DropIndex { name, .. } => {
                    summary.indexes_dropped.push(name.as_ref().to_string());
                }
                Operation::CreateEnum { enum_type, .. } => {
                    summary
                        .enums_created
                        .push(enum_type.name.as_ref().to_string());
                }
                Operation::DropEnum { name, .. } => {
                    summary.enums_dropped.push(name.as_ref().to_string());
                }
                _ => {}
            }
        }

        summary
    }
}

/// Summary of pending changes in a session.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingChangesSummary {
    pub tables_added: Vec<String>,
    pub tables_dropped: Vec<String>,
    pub columns_added: Vec<String>,
    pub columns_dropped: Vec<String>,
    pub indexes_created: Vec<String>,
    pub indexes_dropped: Vec<String>,
    pub enums_created: Vec<String>,
    pub enums_dropped: Vec<String>,
}

impl PendingChangesSummary {
    /// Returns true if there are no pending changes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tables_added.is_empty()
            && self.tables_dropped.is_empty()
            && self.columns_added.is_empty()
            && self.columns_dropped.is_empty()
            && self.indexes_created.is_empty()
            && self.indexes_dropped.is_empty()
            && self.enums_created.is_empty()
            && self.enums_dropped.is_empty()
    }
}

/// Renders a namespace to SQL DDL for initializing PGLite.
fn render_namespace_to_sql(namespace: &Namespace) -> String {
    use crate::db::diff::diff_namespaces;
    use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};

    // Create an empty namespace to diff against
    let empty = Namespace::empty(namespace.name.as_ref());

    // Diff from empty to the target namespace
    let diff = diff_namespaces(&empty, namespace);

    // Create migration plan
    let plan = MigrationPlan::from_diff(&diff);

    // Render to SQL
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);

    script.to_sql()
}
