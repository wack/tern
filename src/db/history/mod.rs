//! Migration history and schema state reconstruction.
//!
//! This module provides the ability to:
//!
//! - **Apply operations to schemas**: Transform a `Namespace` by applying `Operation`s
//! - **Build schemas programmatically**: Ergonomic builders for test schema construction
//! - **Track migration history**: Record which migrations have been applied (future)
//! - **Reconstruct schema state**: Build schema state at any point in history (future)
//!
//! # Schema Construction with Builders
//!
//! The [`builder`] module provides ergonomic builders for constructing schemas:
//!
//! ```ignore
//! use tern::db::history::builder::NamespaceBuilder;
//!
//! let schema = NamespaceBuilder::new("public")
//!     .table("users", |t| {
//!         t.column_with("id", "integer", |c| c.not_null().identity_always())
//!          .column_with("email", "text", |c| c.not_null())
//!          .primary_key(["id"])
//!          .unique(["email"])
//!     })
//!     .enum_type("status", ["pending", "active"])
//!     .build();
//! ```
//!
//! # Operation Application
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
//! use tern::db::history::builder::NamespaceBuilder;
//! use tern::db::diff::diff_namespaces;
//! use tern::db::migrate::{MigrationPlan, PostgresRenderer};
//!
//! // Define source schema
//! let source = NamespaceBuilder::new("public")
//!     .table("users", |t| {
//!         t.column_with("id", "integer", |c| c.not_null())
//!          .column_with("age", "integer", |c| c.not_null())
//!     })
//!     .build();
//!
//! // Define target schema (widened column type)
//! let target = NamespaceBuilder::new("public")
//!     .table("users", |t| {
//!         t.column_with("id", "integer", |c| c.not_null())
//!          .column_with("age", "bigint", |c| c.not_null())  // int4 -> int8
//!     })
//!     .build();
//!
//! // Generate migration
//! let diff = diff_namespaces(&source, &target);
//! let plan = MigrationPlan::from_diff(&diff);
//! let script = plan.render(&PostgresRenderer::default());
//!
//! // Snapshot test the SQL
//! insta::assert_snapshot!(script.to_sql());
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
pub mod builder;
mod error;

pub use apply::OidGenerator;
pub use error::ApplyError;

// Note: Namespace::apply() is implemented in apply.rs as an impl block,
// so it's automatically available on the Namespace type.
