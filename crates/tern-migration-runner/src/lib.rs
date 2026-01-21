//! Tern Migration Runner
//!
//! This crate provides the WebAssembly runtime for executing Tern migration
//! components. It handles loading, validating, and running migration components
//! that have been compiled from database schema changes.
//!
//! # Overview
//!
//! The migration runner is the execution engine for Tern's compiled migrations.
//! It provides:
//!
//! - **Component loading**: Load WebAssembly components from bytes or files
//! - **Metadata extraction**: Inspect migrations without executing them
//! - **Database integration**: Execute SQL statements against PostgreSQL
//! - **Dry-run mode**: Preview SQL statements without execution
//! - **Progress logging**: Emit log messages during execution
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                      Migration Component                        │
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │  Exports:                                                │   │
//! │  │  - describe() -> Metadata                                │   │
//! │  │  - get_statements() -> Vec<Statement>                    │   │
//! │  │  - run() -> Result<(), String>                           │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! └──────────────────────────────┬──────────────────────────────────┘
//!                                │
//!                    Imports:    │
//!                    - database::execute(sql)
//!                    - database::query(sql)
//!                    - log::log(level, message)
//!                                │
//! ┌──────────────────────────────▼──────────────────────────────────┐
//! │                      Migration Runtime                           │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐  │
//! │  │   Wasmtime  │  │ Host State  │  │   PostgreSQL Client     │  │
//! │  │   Engine    │  │  (dry-run,  │  │   (tokio-postgres)      │  │
//! │  │             │  │   logging)  │  │                         │  │
//! │  └─────────────┘  └─────────────┘  └─────────────────────────┘  │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ## Basic Usage
//!
//! ```ignore
//! use tern_migration_runner::runtime::MigrationRuntime;
//! use tern_migration_runner::host::HostState;
//!
//! // Load component from bytes
//! let component_bytes = std::fs::read("migration.wasm")?;
//! let runtime = MigrationRuntime::from_bytes(&component_bytes)?;
//!
//! // Describe the migration (dry-run mode)
//! let metadata = runtime.describe().await?;
//! println!("Migration: {}", metadata.description);
//! println!("Statements: {}", metadata.statement_count);
//!
//! // Get all SQL statements
//! let statements = runtime.get_statements().await?;
//! for stmt in &statements {
//!     println!("{}: {}", stmt.description, stmt.sql);
//! }
//! ```
//!
//! ## Executing Migrations
//!
//! ```ignore
//! use tern_migration_runner::host::{connect_database, HostState};
//!
//! // Connect to database
//! let client = connect_database("postgres://localhost/mydb").await?;
//! let host_state = HostState::with_client(client);
//!
//! // Execute the migration
//! runtime.run(host_state).await?;
//! ```
//!
//! ## Dry-Run Mode
//!
//! ```ignore
//! // Get SQL statements without executing
//! let sql_statements = runtime.run_dry().await?;
//! for sql in &sql_statements {
//!     println!("{}", sql);
//! }
//! ```
//!
//! # Modules
//!
//! - [`error`] - Error types for runtime, database, and CLI operations
//! - [`host`] - Host function implementations (database, logging)
//! - [`runtime`] - WebAssembly runtime wrapper using Wasmtime

pub mod error;
pub mod host;
pub mod runtime;

// Re-export main types for convenience
pub use error::{CliError, DatabaseError, RuntimeError};
pub use host::{DbError, HostState, LogLevel, LogMessage, connect_database};
pub use runtime::{
    BreakingChangeInfo, MigrationMetadata, MigrationRuntime, MigrationRuntimeBuilder,
    MitigationStrategy, RuntimeConfig, StatementInfo, StoreState,
};

/// Current version of the migration runner.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Name of the runner executable.
pub const EXECUTABLE_NAME: &str = "tern-migration-runner";
