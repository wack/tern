//! MCP server command implementation.
//!
//! This module provides the `tern mcp` command that starts an MCP server
//! for AI-assisted migration authoring.

use std::path::PathBuf;

use clap::Parser;
use miette::IntoDiagnostic;

use crate::cli::{ensure_backend_initialized, load_backend};
use crate::db::state::StateBackend;
use crate::mcp::McpServer;

/// Start the MCP server for AI-assisted migration authoring.
///
/// The MCP (Model Context Protocol) server enables AI assistants to interact
/// with Tern's migration system. It uses an in-memory PostgreSQL database
/// (via PGLite) to validate schema changes before generating migrations.
///
/// # Communication
///
/// The server communicates over stdio using newline-delimited JSON-RPC 2.0
/// messages. This is compatible with AI assistants that support the MCP
/// protocol, such as Claude Desktop.
///
/// # Example Configuration
///
/// Add to your MCP configuration:
///
/// ```json
/// {
///   "mcpServers": {
///     "tern": {
///       "command": "tern",
///       "args": ["mcp"],
///       "cwd": "/path/to/project"
///     }
///   }
/// }
/// ```
///
/// # Available Tools
///
/// - `start_session`: Begin a new migration authoring session
/// - `execute_sql`: Execute SQL in the session's in-memory database
/// - `apply_operation`: Apply a structured schema operation
/// - `get_session_schema`: View the current schema state
/// - `get_session_diff`: View pending changes compared to baseline
/// - `generate_migration`: Create a migration from session changes
/// - `cancel_session`: Discard session without creating a migration
/// - `list_sessions`: List all active sessions
///
/// # Available Resources
///
/// - `tern://schema`: Current database schema
/// - `tern://migrations`: List of all migrations
/// - `tern://migration/{id}`: Details of a specific migration
#[derive(Debug, Parser, Clone)]
pub struct Mcp {
    /// Path to the state directory
    ///
    /// If not specified, uses `.tern/` in the current directory.
    #[arg(long)]
    state_path: Option<PathBuf>,
}

impl Mcp {
    /// Runs the MCP server.
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.state_path.as_deref());

        // Ensure backend is initialized
        ensure_backend_initialized(&backend).await?;

        // Load current state
        let current_state = backend.get_current_state().await.into_diagnostic()?;

        // Create and run the server
        let server = McpServer::new(backend, current_state);
        server.run_stdio().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_command_parses() {
        // Just verify the command can be parsed
        use clap::CommandFactory;
        let _ = Mcp::command();
    }
}
