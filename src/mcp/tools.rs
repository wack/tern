//! MCP tools for migration authoring.
//!
//! This module provides the tools that allow MCP clients to create and manage
//! migration sessions, execute SQL, and generate migrations.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(feature = "pglite")]
use crate::db::diff::diff_namespaces;
#[cfg(feature = "pglite")]
use crate::db::migrate::{MigrationPlan, Operation, PostgresRenderer, RenderConfig, Renderer};
#[cfg(feature = "pglite")]
use crate::db::query::{PostgresCatalog, load_namespace};
use crate::db::state::StateHash;
#[cfg(feature = "pglite")]
use crate::db::state::{Migration, StateBackend};
use crate::mcp::error::{McpError, SessionId};
use crate::mcp::protocol::Tool;
use crate::mcp::resources::SchemaResource;
use crate::mcp::session::{SessionManager, SessionSummary};

/// Returns the list of available tools.
pub fn list_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "start_session".to_string(),
            description: Some(
                "Initialize a new session for creating a migration. Returns a session ID that must be used for all subsequent operations.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "description": {
                        "type": "string",
                        "description": "Description for the migration being created (e.g., 'Add user preferences table')"
                    }
                },
                "required": ["description"]
            }),
        },
        Tool {
            name: "execute_sql".to_string(),
            description: Some(
                "Execute raw SQL statement(s) against the session's in-memory database. Use this for schema changes that aren't covered by structured operations, or when you prefer writing SQL directly.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    },
                    "sql": {
                        "type": "string",
                        "description": "SQL statement(s) to execute. Multiple statements can be separated by semicolons."
                    }
                },
                "required": ["sessionId", "sql"]
            }),
        },
        Tool {
            name: "apply_operation".to_string(),
            description: Some(
                "Apply a structured schema modification operation. This is an alternative to raw SQL that provides better validation and error messages.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    },
                    "operation": {
                        "type": "object",
                        "description": "The operation to apply (create_table, drop_table, add_column, etc.)"
                    }
                },
                "required": ["sessionId", "operation"]
            }),
        },
        Tool {
            name: "get_session_schema".to_string(),
            description: Some(
                "Returns the current schema state in the session's database, reflecting all changes made since the session started.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        Tool {
            name: "get_session_diff".to_string(),
            description: Some(
                "Returns a summary of all changes made in the session compared to the base schema.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        Tool {
            name: "generate_migration".to_string(),
            description: Some(
                "Generates a migration from all changes made in the session and saves it to the .tern/migrations directory. This ends the session.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    },
                    "description": {
                        "type": "string",
                        "description": "Override the migration description (uses session description if not provided)"
                    },
                    "force": {
                        "type": "boolean",
                        "default": false,
                        "description": "Generate migration even if it contains breaking changes"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        Tool {
            name: "cancel_session".to_string(),
            description: Some(
                "Cancels an active session and discards all changes. No migration is created.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sessionId": {
                        "type": "string",
                        "description": "Session ID from start_session"
                    }
                },
                "required": ["sessionId"]
            }),
        },
        Tool {
            name: "list_sessions".to_string(),
            description: Some(
                "Returns a list of all currently active sessions. Useful for recovery if a session ID is lost.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

// =============================================================================
// Tool Input Types
// =============================================================================

/// Input for start_session tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionInput {
    /// Description for the migration being created.
    pub description: String,
}

/// Input for execute_sql tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteSqlInput {
    /// Session ID.
    pub session_id: String,
    /// SQL to execute.
    pub sql: String,
}

/// Input for apply_operation tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ApplyOperationInput {
    /// Session ID.
    pub session_id: String,
    /// Operation to apply.
    pub operation: Value,
}

/// Input for get_session_schema tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionSchemaInput {
    /// Session ID.
    pub session_id: String,
}

/// Input for get_session_diff tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionDiffInput {
    /// Session ID.
    pub session_id: String,
}

/// Input for generate_migration tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct GenerateMigrationInput {
    /// Session ID.
    pub session_id: String,
    /// Optional description override.
    pub description: Option<String>,
    /// Force generation even with breaking changes.
    #[serde(default)]
    pub force: bool,
}

