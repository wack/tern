//! MCP server implementation for Tern.
//!
//! This module provides the main MCP service that handles tool calls
//! and manages session state.

// The trait requires this specific signature pattern, not async fn
#![allow(clippy::manual_async_fn)]

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::ErrorData as McpRpcError;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParam, ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value, json};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use tokio::sync::RwLock;

use super::error::{McpError, McpResult};
use super::session::Session;
use super::tools;

/// The main MCP service for Tern.
///
/// This service implements the MCP protocol and exposes Tern's schema
/// modification capabilities as tools.
#[derive(Clone)]
pub struct TernMcpService {
    /// The active session, if any.
    pub session: Arc<RwLock<Option<Session>>>,
    /// Default working directory.
    pub default_working_dir: PathBuf,
}

impl TernMcpService {
    /// Creates a new MCP service.
    ///
    /// # Arguments
    ///
    /// * `working_dir` - Default working directory for session initialization
    #[must_use]
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            session: Arc::new(RwLock::new(None)),
            default_working_dir: working_dir,
        }
    }

    /// Gets the current session, returning an error if none exists.
    pub async fn require_session(
        &self,
    ) -> McpResult<tokio::sync::RwLockReadGuard<'_, Option<Session>>> {
        let guard = self.session.read().await;
        if guard.is_none() {
            return Err(McpError::NoSession);
        }
        Ok(guard)
    }

    /// Gets the current session mutably, returning an error if none exists.
    pub async fn require_session_mut(
        &self,
    ) -> McpResult<tokio::sync::RwLockWriteGuard<'_, Option<Session>>> {
        let guard = self.session.write().await;
        if guard.is_none() {
            return Err(McpError::NoSession);
        }
        Ok(guard)
    }

    /// Dispatches a tool call to the appropriate handler.
    async fn dispatch_tool(
        &self,
        name: &str,
        arguments: Option<serde_json::Map<String, Value>>,
    ) -> Result<Value, McpError> {
        let args = arguments.map(Value::Object).unwrap_or(json!({}));

        match name {
            // Session tools
            "session_start" => {
                let input: SessionStartInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::session::session_start(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "session_status" => {
                let result = tools::session::session_status(self).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "session_reset" => {
                let input: SessionResetInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::session::session_reset(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Inspection tools
            "list_tables" => {
                let input: ListTablesInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::inspect::list_tables(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "describe_table" => {
                let input: DescribeTableInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::inspect::describe_table(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "list_enums" => {
                let result = tools::inspect::list_enums(self).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "list_indexes" => {
                let result = tools::inspect::list_indexes(self).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Table tools
            "create_table" => {
                let input: CreateTableInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::tables::create_table(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "drop_table" => {
                let input: DropTableInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::tables::drop_table(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "rename_table" => {
                let input: RenameTableInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::tables::rename_table(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Column tools
            "add_column" => {
                let input: AddColumnInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::add_column(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "drop_column" => {
                let input: DropColumnInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::drop_column(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "rename_column" => {
                let input: RenameColumnInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::rename_column(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "alter_column_type" => {
                let input: AlterColumnTypeInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::alter_column_type(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "alter_column_default" => {
                let input: AlterColumnDefaultInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::alter_column_default(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "alter_column_nullable" => {
                let input: AlterColumnNullableInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::columns::alter_column_nullable(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Constraint tools
            "add_primary_key" => {
                let input: AddPrimaryKeyInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::constraints::add_primary_key(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "add_foreign_key" => {
                let input: AddForeignKeyInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::constraints::add_foreign_key(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "add_unique_constraint" => {
                let input: AddUniqueConstraintInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::constraints::add_unique_constraint(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "add_check_constraint" => {
                let input: AddCheckConstraintInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::constraints::add_check_constraint(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "drop_constraint" => {
                let input: DropConstraintInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::constraints::drop_constraint(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Index tools
            "create_index" => {
                let input: CreateIndexInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::indexes::create_index(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "drop_index" => {
                let input: DropIndexInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::indexes::drop_index(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Enum tools
            "create_enum" => {
                let input: CreateEnumInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::enums::create_enum(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "add_enum_value" => {
                let input: AddEnumValueInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::enums::add_enum_value(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "drop_enum" => {
                let input: DropEnumInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::enums::drop_enum(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            // Migration tools
            "get_migration_preview" => {
                let input: GetMigrationPreviewInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::migrate::get_migration_preview(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "get_breaking_changes" => {
                let result = tools::migrate::get_breaking_changes(self).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }
            "commit_migration" => {
                let input: CommitMigrationInput =
                    serde_json::from_value(args).map_err(|e| McpError::InvalidInput {
                        message: e.to_string(),
                    })?;
                let result = tools::migrate::commit_migration(self, input).await?;
                serde_json::to_value(&result).map_err(|e| McpError::Internal {
                    message: e.to_string(),
                })
            }

            _ => Err(McpError::InvalidInput {
                message: format!("Unknown tool: {}", name),
            }),
        }
    }

    /// Returns the list of available tools.
    fn tool_definitions() -> Vec<Tool> {
        vec![
            // Session tools
            tool_def::<SessionStartInput>(
                "session_start",
                "Initialize a new session, loading from .tern/",
            ),
            tool_def_no_input("session_status", "Get current session state"),
            tool_def::<SessionResetInput>("session_reset", "Reset session to base state"),
            // Inspection tools
            tool_def::<ListTablesInput>("list_tables", "List all tables in current schema"),
            tool_def::<DescribeTableInput>("describe_table", "Get detailed table definition"),
            tool_def_no_input("list_enums", "List all enum types"),
            tool_def_no_input("list_indexes", "List all indexes"),
            // Table tools
            tool_def::<CreateTableInput>("create_table", "Create a new table with columns"),
            tool_def::<DropTableInput>("drop_table", "Drop an existing table"),
            tool_def::<RenameTableInput>("rename_table", "Rename a table"),
            // Column tools
            tool_def::<AddColumnInput>("add_column", "Add a column to a table"),
            tool_def::<DropColumnInput>("drop_column", "Remove a column from a table"),
            tool_def::<RenameColumnInput>("rename_column", "Rename a column"),
            tool_def::<AlterColumnTypeInput>("alter_column_type", "Change a column's data type"),
            tool_def::<AlterColumnDefaultInput>(
                "alter_column_default",
                "Set or remove default value",
            ),
            tool_def::<AlterColumnNullableInput>(
                "alter_column_nullable",
                "Change nullability constraint",
            ),
            // Constraint tools
            tool_def::<AddPrimaryKeyInput>("add_primary_key", "Add primary key constraint"),
            tool_def::<AddForeignKeyInput>("add_foreign_key", "Add foreign key reference"),
            tool_def::<AddUniqueConstraintInput>("add_unique_constraint", "Add unique constraint"),
            tool_def::<AddCheckConstraintInput>("add_check_constraint", "Add check constraint"),
            tool_def::<DropConstraintInput>("drop_constraint", "Remove a constraint"),
            // Index tools
            tool_def::<CreateIndexInput>("create_index", "Create an index"),
            tool_def::<DropIndexInput>("drop_index", "Drop an index"),
            // Enum tools
            tool_def::<CreateEnumInput>("create_enum", "Create an enum type"),
            tool_def::<AddEnumValueInput>("add_enum_value", "Add a value to existing enum"),
            tool_def::<DropEnumInput>("drop_enum", "Drop an enum type"),
            // Migration tools
            tool_def::<GetMigrationPreviewInput>(
                "get_migration_preview",
                "Preview migration SQL without committing",
            ),
            tool_def_no_input(
                "get_breaking_changes",
                "List any destructive/breaking changes",
            ),
            tool_def::<CommitMigrationInput>(
                "commit_migration",
                "Save migration to .tern/migrations/",
            ),
        ]
    }
}

/// Helper to create a tool definition with JSON schema.
fn tool_def<T: JsonSchema>(name: &str, description: &str) -> Tool {
    let schema = rmcp::schemars::schema_for!(T);
    let input_schema: JsonObject =
        serde_json::from_value(serde_json::to_value(&schema).unwrap_or_default())
            .unwrap_or_default();
    Tool {
        name: Cow::Owned(name.to_string()),
        title: None,
        description: Some(Cow::Owned(description.to_string())),
        input_schema: Arc::new(input_schema),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
    }
}

/// Helper to create a tool definition with no required input.
fn tool_def_no_input(name: &str, description: &str) -> Tool {
    let empty_schema: JsonObject =
        serde_json::from_str(r#"{"type": "object", "properties": {}}"#).unwrap_or_default();
    Tool {
        name: Cow::Owned(name.to_string()),
        title: None,
        description: Some(Cow::Owned(description.to_string())),
        input_schema: Arc::new(empty_schema),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
    }
}

// =============================================================================
// MCP Server Handler Implementation
// =============================================================================

impl ServerHandler for TernMcpService {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation::from_build_env(),
            instructions: Some(
                "Tern MCP server for database schema migrations. \
                 Start with session_start to initialize, then use schema modification tools."
                    .to_string(),
            ),
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpRpcError>> + Send + '_ {
        async move {
            Ok(ListToolsResult {
                tools: Self::tool_definitions(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpRpcError>> + Send + '_ {
        async move {
            match self.dispatch_tool(&request.name, request.arguments).await {
                Ok(value) => {
                    let content =
                        Content::text(serde_json::to_string_pretty(&value).unwrap_or_default());
                    Ok(CallToolResult::success(vec![content]))
                }
                Err(e) => {
                    let error_json = json!({
                        "error": e.code(),
                        "message": e.to_string(),
                        "suggestion": e.suggestion()
                    });
                    let content = Content::text(
                        serde_json::to_string_pretty(&error_json).unwrap_or_default(),
                    );
                    Ok(CallToolResult::error(vec![content]))
                }
            }
        }
    }
}

// =============================================================================
// Input/Output Types
// =============================================================================

/// Input for session_start tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartInput {
    /// Path to directory containing .tern/. Defaults to current directory.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// PostgreSQL schema name to work with. Defaults to 'public'.
    #[serde(default)]
    pub schema_name: Option<String>,
}

/// Output for session_start tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartOutput {
    pub session_id: String,
    pub base_state: BaseStateInfo,
    pub message: String,
}

/// Information about the base state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseStateInfo {
    pub tables: Vec<String>,
    pub enums: Vec<String>,
    pub sequences: Vec<String>,
}

/// Input for session_status tool.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStatusInput {}

/// Output for session_status tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatusOutput {
    pub session_id: String,
    pub operations_applied: usize,
    pub pending_changes: super::session::PendingChangesSummary,
    pub has_breaking_changes: bool,
}

/// Input for session_reset tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionResetInput {
    /// Must be true to confirm reset.
    pub confirm: bool,
}

/// Output for session_reset tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResetOutput {
    pub message: String,
    pub operations_discarded: usize,
}

/// Input for list_tables tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListTablesInput {
    /// Include column names in output. Default false.
    #[serde(default)]
    pub include_columns: bool,
}

/// Output for list_tables tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTablesOutput {
    pub tables: Vec<TableInfo>,
}

/// Basic table information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<String>>,
}

/// Input for describe_table tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DescribeTableInput {
    /// Name of the table to describe.
    pub table_name: String,
}

/// Output for describe_table tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescribeTableOutput {
    pub name: String,
    pub columns: Vec<ColumnInfo>,
    pub constraints: Vec<ConstraintInfo>,
    pub indexes: Vec<IndexInfo>,
    pub foreign_keys_incoming: Vec<ForeignKeyInfo>,
}

/// Column information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnInfo {
    pub name: String,
    pub r#type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub is_primary_key: bool,
}

/// Constraint information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintInfo {
    pub name: String,
    pub r#type: String,
    pub columns: Vec<String>,
}

/// Index information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub is_primary: bool,
}

/// Foreign key information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyInfo {
    pub from_table: String,
    pub from_columns: Vec<String>,
    pub to_columns: Vec<String>,
    pub on_delete: String,
}

/// Input for list_enums tool.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListEnumsInput {}

/// Output for list_enums tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListEnumsOutput {
    pub enums: Vec<EnumInfo>,
}

/// Enum information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumInfo {
    pub name: String,
    pub values: Vec<String>,
}

/// Input for list_indexes tool.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListIndexesInput {}

/// Output for list_indexes tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListIndexesOutput {
    pub indexes: Vec<IndexDetailInfo>,
}

/// Detailed index information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDetailInfo {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub is_unique: bool,
    pub method: String,
}

/// Column definition for create_table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ColumnDef {
    /// Column name.
    pub name: String,
    /// PostgreSQL data type.
    pub r#type: String,
    /// Whether the column allows NULL. Defaults to true.
    #[serde(default = "default_true")]
    pub nullable: bool,
    /// Default value expression.
    #[serde(default)]
    pub default: Option<String>,
    /// Whether this column is a primary key.
    #[serde(default)]
    pub primary_key: bool,
}

fn default_true() -> bool {
    true
}

/// Input for create_table tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateTableInput {
    /// Name of the table to create.
    pub name: String,
    /// List of columns to create.
    pub columns: Vec<ColumnDef>,
    /// Column names for composite primary key (alternative to per-column).
    #[serde(default)]
    pub primary_key: Option<Vec<String>>,
}

/// Output for create_table tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTableOutput {
    pub success: bool,
    pub table_name: String,
    pub sql_executed: String,
    pub message: String,
}

/// Input for drop_table tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DropTableInput {
    /// Name of the table to drop.
    pub name: String,
    /// Also drop dependent objects (foreign keys, views).
    #[serde(default)]
    pub cascade: bool,
}

/// Output for drop_table tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropTableOutput {
    pub success: bool,
    pub table_name: String,
    pub sql_executed: String,
    pub warning: String,
}

/// Input for rename_table tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenameTableInput {
    /// Current table name.
    pub from: String,
    /// New table name.
    pub to: String,
}

/// Output for rename_table tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameTableOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for add_column tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddColumnInput {
    /// Name of the table.
    pub table: String,
    /// Column definition.
    pub column: ColumnDef,
}

/// Output for add_column tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddColumnOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for drop_column tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DropColumnInput {
    /// Name of the table.
    pub table: String,
    /// Name of the column to drop.
    pub column: String,
}

/// Output for drop_column tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropColumnOutput {
    pub success: bool,
    pub sql_executed: String,
    pub warning: String,
}

/// Input for rename_column tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RenameColumnInput {
    /// Name of the table.
    pub table: String,
    /// Current column name.
    pub from: String,
    /// New column name.
    pub to: String,
}

/// Output for rename_column tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameColumnOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for alter_column_type tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlterColumnTypeInput {
    /// Name of the table.
    pub table: String,
    /// Name of the column.
    pub column: String,
    /// New data type.
    pub new_type: String,
    /// USING expression for type conversion.
    #[serde(default)]
    pub using: Option<String>,
}

/// Output for alter_column_type tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterColumnTypeOutput {
    pub success: bool,
    pub sql_executed: String,
    pub warning: Option<String>,
}

/// Input for alter_column_default tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlterColumnDefaultInput {
    /// Name of the table.
    pub table: String,
    /// Name of the column.
    pub column: String,
    /// New default value expression, or null to drop default.
    pub default: Option<String>,
}

/// Output for alter_column_default tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterColumnDefaultOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for alter_column_nullable tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlterColumnNullableInput {
    /// Name of the table.
    pub table: String,
    /// Name of the column.
    pub column: String,
    /// Whether the column should be nullable.
    pub nullable: bool,
}

/// Output for alter_column_nullable tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlterColumnNullableOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for add_primary_key tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddPrimaryKeyInput {
    /// Name of the table.
    pub table: String,
    /// Column names for the primary key.
    pub columns: Vec<String>,
    /// Optional constraint name.
    #[serde(default)]
    pub constraint_name: Option<String>,
}

/// Output for add_primary_key tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddPrimaryKeyOutput {
    pub success: bool,
    pub constraint_name: String,
    pub sql_executed: String,
    pub message: String,
}

/// Input for add_foreign_key tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddForeignKeyInput {
    /// Table containing the foreign key column(s).
    pub table: String,
    /// Column(s) in the source table.
    pub columns: Vec<String>,
    /// Table being referenced.
    pub references_table: String,
    /// Column(s) in the referenced table.
    pub references_columns: Vec<String>,
    /// ON DELETE action.
    #[serde(default)]
    pub on_delete: Option<String>,
    /// ON UPDATE action.
    #[serde(default)]
    pub on_update: Option<String>,
    /// Optional constraint name.
    #[serde(default)]
    pub constraint_name: Option<String>,
}

/// Output for add_foreign_key tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddForeignKeyOutput {
    pub success: bool,
    pub constraint_name: String,
    pub sql_executed: String,
    pub message: String,
}

/// Input for add_unique_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddUniqueConstraintInput {
    /// Name of the table.
    pub table: String,
    /// Column names for the unique constraint.
    pub columns: Vec<String>,
    /// Optional constraint name.
    #[serde(default)]
    pub constraint_name: Option<String>,
}

/// Output for add_unique_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddUniqueConstraintOutput {
    pub success: bool,
    pub constraint_name: String,
    pub sql_executed: String,
    pub message: String,
}

/// Input for add_check_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddCheckConstraintInput {
    /// Name of the table.
    pub table: String,
    /// Check expression.
    pub expression: String,
    /// Optional constraint name.
    #[serde(default)]
    pub constraint_name: Option<String>,
}

/// Output for add_check_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddCheckConstraintOutput {
    pub success: bool,
    pub constraint_name: String,
    pub sql_executed: String,
    pub message: String,
}

/// Input for drop_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DropConstraintInput {
    /// Name of the table.
    pub table: String,
    /// Name of the constraint to drop.
    pub constraint_name: String,
}

/// Output for drop_constraint tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropConstraintOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Index column specification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum IndexColumnSpec {
    /// Simple column name.
    Simple(String),
    /// Column with options.
    WithOptions {
        name: String,
        #[serde(default)]
        order: Option<String>,
        #[serde(default)]
        nulls: Option<String>,
    },
}

/// Input for create_index tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateIndexInput {
    /// Name of the table.
    pub table: String,
    /// Columns to index.
    pub columns: Vec<IndexColumnSpec>,
    /// Index name (auto-generated if not provided).
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the index is unique.
    #[serde(default)]
    pub unique: bool,
    /// Index method (btree, hash, gin, gist, brin).
    #[serde(default)]
    pub method: Option<String>,
    /// Partial index predicate (WHERE clause).
    #[serde(default)]
    pub r#where: Option<String>,
}

/// Output for create_index tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIndexOutput {
    pub success: bool,
    pub index_name: String,
    pub sql_executed: String,
}

/// Input for drop_index tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DropIndexInput {
    /// Name of the index to drop.
    pub name: String,
}

/// Output for drop_index tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropIndexOutput {
    pub success: bool,
    pub sql_executed: String,
    pub message: String,
}

/// Input for create_enum tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateEnumInput {
    /// Name of the enum type.
    pub name: String,
    /// Enum values.
    pub values: Vec<String>,
}

/// Output for create_enum tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEnumOutput {
    pub success: bool,
    pub sql_executed: String,
}

/// Input for add_enum_value tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddEnumValueInput {
    /// Name of the enum type.
    pub enum_name: String,
    /// Value to add.
    pub value: String,
    /// Insert before this value.
    #[serde(default)]
    pub before: Option<String>,
    /// Insert after this value.
    #[serde(default)]
    pub after: Option<String>,
}

/// Output for add_enum_value tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddEnumValueOutput {
    pub success: bool,
    pub sql_executed: String,
}

/// Input for drop_enum tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DropEnumInput {
    /// Name of the enum type to drop.
    pub name: String,
}

/// Output for drop_enum tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropEnumOutput {
    pub success: bool,
    pub sql_executed: String,
}

/// Input for get_migration_preview tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetMigrationPreviewInput {
    /// Wrap in BEGIN/COMMIT.
    #[serde(default = "default_true")]
    pub include_transaction: bool,
    /// Include explanatory comments.
    #[serde(default = "default_true")]
    pub include_comments: bool,
}

