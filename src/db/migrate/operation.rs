//! Semantic operations representing schema changes.
//!
//! Operations are extracted from diffs and represent individual schema
//! modifications. They are database-agnostic—the rendering layer handles
//! translation to specific SQL dialects.

use serde::{Deserialize, Serialize};

use crate::db::model::column::{GeneratedColumn, IdentityKind};
use crate::db::model::types::{SqlExpr, TypeInfo};
use crate::db::model::{Column, Constraint, EnumType, Index, Sequence, Table, View};
use crate::db::schema::{
    ColumnName, ConstraintName, IndexName, SchemaName, SequenceName, TableName, TypeName,
};

// =============================================================================
// Operation Identifier
// =============================================================================

/// A unique identifier for an operation within a migration plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub usize);

// =============================================================================
// Object Kind
// =============================================================================

/// The category of a schema object, used for dependency ordering.
///
/// The ordering of variants is significant: objects are created in this order
/// and dropped in reverse order to respect dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObjectKind {
    /// Enum types (must be created before tables that use them).
    Enum,
    /// Sequences (may be referenced by column defaults).
    Sequence,
    /// Tables (core schema objects).
    Table,
    /// Views (depend on tables).
    View,
    /// Indexes (depend on tables).
    Index,
    /// Constraints (depend on tables, may reference other tables).
    Constraint,
    /// Comments (depend on the object they describe).
    Comment,
}

// =============================================================================
// Operation Enum
// =============================================================================

/// A schema migration operation.
///
/// Each variant represents a single, atomic change to the database schema.
/// Operations are designed to be:
/// - **Serializable**: Can be stored for audit trails
/// - **Reversible**: Most operations have a natural inverse
/// - **Orderable**: Dependencies can be computed for safe execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    // =========================================================================
    // Enum Operations
    // =========================================================================
    /// Create a new enum type.
    CreateEnum {
        schema: SchemaName,
        enum_type: EnumType,
    },

    /// Drop an existing enum type.
    DropEnum { schema: SchemaName, name: TypeName },

    /// Rename an enum type.
    RenameEnum {
        schema: SchemaName,
        from: TypeName,
        to: TypeName,
    },

    /// Add a value to an existing enum.
    AddEnumValue {
        schema: SchemaName,
        enum_name: TypeName,
        value: String,
        position: EnumValuePosition,
    },

    // =========================================================================
    // Sequence Operations
    // =========================================================================
    /// Create a new sequence.
    CreateSequence {
        schema: SchemaName,
        sequence: Sequence,
    },

    /// Drop an existing sequence.
    DropSequence {
        schema: SchemaName,
        name: SequenceName,
    },

    /// Rename a sequence.
    RenameSequence {
        schema: SchemaName,
        from: SequenceName,
        to: SequenceName,
    },

    /// Alter sequence properties.
    AlterSequence {
        schema: SchemaName,
        name: SequenceName,
        changes: SequenceChanges,
    },

    // =========================================================================
    // Table Operations
    // =========================================================================
    /// Create a new table with columns (constraints added separately).
    CreateTable { schema: SchemaName, table: Table },

    /// Drop an existing table.
    DropTable { schema: SchemaName, name: TableName },

    /// Rename a table.
    RenameTable {
        schema: SchemaName,
        from: TableName,
        to: TableName,
    },

    // =========================================================================
    // Column Operations
    // =========================================================================
    /// Add a column to a table.
    AddColumn {
        schema: SchemaName,
        table: TableName,
        column: Column,
    },

    /// Drop a column from a table.
    DropColumn {
        schema: SchemaName,
        table: TableName,
        name: ColumnName,
    },

    /// Rename a column.
    RenameColumn {
        schema: SchemaName,
        table: TableName,
        from: ColumnName,
        to: ColumnName,
    },

    /// Alter column properties.
    AlterColumn {
        schema: SchemaName,
        table: TableName,
        name: ColumnName,
        changes: ColumnChanges,
    },

    // =========================================================================
    // Constraint Operations
    // =========================================================================
    /// Add a constraint to a table.
    AddConstraint {
        schema: SchemaName,
        table: TableName,
        constraint: Constraint,
    },

    /// Drop a constraint from a table.
    DropConstraint {
        schema: SchemaName,
        table: TableName,
        name: ConstraintName,
    },

    /// Rename a constraint.
    RenameConstraint {
        schema: SchemaName,
        table: TableName,
        from: ConstraintName,
        to: ConstraintName,
    },

    // =========================================================================
    // Index Operations
    // =========================================================================
    /// Create an index.
    CreateIndex {
        schema: SchemaName,
        table: TableName,
        index: Index,
        concurrently: bool,
    },

    /// Drop an index.
    DropIndex {
        schema: SchemaName,
        name: IndexName,
        concurrently: bool,
    },

    /// Rename an index.
    RenameIndex {
        schema: SchemaName,
        from: IndexName,
        to: IndexName,
    },

    // =========================================================================
    // View Operations
    // =========================================================================
    /// Create a view.
    CreateView { schema: SchemaName, view: View },

    /// Drop a view.
    DropView {
        schema: SchemaName,
        name: TableName,
        is_materialized: bool,
    },

    /// Rename a view.
    RenameView {
        schema: SchemaName,
        from: TableName,
        to: TableName,
        is_materialized: bool,
    },

    /// Replace a view definition (non-materialized only).
    ReplaceView { schema: SchemaName, view: View },

    /// Refresh a materialized view.
    RefreshMaterializedView {
        schema: SchemaName,
        name: TableName,
        concurrently: bool,
    },

    // =========================================================================
    // Comment Operations
    // =========================================================================
    /// Set or remove a comment on an object.
    SetComment {
        target: CommentTarget,
        comment: Option<String>,
    },
}

