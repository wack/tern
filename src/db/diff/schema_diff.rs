//! Schema-specific diff types.
//!
//! This module provides concrete diff types for PostgreSQL schema objects.

use serde::{Deserialize, Serialize};

use crate::db::model::{
    Column, Constraint, EnumType, Index, Sequence, Table, TableKind, View,
};
use crate::db::model::types::{Comment, QualifiedCollationName, SqlExpr, TypeInfo};
use crate::db::model::column::{GeneratedColumn, IdentityKind};
use crate::db::schema::{
    ColumnName, ConstraintName, IndexName, SchemaName, SequenceName, TableName, TypeName,
};

use super::types::{Diff, FieldChange};

// =============================================================================
// Type Aliases for Diffs
// =============================================================================

/// Diff of tables within a namespace.
pub type TableDiff = Diff<TableName, Table, ModifiedTable>;

/// Diff of columns within a table.
pub type ColumnDiff = Diff<ColumnName, Column, ModifiedColumn>;

/// Diff of constraints within a table.
pub type ConstraintDiff = Diff<ConstraintName, Constraint, ModifiedConstraint>;

/// Diff of indexes within a table.
pub type IndexDiff = Diff<IndexName, Index, ModifiedIndex>;

/// Diff of views within a namespace.
pub type ViewDiff = Diff<TableName, View, ModifiedView>;

/// Diff of sequences within a namespace.
pub type SequenceDiff = Diff<SequenceName, Sequence, ModifiedSequence>;

/// Diff of enum types within a namespace.
pub type EnumDiff = Diff<TypeName, EnumType, ModifiedEnum>;

// =============================================================================
// Namespace Diff
// =============================================================================

/// The complete diff between two namespace snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamespaceDiff {
    /// The schema name being compared.
    pub name: SchemaName,
    /// Differences in tables.
    pub tables: TableDiff,
    /// Differences in views.
    pub views: ViewDiff,
    /// Differences in sequences.
    pub sequences: SequenceDiff,
    /// Differences in enum types.
    pub enums: EnumDiff,
    /// Change in schema comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl NamespaceDiff {
    /// Returns true if there are no differences.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
            && self.views.is_empty()
            && self.sequences.is_empty()
            && self.enums.is_empty()
            && self.comment.is_none()
    }
}

// =============================================================================
// Modified Table
// =============================================================================

/// Details of modifications to a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedTable {
    /// The table name.
    pub name: TableName,
    /// Change in table kind (regular vs partitioned).
    pub kind: Option<FieldChange<TableKind>>,
    /// Differences in columns.
    pub columns: ColumnDiff,
    /// Differences in constraints.
    pub constraints: ConstraintDiff,
    /// Differences in indexes.
    pub indexes: IndexDiff,
    /// Change in table comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl ModifiedTable {
    /// Returns true if there are no actual modifications.
    pub fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.columns.is_empty()
            && self.constraints.is_empty()
            && self.indexes.is_empty()
            && self.comment.is_none()
    }
}

// =============================================================================
// Modified Column
// =============================================================================

/// Details of modifications to a column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedColumn {
    /// The column name.
    pub name: ColumnName,
    /// Change in column position.
    pub position: Option<FieldChange<i16>>,
    /// Change in type information.
    pub type_info: Option<FieldChange<TypeInfo>>,
    /// Change in nullability.
    pub is_nullable: Option<FieldChange<bool>>,
    /// Change in default value.
    pub default: Option<FieldChange<Option<SqlExpr>>>,
    /// Change in generated column specification.
    pub generated: Option<FieldChange<Option<GeneratedColumn>>>,
    /// Change in identity specification.
    pub identity: Option<FieldChange<Option<IdentityKind>>>,
    /// Change in collation.
    pub collation: Option<FieldChange<QualifiedCollationName>>,
    /// Change in column comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl ModifiedColumn {
    /// Returns true if there are no actual modifications.
    pub fn is_empty(&self) -> bool {
        self.position.is_none()
            && self.type_info.is_none()
            && self.is_nullable.is_none()
            && self.default.is_none()
            && self.generated.is_none()
            && self.identity.is_none()
            && self.collation.is_none()
            && self.comment.is_none()
    }
}

// =============================================================================
// Modified Constraint
// =============================================================================

/// Details of modifications to a constraint.
///
/// Constraints are generally immutable in PostgreSQL - most changes require
/// dropping and recreating the constraint. This type captures when the
/// constraint definition has changed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedConstraint {
    /// The constraint name.
    pub name: ConstraintName,
    /// The source constraint.
    pub source: Constraint,
    /// The target constraint.
    pub target: Constraint,
    /// Change in constraint comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

// =============================================================================
// Modified Index
// =============================================================================

/// Details of modifications to an index.
///
/// Like constraints, indexes are generally recreated rather than altered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedIndex {
    /// The index name.
    pub name: IndexName,
    /// The source index.
    pub source: Index,
    /// The target index.
    pub target: Index,
    /// Change in index comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

// =============================================================================
// Modified View
// =============================================================================

/// Details of modifications to a view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedView {
    /// The view name.
    pub name: TableName,
    /// Change in view definition.
    pub definition: Option<FieldChange<SqlExpr>>,
    /// Change in materialized status.
    pub is_materialized: Option<FieldChange<bool>>,
    /// Change in view comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl ModifiedView {
    /// Returns true if there are no actual modifications.
    pub fn is_empty(&self) -> bool {
        self.definition.is_none()
            && self.is_materialized.is_none()
            && self.comment.is_none()
    }
}

// =============================================================================
// Modified Sequence
// =============================================================================

/// Details of modifications to a sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedSequence {
    /// The sequence name.
    pub name: SequenceName,
    /// Change in data type.
    pub data_type: Option<FieldChange<TypeInfo>>,
    /// Change in start value.
    pub start_value: Option<FieldChange<i64>>,
    /// Change in increment.
    pub increment: Option<FieldChange<i64>>,
    /// Change in minimum value.
    pub min_value: Option<FieldChange<i64>>,
    /// Change in maximum value.
    pub max_value: Option<FieldChange<i64>>,
    /// Change in cache size.
    pub cache_size: Option<FieldChange<i64>>,
    /// Change in cyclic behavior.
    pub is_cyclic: Option<FieldChange<bool>>,
    /// Change in sequence comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl ModifiedSequence {
    /// Returns true if there are no actual modifications.
    pub fn is_empty(&self) -> bool {
        self.data_type.is_none()
            && self.start_value.is_none()
            && self.increment.is_none()
            && self.min_value.is_none()
            && self.max_value.is_none()
            && self.cache_size.is_none()
            && self.is_cyclic.is_none()
            && self.comment.is_none()
    }
}

// =============================================================================
// Modified Enum
// =============================================================================

/// Details of modifications to an enum type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModifiedEnum {
    /// The enum type name.
    pub name: TypeName,
    /// Values that were added (in order of appearance in target).
    pub values_added: Vec<String>,
    /// Values that were removed.
    pub values_removed: Vec<String>,
    /// Values that were reordered (source order vs target order differs).
    /// This is significant because PostgreSQL doesn't support reordering enum values.
    pub values_reordered: bool,
    /// Change in enum comment.
    pub comment: Option<FieldChange<Option<Comment>>>,
}

impl ModifiedEnum {
    /// Returns true if there are no actual modifications.
    pub fn is_empty(&self) -> bool {
        self.values_added.is_empty()
            && self.values_removed.is_empty()
            && !self.values_reordered
            && self.comment.is_none()
    }
}
