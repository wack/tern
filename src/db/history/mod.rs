//! Migration history and schema state reconstruction.
//!
//! This module provides the ability to:
//!
//! - **Apply operations to schemas**: Transform a `Namespace` by applying `Operation`s
//! - **Track migration history**: Record which migrations have been applied (future)
//! - **Reconstruct schema state**: Build schema state at any point in history (future)
//!
//! # Core Capability: Operation Application
//!
//! The `Namespace::apply()` method is the inverse of the diff pipeline:
//!
//! ```text
//! Diff Pipeline:     Namespace → Namespace → NamespaceDiff → Vec<Operation>
//! Apply Pipeline:    Namespace + Vec<Operation> → Namespace
//! ```
//!
//! This bidirectional relationship enables:
//!
//! - **Snapshot testing**: Define source/target schemas programmatically, diff them,
//!   and snapshot the generated SQL without needing a database connection
//! - **History reconstruction**: Walk a chain of migrations to rebuild any historical
//!   schema state
//! - **Verification**: Apply operations to a source, re-diff against target, verify
//!   the diff is empty
//!
//! # Example: Snapshot Testing
//!
//! ```ignore
//! use tern::db::model::Namespace;
//! use tern::db::diff::diff_namespaces;
//! use tern::db::migrate::{MigrationPlan, PostgresRenderer};
//!
//! // Define source schema (e.g., with int4 column)
//! let source = Namespace::empty("public");
//! // ... add table with integer column
//!
//! // Define target schema (e.g., with int8 column)
//! let target = Namespace::empty("public");
//! // ... add table with bigint column
//!
//! // Generate migration
//! let diff = diff_namespaces(&source, &target);
//! let plan = MigrationPlan::from_diff(&diff);
//! let script = plan.render(&PostgresRenderer::default());
//!
//! // Snapshot test the SQL
//! insta::assert_snapshot!(script.to_sql());
//!
//! // Verify roundtrip: applying the operations should produce the target
//! let operations = plan.operations();
//! let result = source.apply(operations)?;
//! assert_eq!(diff_namespaces(&result, &target).is_empty(), true);
//! ```
//!
//! # Future: Migration History Tracking
//!
//! The history module will be extended to support:
//!
//! - `MigrationId`: Content-addressable identifier for migrations
//! - `Migration`: A recorded migration with operations and metadata
//! - `MigrationHistory`: The complete history of applied migrations
//!
//! This will enable production migration tracking and multi-step history
//! reconstruction.

mod apply;
mod error;

pub use apply::OidGenerator;
pub use error::ApplyError;

// Note: Namespace::apply() is implemented in apply.rs as an impl block,
// so it's automatically available on the Namespace type.