impl Operation {
    /// Returns the kind of object this operation affects.
    #[must_use]
    pub fn object_kind(&self) -> ObjectKind {
        match self {
            Self::CreateEnum { .. }
            | Self::DropEnum { .. }
            | Self::RenameEnum { .. }
            | Self::AddEnumValue { .. } => ObjectKind::Enum,

            Self::CreateSequence { .. }
            | Self::DropSequence { .. }
            | Self::RenameSequence { .. }
            | Self::AlterSequence { .. } => ObjectKind::Sequence,

            Self::CreateTable { .. }
            | Self::DropTable { .. }
            | Self::RenameTable { .. }
            | Self::AddColumn { .. }
            | Self::DropColumn { .. }
            | Self::RenameColumn { .. }
            | Self::AlterColumn { .. } => ObjectKind::Table,

            Self::CreateView { .. }
            | Self::DropView { .. }
            | Self::RenameView { .. }
            | Self::ReplaceView { .. }
            | Self::RefreshMaterializedView { .. } => ObjectKind::View,

            Self::CreateIndex { .. } | Self::DropIndex { .. } | Self::RenameIndex { .. } => {
                ObjectKind::Index
            }

            Self::AddConstraint { .. }
            | Self::DropConstraint { .. }
            | Self::RenameConstraint { .. } => ObjectKind::Constraint,

            Self::SetComment { .. } => ObjectKind::Comment,
        }
    }

