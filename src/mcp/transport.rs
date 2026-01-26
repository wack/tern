//! Stdio transport for MCP server.
//!
//! This module handles reading JSON-RPC messages from stdin and writing
//! responses to stdout. Messages are newline-delimited JSON.

use std::io::{self, BufRead, Write};

use crate::mcp::error::McpError;
use crate::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};

/// Transport layer for stdio-based MCP communication.
///
/// Messages are newline-delimited JSON. Each message is a single line.
pub struct StdioTransport {
    /// Buffered stdin reader.
    reader: io::BufReader<io::Stdin>,
    /// Stdout writer.
    writer: io::Stdout,
}

impl StdioTransport {
    /// Creates a new stdio transport.
    #[must_use]
    pub fn new() -> Self {
        Self {
            reader: io::BufReader::new(io::stdin()),
            writer: io::stdout(),
        }
    }

    /// Reads the next JSON-RPC request from stdin.
    ///
    /// Returns `None` if stdin is closed (EOF).
    ///
    /// # Errors
    ///
    /// Returns an error if reading fails or the JSON is invalid.
    pub fn read_request(&mut self) -> Result<Option<JsonRpcRequest>, McpError> {
        let mut line = String::new();

        match self.reader.read_line(&mut line) {
            Ok(0) => Ok(None), // EOF
            Ok(_) => {
                let line = line.trim();
                if line.is_empty() {
                    // Skip empty lines and try again
                    return self.read_request();
                }

                tracing::debug!("Received: {}", line);

                serde_json::from_str(line).map(Some).map_err(|e| {
                    McpError::ParseError(format!("failed to parse JSON-RPC request: {e}"))
                })
            }
            Err(e) => Err(McpError::IoError(e)),
        }
    }

    /// Writes a JSON-RPC response to stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_response(&mut self, response: &JsonRpcResponse) -> Result<(), McpError> {
        let json = serde_json::to_string(response)
            .map_err(|e| McpError::InternalError(format!("failed to serialize response: {e}")))?;

        tracing::debug!("Sending: {}", json);

        writeln!(self.writer, "{json}").map_err(McpError::IoError)?;
        self.writer.flush().map_err(McpError::IoError)?;

        Ok(())
    }
}

impl Default for StdioTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::protocol::{JSONRPC_VERSION, JsonRpcError, RequestId};

    #[test]
    fn transport_creation() {
        // Just verify we can create a transport
        let _transport = StdioTransport::new();
    }

    #[test]
    fn response_serialization() {
        // Test that responses serialize correctly
        let response =
            JsonRpcResponse::success(RequestId::from(1_i64), serde_json::json!({"ok": true}));
        let json = serde_json::to_string(&response).unwrap();

        assert!(json.contains("\"jsonrpc\""));
        assert!(json.contains(&format!("\"{}\"", JSONRPC_VERSION)));
        assert!(json.contains("\"result\""));
    }

    #[test]
    fn error_response_serialization() {
        let response = JsonRpcResponse::error(
            RequestId::from(1_i64),
            JsonRpcError::new(-32600, "test error"),
        );
        let json = serde_json::to_string(&response).unwrap();

        assert!(json.contains("\"error\""));
        assert!(json.contains("-32600"));
        assert!(json.contains("test error"));
    }
}
