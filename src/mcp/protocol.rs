//! MCP protocol types and JSON-RPC message handling.
//!
//! This module defines the types used for MCP (Model Context Protocol)
//! communication, following the JSON-RPC 2.0 specification.
//!
//! # Protocol Overview
//!
//! MCP uses JSON-RPC 2.0 over stdio for communication. Messages are:
//! - **Requests**: Client asks server to do something
//! - **Responses**: Server replies to a request
//! - **Notifications**: One-way messages (no response expected)

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp::error::McpError;

/// JSON-RPC version string (always "2.0").
pub const JSONRPC_VERSION: &str = "2.0";

/// MCP protocol version supported by this server.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Server name for MCP initialize response.
pub const SERVER_NAME: &str = "tern";

/// Server version for MCP initialize response.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// =============================================================================
// JSON-RPC Message Types
// =============================================================================

/// A JSON-RPC request message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Request identifier (used to match responses).
    pub id: RequestId,
    /// Method name to invoke.
    pub method: String,
    /// Method parameters (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Request identifier (matches the request).
    pub id: RequestId,
    /// Result on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Creates a successful response.
    #[must_use]
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Creates an error response.
    #[must_use]
    pub fn error(id: RequestId, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }

    /// Creates an error response from an `McpError`.
    #[must_use]
    pub fn from_mcp_error(id: RequestId, error: &McpError) -> Self {
        Self::error(id, JsonRpcError::from_mcp_error(error))
    }
}

/// A JSON-RPC notification message (no response expected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,
    /// Method name.
    pub method: String,
    /// Method parameters (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    /// Error code.
    pub code: i32,
    /// Human-readable error message.
    pub message: String,
    /// Additional error data (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Creates a new JSON-RPC error.
    #[must_use]
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Creates a JSON-RPC error with additional data.
    #[must_use]
    pub fn with_data(code: i32, message: impl Into<String>, data: Value) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Creates a JSON-RPC error from an `McpError`.
    #[must_use]
    pub fn from_mcp_error(error: &McpError) -> Self {
        let code = error.error_code();
        let message = error.to_string();

        // Add additional data for certain error types
        let data = match error {
            McpError::SessionNotFound(session_id) => Some(serde_json::json!({
                "sessionId": session_id.as_str(),
            })),
            McpError::SessionLimitReached {
                max_sessions,
                current_sessions,
            } => Some(serde_json::json!({
                "maxSessions": max_sessions,
                "currentSessions": current_sessions,
            })),
            McpError::SqlExecutionFailed {
                code,
                detail,
                hint,
                position,
                ..
            } => Some(serde_json::json!({
                "code": code,
                "detail": detail,
                "hint": hint,
                "position": position,
            })),
            McpError::BreakingChangesDetected { breaking_changes } => Some(serde_json::json!({
                "breakingChanges": breaking_changes,
            })),
            _ => None,
        };

        Self {
            code,
            message,
            data,
        }
    }

    /// Creates a parse error.
    #[must_use]
    pub fn parse_error(message: impl Into<String>) -> Self {
        Self::new(-32700, message)
    }

    /// Creates an invalid request error.
    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(-32600, message)
    }

    /// Creates a method not found error.
    #[must_use]
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("method not found: {method}"))
    }

    /// Creates an invalid params error.
    #[must_use]
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    /// Creates an internal error.
    #[must_use]
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new(-32603, message)
    }
}

/// A request identifier (can be string, number, or null).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[derive(Default)]
pub enum RequestId {
    /// String identifier.
    String(String),
    /// Numeric identifier.
    Number(i64),
    /// Null identifier (for notifications converted to requests).
    #[default]
    Null,
}

impl From<i64> for RequestId {
    fn from(n: i64) -> Self {
        Self::Number(n)
    }
}

impl From<String> for RequestId {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for RequestId {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

// =============================================================================
// MCP-Specific Types
// =============================================================================

/// MCP initialize request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    /// Protocol version the client supports.
    pub protocol_version: String,
    /// Capabilities the client supports.
    pub capabilities: ClientCapabilities,
    /// Information about the client.
    pub client_info: ClientInfo,
}

/// MCP client capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    /// Whether the client supports roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    /// Whether the client supports sampling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<Value>,
}

/// Roots capability configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootsCapability {
    /// Whether the client can list roots.
    #[serde(default)]
    pub list_changed: bool,
}

/// Information about the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Client name.
    pub name: String,
    /// Client version.
    pub version: String,
}

/// MCP initialize response result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    /// Protocol version the server supports.
    pub protocol_version: String,
    /// Capabilities the server supports.
    pub capabilities: ServerCapabilities,
    /// Information about the server.
    pub server_info: ServerInfo,
}

impl Default for InitializeResult {
    fn default() -> Self {
        Self {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: ServerCapabilities::default(),
            server_info: ServerInfo {
                name: SERVER_NAME.to_string(),
                version: SERVER_VERSION.to_string(),
            },
        }
    }
}

/// MCP server capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerCapabilities {
    /// Resources capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    /// Tools capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    /// Prompts capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
}

/// Resources capability configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcesCapability {
    /// Whether resources are supported.
    #[serde(default)]
    pub subscribe: bool,
    /// Whether resource listing is supported.
    #[serde(default)]
    pub list_changed: bool,
}