    /// Returns a human-readable description of this operation.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            // Enum operations
            Self::CreateEnum { schema, enum_type } => {
                format!(
                    "Create enum type {}.{}",
                    schema.as_ref(),
                    enum_type.name.as_ref()
                )
            }
            Self::DropEnum { schema, name } => {
                format!("Drop enum type {}.{}", schema.as_ref(), name.as_ref())
            }
            Self::RenameEnum { schema, from, to } => {
                format!(
                    "Rename enum type {}.{} to {}",
                    schema.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }
            Self::AddEnumValue {
                schema,
                enum_name,
                value,
                ..
            } => {
                format!(
                    "Add value '{}' to enum {}.{}",
                    value,
                    schema.as_ref(),
                    enum_name.as_ref()
                )
            }

            // Sequence operations
            Self::CreateSequence { schema, sequence } => {
                format!(
                    "Create sequence {}.{}",
                    schema.as_ref(),
                    sequence.name.as_ref()
                )
            }
            Self::DropSequence { schema, name } => {
                format!("Drop sequence {}.{}", schema.as_ref(), name.as_ref())
            }
            Self::RenameSequence { schema, from, to } => {
                format!(
                    "Rename sequence {}.{} to {}",
                    schema.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }
            Self::AlterSequence { schema, name, .. } => {
                format!("Alter sequence {}.{}", schema.as_ref(), name.as_ref())
            }

            // Table operations
            Self::CreateTable { schema, table } => {
                format!("Create table {}.{}", schema.as_ref(), table.name.as_ref())
            }
            Self::DropTable { schema, name } => {
                format!("Drop table {}.{}", schema.as_ref(), name.as_ref())
            }
            Self::RenameTable { schema, from, to } => {
                format!(
                    "Rename table {}.{} to {}",
                    schema.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }

            // Column operations
            Self::AddColumn {
                schema,
                table,
                column,
            } => {
                format!(
                    "Add column {}.{}.{}",
                    schema.as_ref(),
                    table.as_ref(),
                    column.name.as_ref()
                )
            }
            Self::DropColumn {
                schema,
                table,
                name,
            } => {
                format!(
                    "Drop column {}.{}.{}",
                    schema.as_ref(),
                    table.as_ref(),
                    name.as_ref()
                )
            }
            Self::RenameColumn {
                schema,
                table,
                from,
                to,
            } => {
                format!(
                    "Rename column {}.{}.{} to {}",
                    schema.as_ref(),
                    table.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }
            Self::AlterColumn {
                schema,
                table,
                name,
                ..
            } => {
                format!(
                    "Alter column {}.{}.{}",
                    schema.as_ref(),
                    table.as_ref(),
                    name.as_ref()
                )
            }

            // Constraint operations
            Self::AddConstraint {
                schema,
                table,
                constraint,
            } => {
                format!(
                    "Add constraint {} on {}.{}",
                    constraint.name.as_ref(),
                    schema.as_ref(),
                    table.as_ref()
                )
            }
            Self::DropConstraint {
                schema,
                table,
                name,
            } => {
                format!(
                    "Drop constraint {} from {}.{}",
                    name.as_ref(),
                    schema.as_ref(),
                    table.as_ref()
                )
            }
            Self::RenameConstraint {
                schema,
                table,
                from,
                to,
            } => {
                format!(
                    "Rename constraint {}.{}.{} to {}",
                    schema.as_ref(),
                    table.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }

            // Index operations
            Self::CreateIndex {
                schema,
                table,
                index,
                concurrently,
            } => {
                let concurrent = if *concurrently { " concurrently" } else { "" };
                format!(
                    "Create index{} {} on {}.{}",
                    concurrent,
                    index.name.as_ref(),
                    schema.as_ref(),
                    table.as_ref()
                )
            }
            Self::DropIndex {
                schema,
                name,
                concurrently,
            } => {
                let concurrent = if *concurrently { " concurrently" } else { "" };
                format!(
                    "Drop index{} {}.{}",
                    concurrent,
                    schema.as_ref(),
                    name.as_ref()
                )
            }
            Self::RenameIndex { schema, from, to } => {
                format!(
                    "Rename index {}.{} to {}",
                    schema.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }

            // View operations
            Self::CreateView { schema, view } => {
                let kind = if view.is_materialized {
                    "materialized view"
                } else {
                    "view"
                };
                format!("Create {} {}.{}", kind, schema.as_ref(), view.name.as_ref())
            }
            Self::DropView {
                schema,
                name,
                is_materialized,
            } => {
                let kind = if *is_materialized {
                    "materialized view"
                } else {
                    "view"
                };
                format!("Drop {} {}.{}", kind, schema.as_ref(), name.as_ref())
            }
            Self::RenameView {
                schema,
                from,
                to,
                is_materialized,
            } => {
                let kind = if *is_materialized {
                    "materialized view"
                } else {
                    "view"
                };
                format!(
                    "Rename {} {}.{} to {}",
                    kind,
                    schema.as_ref(),
                    from.as_ref(),
                    to.as_ref()
                )
            }
            Self::ReplaceView { schema, view } => {
                format!("Replace view {}.{}", schema.as_ref(), view.name.as_ref())
            }
            Self::RefreshMaterializedView {
                schema,
                name,
                concurrently,
            } => {
                let concurrent = if *concurrently { " concurrently" } else { "" };
                format!(
                    "Refresh materialized view{} {}.{}",
                    concurrent,
                    schema.as_ref(),
                    name.as_ref()
                )
            }

            // Comment operations
            Self::SetComment { target, comment } => {
                let action = if comment.is_some() { "Set" } else { "Remove" };
                format!("{} comment on {}", action, target.description())
            }
        }
    }

    /// Returns true if this is a drop operation.
    #[must_use]
    pub fn is_drop(&self) -> bool {
        matches!(
            self,
            Self::DropEnum { .. }
                | Self::DropSequence { .. }
                | Self::DropTable { .. }
                | Self::DropColumn { .. }
                | Self::DropConstraint { .. }
                | Self::DropIndex { .. }
                | Self::DropView { .. }
        )
    }

    /// Returns true if this is a create operation.
    #[must_use]
    pub fn is_create(&self) -> bool {
        matches!(
            self,
            Self::CreateEnum { .. }
                | Self::CreateSequence { .. }
                | Self::CreateTable { .. }
                | Self::AddColumn { .. }
                | Self::AddConstraint { .. }
                | Self::CreateIndex { .. }
                | Self::CreateView { .. }
                | Self::AddEnumValue { .. }
        )
    }

    /// Returns true if this is a rename operation.
    #[must_use]
    pub fn is_rename(&self) -> bool {
        matches!(
            self,
            Self::RenameEnum { .. }
                | Self::RenameSequence { .. }
                | Self::RenameTable { .. }
                | Self::RenameColumn { .. }
                | Self::RenameConstraint { .. }
                | Self::RenameIndex { .. }
                | Self::RenameView { .. }
        )
    }
}

// =============================================================================
// Enum Value Position
// =============================================================================

/// Position for adding enum values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnumValuePosition {
    /// Add at the end (default).
    #[default]
    End,
    /// Add before an existing value.
    Before(String),
    /// Add after an existing value.
    After(String),
}

// =============================================================================
// Sequence Changes
// =============================================================================

/// Changes to apply to a sequence.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceChanges {
    /// New data type for the sequence.
    pub data_type: Option<TypeInfo>,
    /// New increment value.
    pub increment: Option<i64>,
    /// New minimum value.
    pub min_value: Option<i64>,
    /// New maximum value.
    pub max_value: Option<i64>,
    /// New start value.
    pub start_value: Option<i64>,
    /// New cache size.
    pub cache_size: Option<i64>,
    /// New cyclic behavior.
    pub is_cyclic: Option<bool>,
}

impl SequenceChanges {
    /// Returns true if no changes are specified.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data_type.is_none()
            && self.increment.is_none()
            && self.min_value.is_none()
            && self.max_value.is_none()
            && self.start_value.is_none()
            && self.cache_size.is_none()
            && self.is_cyclic.is_none()
    }
}

// =============================================================================
// Column Changes
// =============================================================================

/// Changes to apply to a column.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnChanges {
    /// Change the column type (with optional USING expression).
    pub set_type: Option<SetColumnType>,
    /// Set or drop NOT NULL.
    pub set_not_null: Option<bool>,
    /// Set a new default or drop the default.
    pub set_default: Option<DefaultChange>,
    /// Set or drop identity.
    pub set_identity: Option<IdentityChange>,
    /// Set or drop generated column.
    pub set_generated: Option<GeneratedChange>,
}