/// Output for get_migration_preview tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetMigrationPreviewOutput {
    pub sql: String,
    pub operation_count: usize,
    pub has_breaking_changes: bool,
    pub summary: String,
}

/// Input for get_breaking_changes tool.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetBreakingChangesInput {}

/// Output for get_breaking_changes tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetBreakingChangesOutput {
    pub has_breaking_changes: bool,
    pub changes: Vec<BreakingChangeInfo>,
}

/// Breaking change information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChangeInfo {
    pub r#type: String,
    pub description: String,
    pub mitigation: String,
}

/// Input for commit_migration tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommitMigrationInput {
    /// Human-readable description of the migration.
    pub description: String,
    /// Commit even with breaking changes.
    #[serde(default)]
    pub force: bool,
}

/// Output for commit_migration tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitMigrationOutput {
    pub success: bool,
    pub migration_id: String,
    pub migration_path: String,
    pub sql_preview: String,
    pub message: String,
}

// =============================================================================
// Server Startup
// =============================================================================

/// Runs the MCP server on stdio.
///
/// This function starts the MCP server and blocks until it shuts down.
///
/// # Arguments
///
/// * `working_dir` - Optional working directory override
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters an error.
pub async fn run_mcp_server(working_dir: Option<PathBuf>) -> miette::Result<()> {
    use miette::{Context, IntoDiagnostic};
    use rmcp::transport::io::stdio;

    let working_dir = working_dir.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let service = TernMcpService::new(working_dir);

    tracing::info!("Starting Tern MCP server");

    let server = service
        .serve(stdio())
        .await
        .into_diagnostic()
        .wrap_err("Failed to start MCP server")?;

    server
        .waiting()
        .await
        .into_diagnostic()
        .wrap_err("MCP server error")?;

    Ok(())
}