/// Input for cancel_session tool.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelSessionInput {
    /// Session ID.
    pub session_id: String,
}

// =============================================================================
// Tool Output Types
// =============================================================================

/// Output for start_session tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionOutput {
    /// Session ID.
    pub session_id: String,
    /// Migration description.
    pub description: String,
    /// Base state hash.
    pub base_state_hash: String,
    /// When the session was started.
    pub started_at: String,
    /// Human-readable message.
    pub message: String,
}

/// Output for execute_sql tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteSqlOutput {
    /// Whether execution succeeded.
    pub success: bool,
    /// Number of rows affected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    /// Human-readable message.
    pub message: String,
    /// Warnings, if any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Error details, if failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SqlError>,
}

/// SQL error details.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlError {
    /// Error message.
    pub message: String,
    /// PostgreSQL error code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Detailed error information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Hint for fixing the error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Position in the SQL where the error occurred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// Output for apply_operation tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOperationOutput {
    /// Whether the operation succeeded.
    pub success: bool,
    /// SQL that was executed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_executed: Option<String>,
    /// Human-readable message.
    pub message: String,
}

/// Output for get_session_diff tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDiffOutput {
    /// Whether there are any changes.
    pub has_changes: bool,
    /// Summary counts.
    pub summary: DiffSummary,
    /// Detailed changes.
    pub details: DiffDetails,
    /// Breaking changes detected.
    pub breaking_changes: Vec<String>,
    /// Whether there are breaking changes.
    pub has_breaking_changes: bool,
}

/// Summary of diff counts.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    /// Number of tables added.
    pub tables_added: usize,
    /// Number of tables removed.
    pub tables_removed: usize,
    /// Number of tables modified.
    pub tables_modified: usize,
    /// Number of columns added.
    pub columns_added: usize,
    /// Number of columns removed.
    pub columns_removed: usize,
    /// Number of columns modified.
    pub columns_modified: usize,
    /// Number of indexes added.
    pub indexes_added: usize,
    /// Number of indexes removed.
    pub indexes_removed: usize,
    /// Number of constraints added.
    pub constraints_added: usize,
    /// Number of constraints removed.
    pub constraints_removed: usize,
    /// Number of enums added.
    pub enums_added: usize,
    /// Number of enums removed.
    pub enums_removed: usize,
}

/// Detailed diff information.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffDetails {
    /// Names of tables added.
    pub tables_added: Vec<String>,
    /// Names of tables removed.
    pub tables_removed: Vec<String>,
    /// Details of modified tables.
    pub tables_modified: Vec<ModifiedTableDetail>,
    /// Indexes added.
    pub indexes_added: Vec<IndexDetail>,
    /// Foreign keys added.
    pub foreign_keys_added: Vec<ForeignKeyDetail>,
}

/// Detail of a modified table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifiedTableDetail {
    /// Table name.
    pub table: String,
    /// Columns added.
    pub columns_added: Vec<String>,
    /// Columns removed.
    pub columns_removed: Vec<String>,
    /// Columns modified.
    pub columns_modified: Vec<String>,
}

/// Detail of an index.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDetail {
    /// Index name.
    pub name: String,
    /// Table name.
    pub table: String,
    /// Column names.
    pub columns: Vec<String>,
}

/// Detail of a foreign key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDetail {
    /// Constraint name.
    pub name: String,
    /// Referencing table.
    pub table: String,
    /// Referencing columns.
    pub columns: Vec<String>,
    /// Referenced table.
    pub references_table: String,
    /// Referenced columns.
    pub references_columns: Vec<String>,
}

/// Output for generate_migration tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateMigrationOutput {
    /// Whether generation succeeded.
    pub success: bool,
    /// Migration ID.
    pub migration_id: String,
    /// Sequence number.
    pub sequence_number: usize,
    /// Migration description.
    pub description: String,
    /// File path where the migration was saved.
    pub file_path: String,
    /// Number of operations in the migration.
    pub operation_count: usize,
    /// Whether the migration has breaking changes.
    pub has_breaking_changes: bool,
    /// Breaking changes detected.
    pub breaking_changes: Vec<String>,
    /// Whether the migration is reversible.
    pub is_reversible: bool,
    /// Whether the session has ended.
    pub session_ended: bool,
    /// Human-readable message.
    pub message: String,
}