/// Tools capability configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolsCapability {
    /// Whether tool listing changes are supported.
    #[serde(default)]
    pub list_changed: bool,
}

/// Prompts capability configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptsCapability {
    /// Whether prompt listing changes are supported.
    #[serde(default)]
    pub list_changed: bool,
}

/// Information about the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,
    /// Server version.
    pub version: String,
}

// =============================================================================
// Resource Types
// =============================================================================

/// A resource descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    /// Resource URI.
    pub uri: String,
    /// Human-readable name.
    pub name: String,
    /// Description of the resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// MIME type of the resource content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

/// Resource content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceContent {
    /// Resource URI.
    pub uri: String,
    /// MIME type of the content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Text content (for text resources).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Binary content as base64 (for binary resources).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// List resources request parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResourcesParams {
    /// Cursor for pagination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// List resources response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResourcesResult {
    /// Available resources.
    pub resources: Vec<Resource>,
    /// Cursor for next page, if more results available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Read resource request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourceParams {
    /// URI of the resource to read.
    pub uri: String,
}

/// Read resource response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResourceResult {
    /// Resource contents.
    pub contents: Vec<ResourceContent>,
}

// =============================================================================
// Tool Types
// =============================================================================

/// A tool descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Tool name.
    pub name: String,
    /// Description of what the tool does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema for the tool's input parameters.
    pub input_schema: Value,
}

/// List tools request parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsParams {
    /// Cursor for pagination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// List tools response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    /// Available tools.
    pub tools: Vec<Tool>,
    /// Cursor for next page, if more results available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Call tool request parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolParams {
    /// Name of the tool to call.
    pub name: String,
    /// Arguments to pass to the tool.
    #[serde(default)]
    pub arguments: Value,
}

/// Call tool response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    /// Tool output content.
    pub content: Vec<ToolContent>,
    /// Whether the tool execution resulted in an error.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

impl CallToolResult {
    /// Creates a successful text result.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text { text: text.into() }],
            is_error: false,
        }
    }

    /// Creates a successful JSON result.
    #[must_use]
    pub fn json(value: Value) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
            }],
            is_error: false,
        }
    }

    /// Creates an error result.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text {
                text: message.into(),
            }],
            is_error: true,
        }
    }
}

/// Tool output content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolContent {
    /// Text content.
    #[serde(rename = "text")]
    Text {
        /// The text content.
        text: String,
    },
    /// Image content (base64 encoded).
    #[serde(rename = "image")]
    Image {
        /// Base64 encoded image data.
        data: String,
        /// MIME type of the image.
        mime_type: String,
    },
}

// =============================================================================
// Ping
// =============================================================================

/// Ping response (empty object).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PingResult {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_variants() {
        let num_id = RequestId::from(42_i64);
        let str_id = RequestId::from("test-id");
        let null_id = RequestId::default();

        assert_eq!(num_id, RequestId::Number(42));
        assert_eq!(str_id, RequestId::String("test-id".to_string()));
        assert_eq!(null_id, RequestId::Null);
    }

    #[test]
    fn json_rpc_response_success() {
        let response =
            JsonRpcResponse::success(RequestId::from(1_i64), serde_json::json!({"ok": true}));

        assert!(response.result.is_some());
        assert!(response.error.is_none());
        assert_eq!(response.jsonrpc, JSONRPC_VERSION);
    }

    #[test]
    fn json_rpc_response_error() {
        let error = JsonRpcError::new(-32600, "invalid request");
        let response = JsonRpcResponse::error(RequestId::from(1_i64), error);

        assert!(response.result.is_none());
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32600);
    }

    #[test]
    fn json_rpc_error_helpers() {
        assert_eq!(JsonRpcError::parse_error("test").code, -32700);
        assert_eq!(JsonRpcError::invalid_request("test").code, -32600);
        assert_eq!(JsonRpcError::method_not_found("test").code, -32601);
        assert_eq!(JsonRpcError::invalid_params("test").code, -32602);
        assert_eq!(JsonRpcError::internal_error("test").code, -32603);
    }

    #[test]
    fn initialize_result_defaults() {
        let result = InitializeResult::default();

        assert_eq!(result.protocol_version, MCP_PROTOCOL_VERSION);
        assert_eq!(result.server_info.name, SERVER_NAME);
        assert_eq!(result.server_info.version, SERVER_VERSION);
    }

    #[test]
    fn call_tool_result_helpers() {
        let text_result = CallToolResult::text("hello");
        assert!(!text_result.is_error);
        assert_eq!(text_result.content.len(), 1);

        let json_result = CallToolResult::json(serde_json::json!({"key": "value"}));
        assert!(!json_result.is_error);

        let error_result = CallToolResult::error("something went wrong");
        assert!(error_result.is_error);
    }

    #[test]
    fn request_serialization() {
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: RequestId::from(1_i64),
            method: "test/method".to_string(),
            params: Some(serde_json::json!({"arg": "value"})),
        };

        let json = serde_json::to_string(&request).unwrap();
        let parsed: JsonRpcRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.method, "test/method");
        assert_eq!(parsed.id, RequestId::Number(1));
    }
}
