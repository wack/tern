//! MCP server implementation.
//!
//! This module provides the main `McpServer` struct that handles the MCP
//! protocol, routing requests to the appropriate handlers.

use std::sync::Arc;

use serde_json::Value;
use tokio::sync::RwLock;

use crate::db::model::Namespace;
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::mcp::error::McpError;
use crate::mcp::protocol::{
    CallToolParams, CallToolResult, InitializeParams, InitializeResult, JsonRpcError,
    JsonRpcRequest, JsonRpcResponse, ListResourcesResult, ListToolsResult, PingResult,
    ReadResourceParams, ReadResourceResult, RequestId, ResourcesCapability, ServerCapabilities,
    ToolsCapability,
};
use crate::mcp::resources;
use crate::mcp::session::SessionManager;
use crate::mcp::tools;
use crate::mcp::transport::StdioTransport;

/// MCP server for AI-assisted migration authoring.
pub struct McpServer {
    /// State backend for reading/writing migrations.
    backend: Arc<LocalFileBackend>,
    /// Current schema state (base state for sessions).
    current_state: Arc<Namespace>,
    /// Session manager for migration authoring sessions.
    session_manager: Arc<RwLock<SessionManager>>,
    /// Whether the server has been initialized.
    initialized: bool,
}

impl McpServer {
    /// Creates a new MCP server with the given backend and state.
    pub fn new(backend: LocalFileBackend, current_state: Namespace) -> Self {
        let backend = Arc::new(backend);
        let session_manager = Arc::new(RwLock::new(SessionManager::new(
            current_state.clone(),
            (*backend).clone(),
        )));

        Self {
            backend,
            current_state: Arc::new(current_state),
            session_manager,
            initialized: false,
        }
    }

    /// Initializes the MCP server with the given backend.
    ///
    /// This method:
    /// 1. Verifies the backend is initialized
    /// 2. Loads the current schema state
    /// 3. Sets up the session manager
    ///
    /// # Errors
    ///
    /// Returns an error if the backend is not initialized or the state
    /// cannot be loaded.
    pub async fn initialize(backend: LocalFileBackend) -> Result<Self, McpError> {
        // Verify backend is initialized
        if !backend
            .is_initialized()
            .await
            .map_err(|e| McpError::BackendNotInitialized(e.to_string()))?
        {
            return Err(McpError::BackendNotInitialized(
                "backend not initialized; run 'tern init' first".into(),
            ));
        }

        // Load current state
        let current_state = backend
            .get_current_state()
            .await
            .map_err(|e| McpError::BackendError(e.to_string()))?;

        Ok(Self::new(backend, current_state))
    }

    /// Runs the MCP server over stdio.
    ///
    /// This method enters a loop reading requests from stdin and writing
    /// responses to stdout until EOF is received.
    pub async fn run_stdio(mut self) -> miette::Result<()> {
        let mut transport = StdioTransport::new();

        tracing::info!("MCP server started, waiting for requests");

        loop {
            match transport.read_request() {
                Ok(Some(request)) => {
                    let response = self.handle_request(request).await;
                    if let Err(e) = transport.write_response(&response) {
                        tracing::error!("Failed to write response: {}", e);
                        break;
                    }
                }
                Ok(None) => {
                    tracing::info!("Received EOF, shutting down");
                    break;
                }
                Err(e) => {
                    tracing::error!("Failed to read request: {}", e);
                    // Try to send an error response
                    let response = JsonRpcResponse::error(
                        RequestId::Null,
                        JsonRpcError::parse_error(e.to_string()),
                    );
                    let _ = transport.write_response(&response);
                }
            }
        }

        Ok(())
    }