/// Output for cancel_session tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelSessionOutput {
    /// Whether cancellation succeeded.
    pub success: bool,
    /// Session ID that was cancelled.
    pub session_id: String,
    /// Number of operations discarded.
    pub changes_discarded: usize,
    /// Human-readable message.
    pub message: String,
}

/// Output for list_sessions tool.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsOutput {
    /// List of active sessions.
    pub sessions: Vec<SessionSummary>,
    /// Total count.
    pub total_count: usize,
}

// =============================================================================
// Tool Handlers
// =============================================================================

/// Handles the start_session tool.
pub async fn handle_start_session(
    input: StartSessionInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = session_manager
        .start_session(input.description.clone())
        .await?;

    let session = session_manager
        .get_session(&session_id)
        .ok_or_else(|| McpError::InternalError("session not found after creation".into()))?;

    let base_state = session_manager.base_state();
    let table_count = base_state.tables.len();
    let view_count = base_state.views.len();
    let sequence_count = base_state.sequences.len();
    let enum_count = base_state.enums.len();

    let output = StartSessionOutput {
        session_id: session_id.to_string(),
        description: input.description,
        base_state_hash: StateHash::from_namespace(base_state).to_hex(),
        started_at: session.started_at.to_string(),
        message: format!(
            "Session started. Current schema has {} tables, {} views, {} sequences, {} enums.",
            table_count, view_count, sequence_count, enum_count
        ),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the execute_sql tool.
#[cfg(feature = "pglite")]
pub async fn handle_execute_sql(
    input: ExecuteSqlInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Ensure the session's PGLite database is initialized
    session_manager
        .ensure_session_initialized(&session_id)
        .await?;

    let session = session_manager
        .get_session_mut(&session_id)
        .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;

    // Get the PGLite client for this session
    let client = session.get_client().await?;

    // Execute the SQL
    match client.batch_execute(&input.sql).await {
        Ok(()) => {
            // Record the SQL in history
            session.record_sql(input.sql.clone());

            let output = ExecuteSqlOutput {
                success: true,
                rows_affected: Some(0),
                message: "SQL executed successfully".to_string(),
                warnings: vec![],
                error: None,
            };

            serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
        }
        Err(e) => {
            let db_error = e.as_db_error();
            let output = ExecuteSqlOutput {
                success: false,
                rows_affected: None,
                message: e.to_string(),
                warnings: vec![],
                error: Some(SqlError {
                    message: e.to_string(),
                    code: db_error.map(|e| e.code().code().to_string()),
                    detail: db_error.and_then(|e| e.detail().map(|s| s.to_string())),
                    hint: db_error.and_then(|e| e.hint().map(|s| s.to_string())),
                    position: db_error.and_then(|e| {
                        e.position().map(|p| match p {
                            tokio_postgres::error::ErrorPosition::Original(pos)
                            | tokio_postgres::error::ErrorPosition::Internal {
                                position: pos,
                                ..
                            } => *pos as i32,
                        })
                    }),
                }),
            };

            serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
        }
    }
}

/// Handles the execute_sql tool (non-pglite version).
#[cfg(not(feature = "pglite"))]
pub async fn handle_execute_sql(
    input: ExecuteSqlInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);
    let session = session_manager
        .get_session_mut(&session_id)
        .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;

    // Record the SQL in history
    session.record_sql(input.sql.clone());

    let output = ExecuteSqlOutput {
        success: true,
        rows_affected: Some(0),
        message: "SQL recorded (PGLite not enabled)".to_string(),
        warnings: vec![],
        error: None,
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the apply_operation tool.
#[cfg(feature = "pglite")]
pub async fn handle_apply_operation(
    input: ApplyOperationInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Parse the operation from JSON
    let operation: Operation = serde_json::from_value(input.operation.clone())
        .map_err(|e| McpError::InvalidParams(format!("invalid operation: {e}")))?;

    // Ensure the session's PGLite database is initialized
    session_manager
        .ensure_session_initialized(&session_id)
        .await?;

    let session = session_manager
        .get_session_mut(&session_id)
        .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;

    // Render the operation to SQL
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let rendered = renderer.render(&operation);
    let sql = rendered.forward.join(";\n");

    // Get the PGLite client for this session
    let client = session.get_client().await?;

    // Execute the SQL
    match client.batch_execute(&sql).await {
        Ok(()) => {
            // Record the operation in history
            session.record_operation(operation, sql.clone());

            let output = ApplyOperationOutput {
                success: true,
                sql_executed: Some(sql),
                message: "Operation applied successfully".to_string(),
            };

            serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
        }
        Err(e) => {
            let output = ApplyOperationOutput {
                success: false,
                sql_executed: Some(sql),
                message: format!("Operation failed: {e}"),
            };

            serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
        }
    }
}

/// Handles the apply_operation tool (non-pglite version).
#[cfg(not(feature = "pglite"))]
pub async fn handle_apply_operation(
    input: ApplyOperationInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);
    let _session = session_manager
        .get_session_mut(&session_id)
        .ok_or(McpError::SessionNotFound(session_id))?;

    let output = ApplyOperationOutput {
        success: false,
        sql_executed: None,
        message: "apply_operation requires PGLite feature to be enabled".to_string(),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the get_session_schema tool.
#[cfg(feature = "pglite")]
pub async fn handle_get_session_schema(
    input: GetSessionSchemaInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Ensure the session's PGLite database is initialized
    session_manager
        .ensure_session_initialized(&session_id)
        .await?;

    let session = session_manager
        .get_session(&session_id)
        .ok_or(McpError::SessionNotFound(session_id))?;

    // Get the current schema from the session's PGLite database
    let client = session.get_client().await?;
    let catalog = PostgresCatalog::new(&client);
    let namespace = load_namespace(&catalog, "public")
        .await
        .map_err(|e| McpError::InternalError(format!("failed to load schema: {e}")))?;

    let output = SchemaResource::from(&namespace);

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the get_session_schema tool (non-pglite version).
#[cfg(not(feature = "pglite"))]
pub async fn handle_get_session_schema(
    input: GetSessionSchemaInput,
    session_manager: &SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);
    let session = session_manager
        .get_session(&session_id)
        .ok_or(McpError::SessionNotFound(session_id))?;

    // Return the base state when PGLite is not available
    let output = SchemaResource::from(&session.base_state);

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the get_session_diff tool.
#[cfg(feature = "pglite")]
pub async fn handle_get_session_diff(
    input: GetSessionDiffInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Ensure the session's PGLite database is initialized
    session_manager
        .ensure_session_initialized(&session_id)
        .await?;

    let session = session_manager
        .get_session(&session_id)
        .ok_or(McpError::SessionNotFound(session_id))?;

    // Get the current schema from the session's PGLite database
    let client = session.get_client().await?;
    let catalog = PostgresCatalog::new(&client);
    let current_state = load_namespace(&catalog, "public")
        .await
        .map_err(|e| McpError::InternalError(format!("failed to load schema: {e}")))?;

    // Diff against base state
    let diff = diff_namespaces(&session.base_state, &current_state);

    // Build the output
    let mut summary = DiffSummary::default();
    let mut details = DiffDetails::default();
    let mut breaking_changes = Vec::new();

    // Process added tables
    for table in &diff.tables.added {
        summary.tables_added += 1;
        details.tables_added.push(table.name.as_ref().to_string());
    }

    // Process removed tables (we only have keys)
    for table_name in &diff.tables.removed {
        summary.tables_removed += 1;
        details.tables_removed.push(table_name.as_ref().to_string());
        breaking_changes.push(format!("Table '{}' removed", table_name.as_ref()));
    }

    // Process modified tables
    for modified in &diff.tables.modified {
        summary.tables_modified += 1;

        let mut modified_detail = ModifiedTableDetail {
            table: modified.name.as_ref().to_string(),
            columns_added: vec![],
            columns_removed: vec![],
            columns_modified: vec![],
        };

        // Check for column changes
        for col in &modified.columns.added {
            summary.columns_added += 1;
            modified_detail
                .columns_added
                .push(col.name.as_ref().to_string());
        }

        for col_name in &modified.columns.removed {
            summary.columns_removed += 1;
            modified_detail
                .columns_removed
                .push(col_name.as_ref().to_string());
            breaking_changes.push(format!(
                "Column '{}.{}' removed",
                modified.name.as_ref(),
                col_name.as_ref()
            ));
        }

        for col_mod in &modified.columns.modified {
            summary.columns_modified += 1;
            modified_detail
                .columns_modified
                .push(col_mod.name.as_ref().to_string());
        }

        // Check for constraint changes
        summary.constraints_added += modified.constraints.added.len();
        summary.constraints_removed += modified.constraints.removed.len();

        // Check for index changes
        summary.indexes_added += modified.indexes.added.len();
        summary.indexes_removed += modified.indexes.removed.len();

        details.tables_modified.push(modified_detail);
    }

    // Process added enums
    summary.enums_added = diff.enums.added.len();

    // Process removed enums
    for enum_name in &diff.enums.removed {
        summary.enums_removed += 1;
        breaking_changes.push(format!("Enum '{}' removed", enum_name.as_ref()));
    }

    let has_changes = summary.tables_added > 0
        || summary.tables_removed > 0
        || summary.tables_modified > 0
        || summary.enums_added > 0
        || summary.enums_removed > 0;

    let output = SessionDiffOutput {
        has_changes,
        summary,
        details,
        breaking_changes: breaking_changes.clone(),
        has_breaking_changes: !breaking_changes.is_empty(),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the get_session_diff tool (non-pglite version).
#[cfg(not(feature = "pglite"))]
pub async fn handle_get_session_diff(
    input: GetSessionDiffInput,
    session_manager: &SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);
    let session = session_manager
        .get_session(&session_id)
        .ok_or(McpError::SessionNotFound(session_id))?;

    // Return an empty diff when PGLite is not available
    let output = SessionDiffOutput {
        has_changes: session.has_changes(),
        summary: DiffSummary::default(),
        details: DiffDetails::default(),
        breaking_changes: vec![],
        has_breaking_changes: false,
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the generate_migration tool.
#[cfg(feature = "pglite")]
pub async fn handle_generate_migration(
    input: GenerateMigrationInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Ensure the session's PGLite database is initialized
    session_manager
        .ensure_session_initialized(&session_id)
        .await?;

    // Get the session's base state and description
    let (base_state, description, parent_hash) = {
        let session = session_manager
            .get_session(&session_id)
            .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;

        if !session.has_changes() && !input.force {
            return Err(McpError::NoChanges);
        }

        let desc = input
            .description
            .clone()
            .unwrap_or_else(|| session.description.clone());
        let parent = StateHash::from_namespace(&session.base_state);
        (session.base_state.clone(), desc, parent)
    };

    // Get the current schema from the session's PGLite database
    let current_state = {
        let session = session_manager
            .get_session(&session_id)
            .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;
        let client = session.get_client().await?;
        let catalog = PostgresCatalog::new(&client);
        load_namespace(&catalog, "public")
            .await
            .map_err(|e| McpError::InternalError(format!("failed to load schema: {e}")))?
    };

    // Diff against base state
    let diff = diff_namespaces(&base_state, &current_state);

    // Check for breaking changes
    let mut breaking_changes = Vec::new();
    for table_name in &diff.tables.removed {
        breaking_changes.push(format!("Table '{}' removed", table_name.as_ref()));
    }
    for modified in &diff.tables.modified {
        for col_name in &modified.columns.removed {
            breaking_changes.push(format!(
                "Column '{}.{}' removed",
                modified.name.as_ref(),
                col_name.as_ref()
            ));
        }
    }
    for enum_name in &diff.enums.removed {
        breaking_changes.push(format!("Enum '{}' removed", enum_name.as_ref()));
    }

    if !breaking_changes.is_empty() && !input.force {
        return Err(McpError::BreakingChangesDetected { breaking_changes });
    }

    // Convert diff to migration plan
    let plan = MigrationPlan::from_diff(&diff);

    if plan.is_empty() && !input.force {
        return Err(McpError::NoChanges);
    }

    // Calculate resulting state hash
    let resulting_hash = StateHash::from_namespace(&current_state);

    // Create the migration
    let migration = Migration::new(
        &description,
        plan.operations.clone(),
        vec![], // down_operations would require inverse operation generation
        parent_hash,
        resulting_hash,
        vec![], // breaking_changes as BreakingChange structs (simplified for now)
    );

    // Save the migration
    let backend = session_manager.backend();
    let migration_index = backend
        .get_migration_index()
        .await
        .map_err(|e| McpError::InternalError(format!("failed to get migration index: {e}")))?;
    let sequence_number = migration_index.len() + 1;

    backend
        .save_migration(&migration)
        .await
        .map_err(|e| McpError::SaveFailed(e.to_string()))?;

    // Save the current state
    backend
        .save_current_state(&current_state)
        .await
        .map_err(|e| McpError::SaveFailed(format!("failed to save current state: {e}")))?;

    // Get the migration file path
    let migrations_dir = backend.root().join("migrations");
    let file_path = migrations_dir
        .join(format!("{:05}.json", sequence_number))
        .to_string_lossy()
        .to_string();

    // Remove the session
    let _ = session_manager.cancel_session(&session_id).await;

    let output = GenerateMigrationOutput {
        success: true,
        migration_id: migration.id.to_hex(),
        sequence_number,
        description,
        file_path,
        operation_count: plan.operations.len(),
        has_breaking_changes: !breaking_changes.is_empty(),
        breaking_changes,
        is_reversible: false, // Would need inverse operation generation
        session_ended: true,
        message: format!(
            "Migration {} created with {} operations",
            migration.id.to_hex(),
            plan.operations.len()
        ),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the generate_migration tool (non-pglite version).
#[cfg(not(feature = "pglite"))]
pub async fn handle_generate_migration(
    input: GenerateMigrationInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id);

    // Check session exists
    {
        let session = session_manager
            .get_session(&session_id)
            .ok_or_else(|| McpError::SessionNotFound(session_id.clone()))?;

        if !session.has_changes() {
            return Err(McpError::NoChanges);
        }
    }

    let output = GenerateMigrationOutput {
        success: false,
        migration_id: "".to_string(),
        sequence_number: 0,
        description: input.description.unwrap_or_default(),
        file_path: "".to_string(),
        operation_count: 0,
        has_breaking_changes: false,
        breaking_changes: vec![],
        is_reversible: false,
        session_ended: false,
        message: "generate_migration requires PGLite feature to be enabled".to_string(),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the cancel_session tool.
pub async fn handle_cancel_session(
    input: CancelSessionInput,
    session_manager: &mut SessionManager,
) -> Result<Value, McpError> {
    let session_id = SessionId::new(input.session_id.clone());
    let discarded = session_manager.cancel_session(&session_id).await?;

    let output = CancelSessionOutput {
        success: true,
        session_id: input.session_id,
        changes_discarded: discarded,
        message: format!(
            "Session cancelled. {} operations discarded, no migration created.",
            discarded
        ),
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

/// Handles the list_sessions tool.
pub async fn handle_list_sessions(session_manager: &SessionManager) -> Result<Value, McpError> {
    let sessions = session_manager.list_sessions();
    let total_count = sessions.len();

    let output = ListSessionsOutput {
        sessions,
        total_count,
    };

    serde_json::to_value(output).map_err(|e| McpError::InternalError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_tools_returns_expected() {
        let tools = list_tools();

        assert!(tools.iter().any(|t| t.name == "start_session"));
        assert!(tools.iter().any(|t| t.name == "execute_sql"));
        assert!(tools.iter().any(|t| t.name == "apply_operation"));
        assert!(tools.iter().any(|t| t.name == "get_session_schema"));
        assert!(tools.iter().any(|t| t.name == "get_session_diff"));
        assert!(tools.iter().any(|t| t.name == "generate_migration"));
        assert!(tools.iter().any(|t| t.name == "cancel_session"));
        assert!(tools.iter().any(|t| t.name == "list_sessions"));
    }
}