impl ColumnChanges {
    /// Returns true if no changes are specified.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.set_type.is_none()
            && self.set_not_null.is_none()
            && self.set_default.is_none()
            && self.set_identity.is_none()
            && self.set_generated.is_none()
    }
}

/// Type change with optional USING clause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetColumnType {
    /// The new type information.
    pub type_info: TypeInfo,
    /// Optional expression for type conversion (e.g., `column::new_type`).
    pub using: Option<SqlExpr>,
}

/// Default value change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefaultChange {
    /// Set a new default expression.
    Set(SqlExpr),
    /// Drop the default.
    Drop,
}

/// Identity column change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityChange {
    /// Add identity (ALWAYS or BY DEFAULT).
    Add(IdentityKind),
    /// Drop identity.
    Drop,
}

/// Generated column change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeneratedChange {
    /// Set as generated column.
    Set(GeneratedColumn),
    /// Drop generated expression.
    Drop,
}

// =============================================================================
// Comment Target
// =============================================================================

/// Target for a COMMENT ON statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentTarget {
    /// Comment on a schema.
    Schema(SchemaName),
    /// Comment on a table.
    Table {
        schema: SchemaName,
        table: TableName,
    },
    /// Comment on a column.
    Column {
        schema: SchemaName,
        table: TableName,
        column: ColumnName,
    },
    /// Comment on an index.
    Index {
        schema: SchemaName,
        index: IndexName,
    },
    /// Comment on a constraint.
    Constraint {
        schema: SchemaName,
        table: TableName,
        constraint: ConstraintName,
    },
    /// Comment on a sequence.
    Sequence {
        schema: SchemaName,
        sequence: SequenceName,
    },
    /// Comment on a type (including enums).
    Type {
        schema: SchemaName,
        type_name: TypeName,
    },
    /// Comment on a view.
    View { schema: SchemaName, view: TableName },
}

