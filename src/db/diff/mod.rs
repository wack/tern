//! Schema comparison and diff generation.
//!
//! This module provides functionality for comparing two PostgreSQL schema snapshots
//! and producing a detailed diff that can be used to generate migration DDL.
//!
//! # Overview
//!
//! The diff system compares source and target schemas, identifying:
//! - **Added** items: Present in target but not in source
//! - **Removed** items: Present in source but not in target
//! - **Modified** items: Present in both but with differences
//! - **Potential renames**: Items that may have been renamed (detected by similarity)
//!
//! # Rename Detection
//!
//! When an item is removed from source and a similar item is added in target,
//! the diff system can detect this as a potential rename rather than a drop + create.
//! This is important because:
//! - `ALTER TABLE RENAME` preserves data
//! - `DROP TABLE` + `CREATE TABLE` loses data
//!
//! Rename detection uses similarity scoring based on structural comparison
//! (column overlap, type matches, constraint similarity, etc.).
//!
//! # Example
//!
//! ```
//! use tern::db::diff::{diff_namespaces, DiffConfig};
//! use tern::db::model::Namespace;
//! # use tern::db::schema::{Oid, SchemaName};
//!
//! # fn example(source: Namespace, target: Namespace) {
//! // Compare with default settings (rename threshold = 0.5)
//! let diff = diff_namespaces(&source, &target);
//!
//! if diff.is_empty() {
//!     println!("Schemas are identical");
//! } else {
//!     println!("Tables added: {}", diff.tables.added.len());
//!     println!("Tables removed: {}", diff.tables.removed.len());
//!     println!("Tables modified: {}", diff.tables.modified.len());
//!     println!("Potential table renames: {}", diff.tables.potential_renames.len());
//! }
//! # }
//! ```

mod compare;
mod schema_diff;
mod types;

// Re-export main types
pub use compare::{diff_namespaces, diff_namespaces_with_config};
pub use schema_diff::{
    ColumnDiff, ConstraintDiff, EnumDiff, IndexDiff, ModifiedColumn, ModifiedConstraint,
    ModifiedEnum, ModifiedIndex, ModifiedSequence, ModifiedTable, ModifiedView, NamespaceDiff,
    SequenceDiff, TableDiff, ViewDiff,
};
pub use types::{Diff, DiffConfig, FieldChange, PotentialRename};

#[cfg(test)]
mod tests;