    /// Handles a single JSON-RPC request and returns a response.
    async fn handle_request(&mut self, request: JsonRpcRequest) -> JsonRpcResponse {
        tracing::debug!("Handling request: method={}", request.method);

        let result = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.params).await,
            "initialized" => {
                // Notification, no response needed but we return empty for consistency
                Ok(Value::Null)
            }
            "ping" => self.handle_ping().await,
            "resources/list" => self.handle_list_resources(request.params).await,
            "resources/read" => self.handle_read_resource(request.params).await,
            "tools/list" => self.handle_list_tools(request.params).await,
            "tools/call" => self.handle_call_tool(request.params).await,
            _ => Err(McpError::MethodNotFound(request.method.clone())),
        };

        match result {
            Ok(value) => JsonRpcResponse::success(request.id, value),
            Err(error) => JsonRpcResponse::from_mcp_error(request.id, &error),
        }
    }

    /// Handles the initialize request.
    async fn handle_initialize(&mut self, params: Option<Value>) -> Result<Value, McpError> {
        let _params: InitializeParams = params
            .map(|p| serde_json::from_value(p).map_err(|e| McpError::InvalidParams(e.to_string())))
            .transpose()?
            .unwrap_or_else(|| InitializeParams {
                protocol_version: "2024-11-05".to_string(),
                capabilities: Default::default(),
                client_info: crate::mcp::protocol::ClientInfo {
                    name: "unknown".to_string(),
                    version: "0.0.0".to_string(),
                },
            });

        self.initialized = true;

        let result = InitializeResult {
            capabilities: ServerCapabilities {
                resources: Some(ResourcesCapability {
                    subscribe: false,
                    list_changed: false,
                }),
                tools: Some(ToolsCapability {
                    list_changed: false,
                }),
                prompts: None,
            },
            ..Default::default()
        };

        serde_json::to_value(result).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Handles the ping request.
    async fn handle_ping(&self) -> Result<Value, McpError> {
        serde_json::to_value(PingResult {}).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Handles the resources/list request.
    async fn handle_list_resources(&self, _params: Option<Value>) -> Result<Value, McpError> {
        let result = ListResourcesResult {
            resources: resources::list_resources(),
            next_cursor: None,
        };

        serde_json::to_value(result).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Handles the resources/read request.
    async fn handle_read_resource(&self, params: Option<Value>) -> Result<Value, McpError> {
        let params: ReadResourceParams = params
            .map(|p| serde_json::from_value(p).map_err(|e| McpError::InvalidParams(e.to_string())))
            .transpose()?
            .ok_or_else(|| McpError::InvalidParams("missing params".into()))?;

        let content =
            resources::read_resource(&params.uri, &self.backend, &self.current_state).await?;

        let result = ReadResourceResult {
            contents: vec![content],
        };

        serde_json::to_value(result).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Handles the tools/list request.
    async fn handle_list_tools(&self, _params: Option<Value>) -> Result<Value, McpError> {
        let result = ListToolsResult {
            tools: tools::list_tools(),
            next_cursor: None,
        };

        serde_json::to_value(result).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Handles the tools/call request.
    async fn handle_call_tool(&mut self, params: Option<Value>) -> Result<Value, McpError> {
        let params: CallToolParams = params
            .map(|p| serde_json::from_value(p).map_err(|e| McpError::InvalidParams(e.to_string())))
            .transpose()?
            .ok_or_else(|| McpError::InvalidParams("missing params".into()))?;

        let result = self.dispatch_tool(&params.name, params.arguments).await?;

        let tool_result = CallToolResult::json(result);

        serde_json::to_value(tool_result).map_err(|e| McpError::InternalError(e.to_string()))
    }

    /// Dispatches a tool call to the appropriate handler.
    async fn dispatch_tool(&mut self, name: &str, arguments: Value) -> Result<Value, McpError> {
        match name {
            "start_session" => {
                let input: tools::StartSessionInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_start_session(input, &mut manager).await
            }
            "execute_sql" => {
                let input: tools::ExecuteSqlInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_execute_sql(input, &mut manager).await
            }
            "apply_operation" => {
                let input: tools::ApplyOperationInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_apply_operation(input, &mut manager).await
            }
            "get_session_schema" => {
                let input: tools::GetSessionSchemaInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_get_session_schema(input, &mut manager).await
            }
            "get_session_diff" => {
                let input: tools::GetSessionDiffInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_get_session_diff(input, &mut manager).await
            }
            "generate_migration" => {
                let input: tools::GenerateMigrationInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_generate_migration(input, &mut manager).await
            }
            "cancel_session" => {
                let input: tools::CancelSessionInput = serde_json::from_value(arguments)
                    .map_err(|e| McpError::InvalidParams(e.to_string()))?;
                let mut manager = self.session_manager.write().await;
                tools::handle_cancel_session(input, &mut manager).await
            }
            "list_sessions" => {
                let manager = self.session_manager.read().await;
                tools::handle_list_sessions(&manager).await
            }
            _ => Err(McpError::MethodNotFound(format!("tool not found: {name}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_server() -> (TempDir, McpServer) {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));

        // Initialize the backend
        backend.initialize().await.unwrap();

        // Create an empty initial state
        let namespace = Namespace::empty("public");
        backend.save_current_state(&namespace).await.unwrap();

        let server = McpServer::new(backend, namespace);
        (temp_dir, server)
    }

    #[tokio::test]
    async fn server_handles_initialize() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "test",
                    "version": "1.0.0"
                }
            })),
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn server_handles_ping() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "ping".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn server_handles_list_resources() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "resources/list".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());

        let result: ListResourcesResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(!result.resources.is_empty());
    }

    #[tokio::test]
    async fn server_handles_list_tools() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());

        let result: ListToolsResult = serde_json::from_value(response.result.unwrap()).unwrap();
        assert!(!result.tools.is_empty());
    }

    #[tokio::test]
    async fn server_handles_unknown_method() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "unknown/method".to_string(),
            params: None,
        };

        let response = server.handle_request(request).await;
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn server_handles_start_session() {
        let (_temp_dir, mut server) = create_test_server().await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::from(1_i64),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "start_session",
                "arguments": {
                    "description": "Test migration"
                }
            })),
        };

        let response = server.handle_request(request).await;
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }
}
