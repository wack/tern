//! Migration execution module.
//!
//! This module provides functionality for executing migrations against live
//! PostgreSQL databases and tracking their state in the `tern` schema.
//!
//! # Architecture
//!
//! The execution module consists of:
//!
//! - [`MigrationTracker`]: Manages the `tern.migrations` and `tern.current`
//!   tables that track applied migrations
//! - [`MigrationExecutor`]: Coordinates the execution of pending migrations
//! - Error types for execution failures
//!
//! # Database Tables
//!
//! The module creates and manages two tables in the `tern` schema:
//!
//! ## tern.migrations
//!
//! Records all applied migrations:
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | id | TEXT | Migration ID (BLAKE3 content hash) |
//! | sequence | INTEGER | Application order (1, 2, 3, ...) |
//! | description | TEXT | Human-readable description |
//! | migration_hash | TEXT | BLAKE3 hash of operations (integrity check) |
//! | schema_hash | TEXT | xxhash3 64-bit hash of resulting schema |
//! | applied_at | TIMESTAMPTZ | When the migration was applied |
//!
//! ## tern.current
//!
//! Single-row table pointing to the current migration:
//!
//! | Column | Type | Description |
//! |--------|------|-------------|
//! | singleton | BOOLEAN | Always true (ensures single row) |
//! | migration_id | TEXT | Foreign key to tern.migrations |
//! | updated_at | TIMESTAMPTZ | Last update time |
//!
//! # Checksum Purposes
//!
//! - **migration_hash**: BLAKE3 hash of the operations array only (not
//!   description/timestamps). Used to detect if historical migrations were
//!   modified (history divergence). Updating descriptions won't trigger
//!   divergence errors.
//!
//! - **schema_hash**: xxhash3 64-bit checksum of the resulting schema. Used
//!   for quick state comparison and drift detection.
//!
//! # Example
//!
//! ```ignore
//! use tern::db::{connect, execution::MigrationExecutor};
//! use tern::db::state::LocalFileBackend;
//!
//! async fn run_migrations() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = connect("postgres://localhost/mydb").await?;
//!     let backend = LocalFileBackend::default_location();
//!
//!     let executor = MigrationExecutor::new(&client, &backend, "public");
//!     let result = executor.execute_pending(false).await?;
//!
//!     println!("Applied {} migration(s)", result.count());
//!     Ok(())
//! }
//! ```

mod error;
mod executor;
mod tracker;

pub use error::{ExecutionError, ExecutionResult, MigrationResult};
pub use executor::{MigrationExecutor, compute_migration_hash};
pub use tracker::{MigrationRecord, MigrationTracker};
