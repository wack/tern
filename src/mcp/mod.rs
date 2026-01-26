//! MCP (Model Context Protocol) server for AI-assisted migration authoring.
//!
//! This module implements an MCP server that enables AI assistants to interact
//! with Tern's migration system programmatically. The server runs locally and
//! uses an in-memory PostgreSQL database (via PGLite) for schema validation.
//!
//! # Architecture
//!
//! The MCP server consists of several components:
//!
//! - **Transport**: Handles JSON-RPC communication over stdio
//! - **Protocol**: Defines MCP message types and request/response handling
//! - **Resources**: Read-only data exposed to clients (schema, migrations)
//! - **Tools**: Actions that modify state (session management, SQL execution)
//! - **Sessions**: Manages migration authoring sessions with isolated databases
//!
//! # Usage
//!
//! The server is started via the `tern mcp` command and communicates over stdio:
//!
//! ```text
//! $ tern mcp
//! ```
//!
//! # Example Session
//!
//! 1. Client reads `tern://schema` resource to understand current schema
//! 2. Client calls `start_session` tool to begin a migration
//! 3. Client makes changes via `execute_sql` or `apply_operation`
//! 4. Client calls `get_session_diff` to review changes
//! 5. Client calls `generate_migration` to create the migration file
//!
//! # Feature Requirements
//!
//! This module requires the `pglite` feature for the embedded PostgreSQL
//! functionality used in session databases.

// Allow dead code for work-in-progress items that will be used when PGLite integration is complete.
#![allow(dead_code)]

mod error;
mod protocol;
mod resources;
mod server;
mod session;
mod tools;
mod transport;

pub use error::{McpError, SessionId};
pub use server::McpServer;