impl CommentTarget {
    /// Returns a human-readable description of the comment target.
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::Schema(name) => format!("schema {}", name.as_ref()),
            Self::Table { schema, table } => {
                format!("table {}.{}", schema.as_ref(), table.as_ref())
            }
            Self::Column {
                schema,
                table,
                column,
            } => {
                format!(
                    "column {}.{}.{}",
                    schema.as_ref(),
                    table.as_ref(),
                    column.as_ref()
                )
            }
            Self::Index { schema, index } => {
                format!("index {}.{}", schema.as_ref(), index.as_ref())
            }
            Self::Constraint {
                schema,
                table,
                constraint,
            } => {
                format!(
                    "constraint {} on {}.{}",
                    constraint.as_ref(),
                    schema.as_ref(),
                    table.as_ref()
                )
            }
            Self::Sequence { schema, sequence } => {
                format!("sequence {}.{}", schema.as_ref(), sequence.as_ref())
            }
            Self::Type { schema, type_name } => {
                format!("type {}.{}", schema.as_ref(), type_name.as_ref())
            }
            Self::View { schema, view } => {
                format!("view {}.{}", schema.as_ref(), view.as_ref())
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod object_kind_tests {
        use super::*;

        #[test]
        fn ordering_is_correct() {
            // Enums should come before tables
            assert!(ObjectKind::Enum < ObjectKind::Table);
            // Tables should come before views
            assert!(ObjectKind::Table < ObjectKind::View);
            // Views should come before indexes
            assert!(ObjectKind::View < ObjectKind::Index);
            // Indexes should come before constraints
            assert!(ObjectKind::Index < ObjectKind::Constraint);
            // Comments should come last
            assert!(ObjectKind::Constraint < ObjectKind::Comment);
        }
    }

    mod column_changes_tests {
        use super::*;

        #[test]
        fn is_empty_when_default() {
            let changes = ColumnChanges::default();
            assert!(changes.is_empty());
        }

        #[test]
        fn is_not_empty_with_not_null() {
            let changes = ColumnChanges {
                set_not_null: Some(true),
                ..Default::default()
            };
            assert!(!changes.is_empty());
        }
    }

    mod sequence_changes_tests {
        use super::*;

        #[test]
        fn is_empty_when_default() {
            let changes = SequenceChanges::default();
            assert!(changes.is_empty());
        }

        #[test]
        fn is_not_empty_with_increment() {
            let changes = SequenceChanges {
                increment: Some(5),
                ..Default::default()
            };
            assert!(!changes.is_empty());
        }
    }
}
