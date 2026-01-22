//! MCP (Model Context Protocol) server for Tern.
//!
//! This module provides an MCP server that exposes Tern's schema modification
//! capabilities as programmatic tools. This enables AI assistants and other MCP
//! clients to create database migrations through a sequence of structured tool
//! calls, without requiring users to manually edit SQL files.
//!
//! # Architecture
//!
//! The MCP server consists of several components:
//!
//! - [`TernMcpService`]: The main MCP service implementing the MCP protocol
//! - [`Session`]: Manages the working state including PGLite runtime
//! - Tool modules: Individual tools organized by category
//!
//! # Example
//!
//! ```ignore
//! use tern::mcp::run_mcp_server;
//!
//! // Start the MCP server on stdio
//! run_mcp_server().await?;
//! ```

mod error;
mod server;
mod session;
pub mod tools;

pub use error::McpError;
pub use server::{TernMcpService, run_mcp_server};
pub use session::Session;
