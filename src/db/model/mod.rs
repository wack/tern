//! Domain model for PostgreSQL schema representation.
//!
//! This module provides types for representing a PostgreSQL schema in memory,
//! as queried from the system catalog. The model is designed to be:
//!
//! - **Serializable**: Can be saved/loaded for migration state tracking
//! - **Comparable**: Can diff two schemas to detect changes
//! - **Complete**: Captures enough detail to regenerate DDL
//!
//! Core DDL types (Table, Column, Constraint, Index) are re-exported from `tern_ddl`.
//! Additional namespace-level types (Namespace, View, Sequence, EnumType) are defined here.
//!
//! # Example
//!
//! ```
//! use tern::db::model::{Namespace, Table, Column, TableKind};
//! use tern::db::model::types::{TypeInfo, QualifiedCollationName};
//! use tern::db::schema::{Oid, SchemaName, TableName, ColumnName, TypeName, CollationName};
//!
//! // Build a simple table representation
//! let column = Column {
//!     name: ColumnName::try_new("id".to_string()).unwrap(),
//!     position: 1,
//!     type_info: TypeInfo {
//!         name: TypeName::try_new("int4".to_string()).unwrap(),
//!         schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
//!         formatted: "integer".to_string(),
//!         is_array: false,
//!     },
//!     is_nullable: false,
//!     default: None,
//!     generated: None,
//!     identity: None,
//!     collation: QualifiedCollationName::new(
//!         SchemaName::try_new("pg_catalog".to_string()).unwrap(),
//!         CollationName::try_new("default".to_string()).unwrap(),
//!     ),
//!     comment: None,
//! };
//!
//! let table = Table {
//!     oid: Oid::new(12345),
//!     name: TableName::try_new("users".to_string()).unwrap(),
//!     kind: TableKind::Regular,
//!     columns: vec![column],
//!     constraints: vec![],
//!     indexes: vec![],
//!     comment: None,
//! };
//! ```

pub mod namespace;

// Re-export DDL types from tern-ddl
pub mod column {
    pub use tern_ddl::column::*;
}

pub mod constraint {
    pub use tern_ddl::constraint::*;
}

pub mod index {
    pub use tern_ddl::index::*;
}

pub mod table {
    pub use tern_ddl::table::*;
}

pub mod types {
    pub use tern_ddl::types::*;
}

// Re-export main types for convenience
pub use column::{Column, GeneratedColumn, GeneratedStorage, IdentityKind};
pub use constraint::{
    CheckConstraint, Constraint, ConstraintKind, ExclusionConstraint, ExclusionElement,
    ForeignKeyConstraint, PrimaryKeyConstraint, UniqueConstraint,
};
pub use index::{Index, IndexColumn, NullsOrder, SortOrder};
pub use namespace::{EnumType, Namespace, Sequence, View};
pub use table::{Table, TableKind};
pub use types::{
    Comment, ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedName,
    QualifiedTableName, QualifiedTypeName, SqlExpr, TypeInfo,
};
