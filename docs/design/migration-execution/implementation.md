# Migration Execution: Hybrid Pipeline Implementation Plan

## Overview

This design introduces a three-layer architecture for converting schema diffs into executable SQL:

```
NamespaceDiff → [Collector] → Vec<Operation> → [Renderer] → MigrationScript → SQL
```

Each layer has a single responsibility:
1. **Collector**: Extracts semantic operations from the diff structure
2. **Renderer**: Converts operations to database-specific SQL
3. **Script**: Aggregates rendered operations for execution

---

## 1. Module Structure

```
src/db/
├── migrate/                    # New module
│   ├── mod.rs                  # Re-exports, module documentation
│   ├── operation.rs            # Operation enum and associated types
│   ├── collector.rs            # Diff → Operations conversion
│   ├── render/
│   │   ├── mod.rs              # Renderer trait and re-exports
│   │   ├── postgres.rs         # PostgreSQL-specific rendering
│   │   └── identifier.rs       # Identifier quoting utilities
│   ├── plan.rs                 # MigrationPlan (ordered operations)
│   ├── script.rs               # MigrationScript (rendered output)
│   ├── ordering.rs             # Dependency analysis and topological sort
│   └── error.rs                # Migration-specific errors
├── diff/                       # Existing
├── model/                      # Existing
├── query/                      # Existing
└── schema.rs                   # Existing
```

---

## 2. Core Types

### 2.1 Operations (`operation.rs`)

Operations are database-agnostic representations of schema changes. They capture *what* changed, not *how* to apply it.

```rust
//! Semantic operations representing schema changes.
//!
//! Operations are extracted from diffs and represent individual schema
//! modifications. They are database-agnostic—the rendering layer handles
//! translation to specific SQL dialects.

use serde::{Deserialize, Serialize};

use crate::db::model::{
    Column, Constraint, EnumType, GeneratedColumn, Index, Sequence, Table, View,
};
use crate::db::model::types::{
    ForeignKeyAction, IndexMethod, SqlExpr, TypeInfo, QualifiedTableName,
};
use crate::db::schema::{
    ColumnName, ConstraintName, IndexName, SchemaName, SequenceName, TableName, TypeName,
};

/// A unique identifier for an operation within a migration plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OperationId(pub usize);

/// The category of a schema object, used for dependency ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ObjectKind {
    /// Enum types (must be created before tables that use them)
    Enum,
    /// Sequences (may be referenced by column defaults)
    Sequence,
    /// Tables (core schema objects)
    Table,
    /// Views (depend on tables)
    View,
    /// Indexes (depend on tables)
    Index,
    /// Constraints (depend on tables, may reference other tables)
    Constraint,
    /// Comments (depend on the object they describe)
    Comment,
}

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
    DropEnum {
        schema: SchemaName,
        name: TypeName,
    },

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
    CreateTable {
        schema: SchemaName,
        table: Table,
    },

    /// Drop an existing table.
    DropTable {
        schema: SchemaName,
        name: TableName,
    },

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
    CreateView {
        schema: SchemaName,
        view: View,
    },

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
    ReplaceView {
        schema: SchemaName,
        view: View,
    },

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

/// Position for adding enum values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EnumValuePosition {
    /// Add at the end (default).
    End,
    /// Add before an existing value.
    Before(String),
    /// Add after an existing value.
    After(String),
}

/// Changes to apply to a sequence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SequenceChanges {
    pub data_type: Option<TypeInfo>,
    pub increment: Option<i64>,
    pub min_value: Option<i64>,
    pub max_value: Option<i64>,
    pub start_value: Option<i64>,
    pub cache_size: Option<i64>,
    pub is_cyclic: Option<bool>,
}

/// Changes to apply to a column.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    pub fn is_empty(&self) -> bool {
        self.set_type.is_none()
            && self.set_not_null.is_none()
            && self.set_default.is_none()
            && self.set_identity.is_none()
            && self.set_generated.is_none()
    }
}

/// Type change with optional USING clause.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetColumnType {
    pub type_info: TypeInfo,
    /// Optional expression for type conversion (e.g., `column::new_type`).
    pub using: Option<SqlExpr>,
}

/// Default value change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DefaultChange {
    /// Set a new default expression.
    Set(SqlExpr),
    /// Drop the default.
    Drop,
}

/// Identity column change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IdentityChange {
    /// Add identity (ALWAYS or BY DEFAULT).
    Add(crate::db::model::column::IdentityKind),
    /// Drop identity.
    Drop,
}

/// Generated column change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeneratedChange {
    /// Set as generated column.
    Set(GeneratedColumn),
    /// Drop generated expression.
    Drop,
}

/// Target for a COMMENT ON statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommentTarget {
    Schema(SchemaName),
    Table { schema: SchemaName, table: TableName },
    Column { schema: SchemaName, table: TableName, column: ColumnName },
    Index { schema: SchemaName, index: IndexName },
    Constraint { schema: SchemaName, table: TableName, constraint: ConstraintName },
    Sequence { schema: SchemaName, sequence: SequenceName },
    Type { schema: SchemaName, type_name: TypeName },
    View { schema: SchemaName, view: TableName },
}

impl Operation {
    /// Returns the kind of object this operation affects.
    pub fn object_kind(&self) -> ObjectKind {
        match self {
            Self::CreateEnum { .. } | Self::DropEnum { .. }
            | Self::RenameEnum { .. } | Self::AddEnumValue { .. } => ObjectKind::Enum,

            Self::CreateSequence { .. } | Self::DropSequence { .. }
            | Self::RenameSequence { .. } | Self::AlterSequence { .. } => ObjectKind::Sequence,

            Self::CreateTable { .. } | Self::DropTable { .. }
            | Self::RenameTable { .. } | Self::AddColumn { .. }
            | Self::DropColumn { .. } | Self::RenameColumn { .. }
            | Self::AlterColumn { .. } => ObjectKind::Table,

            Self::CreateView { .. } | Self::DropView { .. }
            | Self::RenameView { .. } | Self::ReplaceView { .. }
            | Self::RefreshMaterializedView { .. } => ObjectKind::View,

            Self::CreateIndex { .. } | Self::DropIndex { .. }
            | Self::RenameIndex { .. } => ObjectKind::Index,

            Self::AddConstraint { .. } | Self::DropConstraint { .. }
            | Self::RenameConstraint { .. } => ObjectKind::Constraint,

            Self::SetComment { .. } => ObjectKind::Comment,
        }
    }

    /// Returns a human-readable description of this operation.
    pub fn description(&self) -> String {
        match self {
            Self::CreateTable { schema, table } => {
                format!("Create table {}.{}", schema.as_ref(), table.name.as_ref())
            }
            Self::DropTable { schema, name } => {
                format!("Drop table {}.{}", schema.as_ref(), name.as_ref())
            }
            Self::RenameTable { schema, from, to } => {
                format!("Rename table {}.{} to {}", schema.as_ref(), from.as_ref(), to.as_ref())
            }
            Self::AddColumn { schema, table, column } => {
                format!("Add column {}.{}.{}", schema.as_ref(), table.as_ref(), column.name.as_ref())
            }
            // ... similar for other operations
            _ => format!("{:?}", std::mem::discriminant(self)),
        }
    }
}
```

### 2.2 Migration Plan (`plan.rs`)

```rust
//! Migration plan containing ordered operations.

use serde::{Deserialize, Serialize};

use super::operation::{Operation, OperationId};
use super::ordering::DependencyGraph;
use crate::db::diff::NamespaceDiff;

/// Configuration for migration plan generation.
#[derive(Debug, Clone, Default)]
pub struct PlanConfig {
    /// Use CONCURRENTLY for index operations when possible.
    pub concurrent_indexes: bool,
    /// Include operations to refresh materialized views.
    pub refresh_materialized_views: bool,
    /// Include comment operations.
    pub include_comments: bool,
}

/// A migration plan containing operations in execution order.
///
/// Operations are ordered to respect dependencies:
/// 1. Enums (before tables that use them)
/// 2. Sequences (before columns with defaults referencing them)
/// 3. Tables (before constraints, indexes, views)
/// 4. Constraints (foreign keys after referenced tables)
/// 5. Indexes
/// 6. Views
/// 7. Comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    /// Operations in execution order.
    pub operations: Vec<Operation>,
}

impl MigrationPlan {
    /// Create a migration plan from a namespace diff.
    pub fn from_diff(diff: &NamespaceDiff) -> Self {
        Self::from_diff_with_config(diff, &PlanConfig::default())
    }

    /// Create a migration plan with custom configuration.
    pub fn from_diff_with_config(diff: &NamespaceDiff, config: &PlanConfig) -> Self {
        let collector = super::collector::OperationCollector::new(config);
        let operations = collector.collect(diff);
        let ordered = super::ordering::topological_sort(operations);
        Self { operations: ordered }
    }

    /// Returns true if the plan has no operations.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns the number of operations.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Iterate over operations with their IDs.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (OperationId, &Operation)> {
        self.operations
            .iter()
            .enumerate()
            .map(|(i, op)| (OperationId(i), op))
    }
}
```

### 2.3 Rendering (`render/mod.rs`)

```rust
//! SQL rendering for migration operations.

mod identifier;
mod postgres;

pub use identifier::IdentifierQuoting;
pub use postgres::PostgresRenderer;

use crate::db::migrate::operation::Operation;

/// Rendered SQL for a single operation.
#[derive(Debug, Clone)]
pub struct RenderedOperation {
    /// Forward migration SQL statements.
    pub forward: Vec<String>,
    /// Rollback SQL statements (in reverse order of forward).
    pub rollback: Option<Vec<String>>,
    /// Human-readable description.
    pub description: String,
}

/// Configuration for SQL rendering.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// How to quote identifiers.
    pub quoting: IdentifierQuoting,
    /// Add IF EXISTS to DROP statements.
    pub if_exists: bool,
    /// Add IF NOT EXISTS to CREATE statements.
    pub if_not_exists: bool,
    /// Add CASCADE to DROP statements.
    pub cascade: bool,
    /// Generate rollback SQL.
    pub generate_rollback: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            quoting: IdentifierQuoting::WhenNeeded,
            if_exists: false,
            if_not_exists: false,
            cascade: false,
            generate_rollback: true,
        }
    }
}

/// Trait for rendering operations to SQL.
pub trait Renderer {
    /// Render a single operation to SQL.
    fn render(&self, operation: &Operation) -> RenderedOperation;

    /// Render multiple operations.
    fn render_all(&self, operations: &[Operation]) -> Vec<RenderedOperation> {
        operations.iter().map(|op| self.render(op)).collect()
    }
}
```

### 2.4 PostgreSQL Renderer (`render/postgres.rs`)

```rust
//! PostgreSQL-specific SQL rendering.

use super::{RenderedOperation, RenderConfig, Renderer};
use crate::db::migrate::operation::*;
use crate::db::model::constraint::{Constraint, ConstraintKind};
use crate::db::model::column::Column;
use crate::db::model::index::{Index, IndexColumn};

/// PostgreSQL SQL renderer.
pub struct PostgresRenderer {
    config: RenderConfig,
}

impl PostgresRenderer {
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Quote an identifier if needed.
    fn quote(&self, name: &str) -> String {
        self.config.quoting.quote(name)
    }

    /// Format a qualified name (schema.object).
    fn qualified(&self, schema: &str, name: &str) -> String {
        format!("{}.{}", self.quote(schema), self.quote(name))
    }

    // =========================================================================
    // Table Rendering
    // =========================================================================

    fn render_create_table(&self, schema: &SchemaName, table: &Table) -> RenderedOperation {
        let mut sql = String::new();

        // CREATE TABLE
        sql.push_str("CREATE TABLE ");
        if self.config.if_not_exists {
            sql.push_str("IF NOT EXISTS ");
        }
        sql.push_str(&self.qualified(schema.as_ref(), table.name.as_ref()));
        sql.push_str(" (\n");

        // Columns
        let column_defs: Vec<String> = table.columns
            .iter()
            .map(|col| self.render_column_definition(col))
            .collect();
        sql.push_str(&column_defs.join(",\n"));

        // Inline constraints (PK, UNIQUE, CHECK - not FK)
        for constraint in &table.constraints {
            if let Some(inline) = self.render_inline_constraint(constraint) {
                sql.push_str(",\n");
                sql.push_str(&inline);
            }
        }

        sql.push_str("\n)");

        // Rollback
        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "DROP TABLE {}{}",
                self.qualified(schema.as_ref(), table.name.as_ref()),
                if self.config.cascade { " CASCADE" } else { "" }
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!("Create table {}.{}", schema.as_ref(), table.name.as_ref()),
        }
    }

    fn render_column_definition(&self, column: &Column) -> String {
        let mut def = format!(
            "    {} {}",
            self.quote(column.name.as_ref()),
            column.type_info.formatted
        );

        // Collation
        if column.collation.name.as_ref() != "default" {
            def.push_str(&format!(" COLLATE {}",
                self.qualified(column.collation.schema.as_ref(), column.collation.name.as_ref())));
        }

        // NOT NULL
        if !column.is_nullable {
            def.push_str(" NOT NULL");
        }

        // Default
        if let Some(ref default) = column.default {
            def.push_str(&format!(" DEFAULT {}", default.as_ref()));
        }

        // Generated
        if let Some(ref gen) = column.generated {
            def.push_str(&format!(" GENERATED ALWAYS AS ({}) STORED", gen.expression.as_ref()));
        }

        // Identity
        if let Some(ref identity) = column.identity {
            match identity {
                IdentityKind::Always => def.push_str(" GENERATED ALWAYS AS IDENTITY"),
                IdentityKind::ByDefault => def.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
            }
        }

        def
    }

    fn render_inline_constraint(&self, constraint: &Constraint) -> Option<String> {
        match &constraint.kind {
            ConstraintKind::PrimaryKey(pk) => {
                let cols: Vec<_> = pk.columns.iter()
                    .map(|c| self.quote(c.as_ref()))
                    .collect();
                Some(format!(
                    "    CONSTRAINT {} PRIMARY KEY ({})",
                    self.quote(constraint.name.as_ref()),
                    cols.join(", ")
                ))
            }
            ConstraintKind::Unique(uq) => {
                let cols: Vec<_> = uq.columns.iter()
                    .map(|c| self.quote(c.as_ref()))
                    .collect();
                let nulls = if uq.nulls_not_distinct { " NULLS NOT DISTINCT" } else { "" };
                Some(format!(
                    "    CONSTRAINT {} UNIQUE{} ({})",
                    self.quote(constraint.name.as_ref()),
                    nulls,
                    cols.join(", ")
                ))
            }
            ConstraintKind::Check(ck) => {
                let no_inherit = if ck.is_no_inherit { " NO INHERIT" } else { "" };
                Some(format!(
                    "    CONSTRAINT {} CHECK ({}){}",
                    self.quote(constraint.name.as_ref()),
                    ck.expression.as_ref(),
                    no_inherit
                ))
            }
            // FK and Exclusion are added separately via ALTER TABLE
            ConstraintKind::ForeignKey(_) | ConstraintKind::Exclusion(_) => None,
        }
    }

    // =========================================================================
    // Column Rendering
    // =========================================================================

    fn render_add_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        column: &Column,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER TABLE {} ADD COLUMN {}",
            self.qualified(schema.as_ref(), table.as_ref()),
            self.render_column_definition(column).trim()
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.qualified(schema.as_ref(), table.as_ref()),
                self.quote(column.name.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Add column {}.{}.{}",
                schema.as_ref(), table.as_ref(), column.name.as_ref()
            ),
        }
    }

    fn render_alter_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        name: &ColumnName,
        changes: &ColumnChanges,
    ) -> RenderedOperation {
        let mut forward = Vec::new();
        let mut rollback = Vec::new();
        let table_name = self.qualified(schema.as_ref(), table.as_ref());
        let col_name = self.quote(name.as_ref());

        // Type change
        if let Some(ref type_change) = changes.set_type {
            let using = type_change.using.as_ref()
                .map(|u| format!(" USING {}", u.as_ref()))
                .unwrap_or_default();
            forward.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} TYPE {}{}",
                table_name, col_name, type_change.type_info.formatted, using
            ));
            // Note: rollback for type change requires knowing the old type
        }

        // NOT NULL change
        if let Some(not_null) = changes.set_not_null {
            if not_null {
                forward.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                    table_name, col_name
                ));
                rollback.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                    table_name, col_name
                ));
            } else {
                forward.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                    table_name, col_name
                ));
                rollback.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                    table_name, col_name
                ));
            }
        }

        // Default change
        if let Some(ref default_change) = changes.set_default {
            match default_change {
                DefaultChange::Set(expr) => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                        table_name, col_name, expr.as_ref()
                    ));
                    rollback.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                        table_name, col_name
                    ));
                }
                DefaultChange::Drop => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                        table_name, col_name
                    ));
                    // Rollback requires knowing old default
                }
            }
        }

        // Identity change
        if let Some(ref identity_change) = changes.set_identity {
            match identity_change {
                IdentityChange::Add(kind) => {
                    let kind_sql = match kind {
                        IdentityKind::Always => "ALWAYS",
                        IdentityKind::ByDefault => "BY DEFAULT",
                    };
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} ADD GENERATED {} AS IDENTITY",
                        table_name, col_name, kind_sql
                    ));
                    rollback.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP IDENTITY",
                        table_name, col_name
                    ));
                }
                IdentityChange::Drop => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP IDENTITY",
                        table_name, col_name
                    ));
                }
            }
        }

        RenderedOperation {
            forward,
            rollback: if self.config.generate_rollback && !rollback.is_empty() {
                Some(rollback)
            } else {
                None
            },
            description: format!(
                "Alter column {}.{}.{}",
                schema.as_ref(), table.as_ref(), name.as_ref()
            ),
        }
    }

    // ... similar methods for other operations
}

impl Renderer for PostgresRenderer {
    fn render(&self, operation: &Operation) -> RenderedOperation {
        match operation {
            Operation::CreateTable { schema, table } => {
                self.render_create_table(schema, table)
            }
            Operation::DropTable { schema, name } => {
                self.render_drop_table(schema, name)
            }
            Operation::RenameTable { schema, from, to } => {
                self.render_rename_table(schema, from, to)
            }
            Operation::AddColumn { schema, table, column } => {
                self.render_add_column(schema, table, column)
            }
            Operation::DropColumn { schema, table, name } => {
                self.render_drop_column(schema, table, name)
            }
            Operation::RenameColumn { schema, table, from, to } => {
                self.render_rename_column(schema, table, from, to)
            }
            Operation::AlterColumn { schema, table, name, changes } => {
                self.render_alter_column(schema, table, name, changes)
            }
            // ... other operations
        }
    }
}
```

### 2.5 Operation Collector (`collector.rs`)

```rust
//! Extracts operations from namespace diffs.

use crate::db::diff::{
    ColumnDiff, ConstraintDiff, EnumDiff, IndexDiff, ModifiedColumn, ModifiedEnum,
    ModifiedSequence, ModifiedTable, ModifiedView, NamespaceDiff, SequenceDiff,
    TableDiff, ViewDiff,
};
use crate::db::migrate::operation::*;
use crate::db::migrate::plan::PlanConfig;
use crate::db::schema::SchemaName;

/// Collects operations from a namespace diff.
pub struct OperationCollector<'a> {
    config: &'a PlanConfig,
    schema: Option<SchemaName>,
    operations: Vec<Operation>,
}

impl<'a> OperationCollector<'a> {
    pub fn new(config: &'a PlanConfig) -> Self {
        Self {
            config,
            schema: None,
            operations: Vec::new(),
        }
    }

    /// Collect all operations from a namespace diff.
    pub fn collect(mut self, diff: &NamespaceDiff) -> Vec<Operation> {
        self.schema = Some(diff.name.clone());

        // Order matters here for dependency correctness:
        // 1. Drop dependent objects first (reverse order)
        self.collect_drops(diff);

        // 2. Create/modify independent objects
        self.collect_enums(&diff.enums);
        self.collect_sequences(&diff.sequences);

        // 3. Create/modify tables
        self.collect_tables(&diff.tables);

        // 4. Create/modify views (depend on tables)
        self.collect_views(&diff.views);

        // 5. Comments last
        if self.config.include_comments {
            self.collect_comments(diff);
        }

        self.operations
    }

    fn schema(&self) -> &SchemaName {
        self.schema.as_ref().expect("schema not set")
    }

    // =========================================================================
    // Drop Collection (reverse dependency order)
    // =========================================================================

    fn collect_drops(&mut self, diff: &NamespaceDiff) {
        // Drop views first (depend on tables)
        for name in &diff.views.removed {
            self.operations.push(Operation::DropView {
                schema: self.schema().clone(),
                name: name.clone(),
                is_materialized: false, // Would need to track this
            });
        }

        // Drop table indexes and constraints
        for modified in &diff.tables.modified {
            self.collect_table_drops(modified);
        }

        // Drop tables
        for name in &diff.tables.removed {
            self.operations.push(Operation::DropTable {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }

        // Drop sequences
        for name in &diff.sequences.removed {
            self.operations.push(Operation::DropSequence {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }

        // Drop enums last (tables may depend on them)
        for name in &diff.enums.removed {
            self.operations.push(Operation::DropEnum {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }
    }

    fn collect_table_drops(&mut self, modified: &ModifiedTable) {
        // Drop indexes
        for name in &modified.indexes.removed {
            self.operations.push(Operation::DropIndex {
                schema: self.schema().clone(),
                name: name.clone(),
                concurrently: self.config.concurrent_indexes,
            });
        }

        // Drop constraints
        for name in &modified.constraints.removed {
            self.operations.push(Operation::DropConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                name: name.clone(),
            });
        }

        // Drop columns
        for name in &modified.columns.removed {
            self.operations.push(Operation::DropColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                name: name.clone(),
            });
        }
    }

    // =========================================================================
    // Enum Collection
    // =========================================================================

    fn collect_enums(&mut self, diff: &EnumDiff) {
        // Handle renames
        for rename in &diff.potential_renames {
            self.operations.push(Operation::RenameEnum {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
            // Check for value changes after rename
            // (would need source enum to compare)
        }

        // Create new enums
        for enum_type in &diff.added {
            self.operations.push(Operation::CreateEnum {
                schema: self.schema().clone(),
                enum_type: enum_type.clone(),
            });
        }

        // Modify existing enums (add values)
        for modified in &diff.modified {
            self.collect_enum_modifications(modified);
        }
    }

    fn collect_enum_modifications(&mut self, modified: &ModifiedEnum) {
        // PostgreSQL only allows adding enum values, not removing
        // New values must be added one at a time
        for value in &modified.added_values {
            self.operations.push(Operation::AddEnumValue {
                schema: self.schema().clone(),
                enum_name: modified.name.clone(),
                value: value.clone(),
                position: EnumValuePosition::End,
            });
        }
    }

    // =========================================================================
    // Table Collection
    // =========================================================================

    fn collect_tables(&mut self, diff: &TableDiff) {
        // Handle renames
        for rename in &diff.potential_renames {
            self.operations.push(Operation::RenameTable {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Create new tables
        for table in &diff.added {
            self.operations.push(Operation::CreateTable {
                schema: self.schema().clone(),
                table: table.clone(),
            });

            // Add non-inline constraints (FK, exclusion)
            for constraint in &table.constraints {
                if self.is_separate_constraint(&constraint.kind) {
                    self.operations.push(Operation::AddConstraint {
                        schema: self.schema().clone(),
                        table: table.name.clone(),
                        constraint: constraint.clone(),
                    });
                }
            }

            // Add indexes (non-constraint-backing)
            for index in &table.indexes {
                if !index.is_constraint_index {
                    self.operations.push(Operation::CreateIndex {
                        schema: self.schema().clone(),
                        table: table.name.clone(),
                        index: index.clone(),
                        concurrently: self.config.concurrent_indexes,
                    });
                }
            }
        }

        // Modify existing tables
        for modified in &diff.modified {
            self.collect_table_modifications(modified);
        }
    }

    fn collect_table_modifications(&mut self, modified: &ModifiedTable) {
        // Handle column renames
        for rename in &modified.columns.potential_renames {
            self.operations.push(Operation::RenameColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Add new columns
        for column in &modified.columns.added {
            self.operations.push(Operation::AddColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                column: column.clone(),
            });
        }

        // Modify columns
        for mod_col in &modified.columns.modified {
            if let Some(changes) = self.build_column_changes(mod_col) {
                self.operations.push(Operation::AlterColumn {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                    name: mod_col.name.clone(),
                    changes,
                });
            }
        }

        // Add new constraints
        for constraint in &modified.constraints.added {
            self.operations.push(Operation::AddConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                constraint: constraint.clone(),
            });
        }

        // Add new indexes
        for index in &modified.indexes.added {
            if !index.is_constraint_index {
                self.operations.push(Operation::CreateIndex {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                    index: index.clone(),
                    concurrently: self.config.concurrent_indexes,
                });
            }
        }
    }

    fn build_column_changes(&self, modified: &ModifiedColumn) -> Option<ColumnChanges> {
        let mut changes = ColumnChanges::default();

        if let Some(ref type_change) = modified.type_info {
            changes.set_type = Some(SetColumnType {
                type_info: type_change.target.clone(),
                using: None, // Could be enhanced to suggest USING clause
            });
        }

        if let Some(ref null_change) = modified.is_nullable {
            changes.set_not_null = Some(!null_change.target);
        }

        if let Some(ref default_change) = modified.default {
            changes.set_default = Some(match &default_change.target {
                Some(expr) => DefaultChange::Set(expr.clone()),
                None => DefaultChange::Drop,
            });
        }

        if let Some(ref identity_change) = modified.identity {
            changes.set_identity = Some(match &identity_change.target {
                Some(kind) => IdentityChange::Add(kind.clone()),
                None => IdentityChange::Drop,
            });
        }

        if changes.is_empty() {
            None
        } else {
            Some(changes)
        }
    }

    fn is_separate_constraint(&self, kind: &ConstraintKind) -> bool {
        matches!(kind, ConstraintKind::ForeignKey(_) | ConstraintKind::Exclusion(_))
    }

    // ... similar methods for sequences, views, comments
}
```

### 2.6 Migration Script (`script.rs`)

```rust
//! Final rendered migration output.

use super::render::RenderedOperation;

/// A complete migration script ready for execution.
#[derive(Debug, Clone)]
pub struct MigrationScript {
    /// Rendered operations in order.
    pub operations: Vec<RenderedOperation>,
}

impl MigrationScript {
    /// Create from rendered operations.
    pub fn new(operations: Vec<RenderedOperation>) -> Self {
        Self { operations }
    }

    /// Generate the forward migration SQL.
    pub fn to_sql(&self) -> String {
        self.to_sql_with_options(SqlOptions::default())
    }

    /// Generate forward SQL with options.
    pub fn to_sql_with_options(&self, options: SqlOptions) -> String {
        let mut sql = String::new();

        if options.include_transaction {
            sql.push_str("BEGIN;\n\n");
        }

        for (i, op) in self.operations.iter().enumerate() {
            if options.include_comments {
                sql.push_str(&format!("-- {}\n", op.description));
            }

            for stmt in &op.forward {
                sql.push_str(stmt);
                sql.push_str(";\n");
            }

            if i < self.operations.len() - 1 {
                sql.push('\n');
            }
        }

        if options.include_transaction {
            sql.push_str("\nCOMMIT;\n");
        }

        sql
    }

    /// Generate the rollback SQL (operations in reverse order).
    pub fn to_rollback_sql(&self) -> Option<String> {
        self.to_rollback_sql_with_options(SqlOptions::default())
    }

    /// Generate rollback SQL with options.
    pub fn to_rollback_sql_with_options(&self, options: SqlOptions) -> Option<String> {
        let mut sql = String::new();
        let mut has_any = false;

        if options.include_transaction {
            sql.push_str("BEGIN;\n\n");
        }

        // Reverse order
        for op in self.operations.iter().rev() {
            if let Some(ref rollback) = op.rollback {
                has_any = true;
                if options.include_comments {
                    sql.push_str(&format!("-- Rollback: {}\n", op.description));
                }
                for stmt in rollback {
                    sql.push_str(stmt);
                    sql.push_str(";\n");
                }
                sql.push('\n');
            }
        }

        if options.include_transaction {
            sql.push_str("COMMIT;\n");
        }

        if has_any { Some(sql) } else { None }
    }

    /// Get all forward statements as a flat list.
    pub fn all_statements(&self) -> Vec<&str> {
        self.operations
            .iter()
            .flat_map(|op| op.forward.iter().map(String::as_str))
            .collect()
    }

    /// Get operation descriptions.
    pub fn descriptions(&self) -> Vec<&str> {
        self.operations.iter().map(|op| op.description.as_str()).collect()
    }
}

/// Options for SQL generation.
#[derive(Debug, Clone)]
pub struct SqlOptions {
    /// Wrap in BEGIN/COMMIT transaction.
    pub include_transaction: bool,
    /// Include comment descriptions.
    pub include_comments: bool,
}

impl Default for SqlOptions {
    fn default() -> Self {
        Self {
            include_transaction: true,
            include_comments: true,
        }
    }
}
```

### 2.7 Dependency Ordering (`ordering.rs`)

```rust
//! Topological sorting of operations based on dependencies.

use std::collections::{HashMap, HashSet, VecDeque};

use super::operation::{ObjectKind, Operation};
use crate::db::model::constraint::ConstraintKind;
use crate::db::schema::{SchemaName, TableName};

/// Object reference for dependency tracking.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ObjectRef {
    Enum(SchemaName, TypeName),
    Sequence(SchemaName, SequenceName),
    Table(SchemaName, TableName),
    View(SchemaName, TableName),
}

/// Sort operations to respect dependencies.
///
/// The algorithm:
/// 1. Group operations by phase (drops, creates, alters)
/// 2. Within each phase, sort by object kind priority
/// 3. For creates, ensure referenced objects come first
/// 4. For drops, ensure dependent objects come first (reverse)
pub fn topological_sort(operations: Vec<Operation>) -> Vec<Operation> {
    let mut drops = Vec::new();
    let mut creates = Vec::new();
    let mut alters = Vec::new();
    let mut renames = Vec::new();
    let mut comments = Vec::new();

    // Partition by operation type
    for op in operations {
        match &op {
            Operation::DropTable { .. }
            | Operation::DropEnum { .. }
            | Operation::DropSequence { .. }
            | Operation::DropView { .. }
            | Operation::DropIndex { .. }
            | Operation::DropColumn { .. }
            | Operation::DropConstraint { .. } => drops.push(op),

            Operation::CreateTable { .. }
            | Operation::CreateEnum { .. }
            | Operation::CreateSequence { .. }
            | Operation::CreateView { .. }
            | Operation::CreateIndex { .. }
            | Operation::AddColumn { .. }
            | Operation::AddConstraint { .. }
            | Operation::AddEnumValue { .. } => creates.push(op),

            Operation::RenameTable { .. }
            | Operation::RenameEnum { .. }
            | Operation::RenameSequence { .. }
            | Operation::RenameView { .. }
            | Operation::RenameIndex { .. }
            | Operation::RenameColumn { .. }
            | Operation::RenameConstraint { .. } => renames.push(op),

            Operation::AlterColumn { .. }
            | Operation::AlterSequence { .. }
            | Operation::ReplaceView { .. }
            | Operation::RefreshMaterializedView { .. } => alters.push(op),

            Operation::SetComment { .. } => comments.push(op),
        }
    }

    // Sort drops in reverse dependency order
    // (views before tables, constraints before tables, etc.)
    drops.sort_by_key(|op| std::cmp::Reverse(op.object_kind()));

    // Sort creates in dependency order
    // (enums before tables, tables before views, etc.)
    let creates = sort_creates_by_dependency(creates);

    // Combine: drops first, then renames, then creates, then alters, then comments
    let mut result = Vec::with_capacity(
        drops.len() + renames.len() + creates.len() + alters.len() + comments.len()
    );
    result.extend(drops);
    result.extend(renames);
    result.extend(creates);
    result.extend(alters);
    result.extend(comments);

    result
}

/// Sort create operations respecting foreign key dependencies.
fn sort_creates_by_dependency(mut operations: Vec<Operation>) -> Vec<Operation> {
    // First, sort by object kind (enums < sequences < tables < constraints < indexes < views)
    operations.sort_by_key(|op| op.object_kind());

    // Then, within tables and constraints, respect FK dependencies
    // This is a simplified topological sort

    // Build dependency graph for tables (based on FK references)
    let mut table_order: HashMap<(SchemaName, TableName), usize> = HashMap::new();
    let mut fk_deps: Vec<((SchemaName, TableName), (SchemaName, TableName))> = Vec::new();

    for op in &operations {
        if let Operation::CreateTable { schema, table } = op {
            table_order.insert((schema.clone(), table.name.clone()), 0);

            // Find FK dependencies
            for constraint in &table.constraints {
                if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                    fk_deps.push((
                        (schema.clone(), table.name.clone()),
                        (fk.referenced_table.schema.clone(), fk.referenced_table.name.clone()),
                    ));
                }
            }
        }
    }

    // Simple topological ordering for tables
    // (Full Kahn's algorithm would be more robust for complex cases)
    for (from, to) in &fk_deps {
        if let (Some(from_order), Some(to_order)) = (
            table_order.get_mut(from),
            table_order.get(to),
        ) {
            *from_order = (*from_order).max(to_order + 1);
        }
    }

    // Re-sort tables by computed order
    operations.sort_by(|a, b| {
        let a_order = match a {
            Operation::CreateTable { schema, table } => {
                (a.object_kind(), *table_order.get(&(schema.clone(), table.name.clone())).unwrap_or(&0))
            }
            _ => (a.object_kind(), 0),
        };
        let b_order = match b {
            Operation::CreateTable { schema, table } => {
                (b.object_kind(), *table_order.get(&(schema.clone(), table.name.clone())).unwrap_or(&0))
            }
            _ => (b.object_kind(), 0),
        };
        a_order.cmp(&b_order)
    });

    operations
}
```

---

## 3. Error Handling (`error.rs`)

```rust
//! Migration-specific error types.

use miette::Diagnostic;
use thiserror::Error;

use crate::db::schema::{ColumnName, SchemaName, TableName, TypeName};

/// Errors that can occur during migration planning or execution.
#[derive(Debug, Error, Diagnostic)]
pub enum MigrationError {
    #[error("circular dependency detected involving table {schema}.{table}")]
    #[diagnostic(code(tern::migrate::circular_dependency))]
    CircularDependency {
        schema: SchemaName,
        table: TableName,
    },

    #[error("cannot drop enum {schema}.{name} because it is still in use")]
    #[diagnostic(
        code(tern::migrate::enum_in_use),
        help("Drop or alter the columns using this enum first")
    )]
    EnumInUse {
        schema: SchemaName,
        name: TypeName,
    },

    #[error("cannot generate rollback for {operation}")]
    #[diagnostic(
        code(tern::migrate::no_rollback),
        help("Some operations like DROP COLUMN cannot be automatically reversed")
    )]
    NoRollback {
        operation: String,
    },

    #[error("unsupported operation: {description}")]
    #[diagnostic(code(tern::migrate::unsupported))]
    Unsupported {
        description: String,
    },
}
```

---

## 4. Public API (`mod.rs`)

```rust
//! Migration planning and execution.
//!
//! This module provides the pipeline for converting schema diffs into
//! executable SQL migrations.
//!
//! # Architecture
//!
//! The migration system uses a three-layer pipeline:
//!
//! 1. **Collection**: `OperationCollector` extracts semantic operations from diffs
//! 2. **Ordering**: Operations are topologically sorted by dependencies
//! 3. **Rendering**: `PostgresRenderer` converts operations to SQL
//!
//! # Example
//!
//! ```ignore
//! use tern::db::diff::diff_namespaces;
//! use tern::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
//!
//! // Compare schemas
//! let diff = diff_namespaces(&source, &target);
//!
//! // Create migration plan
//! let plan = MigrationPlan::from_diff(&diff);
//!
//! // Render to SQL
//! let renderer = PostgresRenderer::new(RenderConfig::default());
//! let script = plan.render(&renderer);
//!
//! // Get the SQL
//! println!("{}", script.to_sql());
//! ```

mod collector;
mod error;
mod operation;
mod ordering;
mod plan;
mod render;
mod script;

pub use error::MigrationError;
pub use operation::{
    ColumnChanges, CommentTarget, DefaultChange, EnumValuePosition,
    GeneratedChange, IdentityChange, ObjectKind, Operation, OperationId,
    SequenceChanges, SetColumnType,
};
pub use plan::{MigrationPlan, PlanConfig};
pub use render::{IdentifierQuoting, PostgresRenderer, RenderConfig, RenderedOperation, Renderer};
pub use script::{MigrationScript, SqlOptions};
```

---

## 5. Implementation Phases

### Phase 1: Foundation
- [ ] Create module structure (`src/db/migrate/`)
- [ ] Define `Operation` enum with all variants
- [ ] Define supporting types (`ColumnChanges`, `SequenceChanges`, etc.)
- [ ] Implement `ObjectKind` ordering
- [ ] Add basic error types

### Phase 2: PostgreSQL Renderer
- [ ] Implement `IdentifierQuoting` utilities
- [ ] Implement `PostgresRenderer` for table operations
- [ ] Implement column operation rendering
- [ ] Implement constraint operation rendering
- [ ] Implement index operation rendering
- [ ] Implement enum operation rendering
- [ ] Implement sequence operation rendering
- [ ] Implement view operation rendering
- [ ] Implement comment operation rendering

### Phase 3: Operation Collection
- [ ] Implement `OperationCollector::collect()`
- [ ] Handle drops in reverse dependency order
- [ ] Handle creates with FK awareness
- [ ] Handle modifications (columns, constraints, indexes)
- [ ] Handle renames (tables, columns, etc.)
- [ ] Handle enum modifications (add values)

### Phase 4: Dependency Ordering
- [ ] Implement basic `ObjectKind` sorting
- [ ] Implement FK-aware table ordering
- [ ] Handle circular dependency detection
- [ ] Handle self-referential FK tables

### Phase 5: Migration Script
- [ ] Implement `MigrationScript` aggregation
- [ ] Implement `to_sql()` with options
- [ ] Implement `to_rollback_sql()`
- [ ] Add transaction wrapping options

### Phase 6: Testing
- [ ] Unit tests for each renderer method
- [ ] Unit tests for operation collection
- [ ] Integration tests with full diff → SQL pipeline
- [ ] Golden file tests comparing expected SQL output
- [ ] Round-trip tests (apply migration, compare schemas)

### Phase 7: CLI Integration
- [ ] Add `migrate diff` command
- [ ] Add `migrate plan` command (shows operations)
- [ ] Add `migrate sql` command (outputs SQL)
- [ ] Add dry-run mode

---

## 6. Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    mod postgres_renderer {
        use super::*;

        #[test]
        fn render_create_table_basic() {
            let table = Table {
                name: TableName::try_new("users".to_string()).unwrap(),
                columns: vec![
                    Column {
                        name: ColumnName::try_new("id".to_string()).unwrap(),
                        type_info: TypeInfo {
                            name: TypeName::try_new("int4".to_string()).unwrap(),
                            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                            formatted: "integer".to_string(),
                            is_array: false,
                        },
                        is_nullable: false,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };

            let renderer = PostgresRenderer::new(RenderConfig::default());
            let op = Operation::CreateTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                table,
            };

            let rendered = renderer.render(&op);

            assert!(rendered.forward[0].contains("CREATE TABLE"));
            assert!(rendered.forward[0].contains("public"));
            assert!(rendered.forward[0].contains("users"));
            assert!(rendered.rollback.is_some());
        }

        #[test]
        fn render_alter_column_type() {
            let renderer = PostgresRenderer::new(RenderConfig::default());
            let op = Operation::AlterColumn {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                table: TableName::try_new("users".to_string()).unwrap(),
                name: ColumnName::try_new("email".to_string()).unwrap(),
                changes: ColumnChanges {
                    set_type: Some(SetColumnType {
                        type_info: TypeInfo {
                            formatted: "text".to_string(),
                            ..Default::default()
                        },
                        using: None,
                    }),
                    ..Default::default()
                },
            };

            let rendered = renderer.render(&op);

            assert!(rendered.forward[0].contains("ALTER TABLE"));
            assert!(rendered.forward[0].contains("ALTER COLUMN"));
            assert!(rendered.forward[0].contains("TYPE text"));
        }
    }

    mod operation_collector {
        use super::*;

        #[test]
        fn collect_added_table() {
            let diff = NamespaceDiff {
                name: SchemaName::try_new("public".to_string()).unwrap(),
                tables: TableDiff {
                    added: vec![/* table */],
                    removed: vec![],
                    modified: vec![],
                    potential_renames: vec![],
                },
                ..Default::default()
            };

            let plan = MigrationPlan::from_diff(&diff);

            assert!(!plan.is_empty());
            assert!(matches!(plan.operations[0], Operation::CreateTable { .. }));
        }
    }

    mod ordering {
        use super::*;

        #[test]
        fn enums_before_tables() {
            let ops = vec![
                Operation::CreateTable { /* uses enum */ },
                Operation::CreateEnum { /* enum definition */ },
            ];

            let sorted = topological_sort(ops);

            assert!(matches!(sorted[0], Operation::CreateEnum { .. }));
            assert!(matches!(sorted[1], Operation::CreateTable { .. }));
        }

        #[test]
        fn fk_referenced_table_first() {
            // Table A references Table B via FK
            // B should be created before A
            let ops = vec![
                Operation::CreateTable { /* table A with FK to B */ },
                Operation::CreateTable { /* table B */ },
            ];

            let sorted = topological_sort(ops);

            // Verify B comes before A
        }
    }
}
```

### Integration Tests

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn full_pipeline_add_column() {
        // Source: table with 2 columns
        // Target: table with 3 columns
        let source = Namespace { /* ... */ };
        let target = Namespace { /* ... */ };

        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let script = plan.render(&renderer);

        let sql = script.to_sql();

        assert!(sql.contains("ALTER TABLE"));
        assert!(sql.contains("ADD COLUMN"));
    }
}
```

---

## 7. Future Considerations

### Extensibility Points

1. **New Renderers**: The `Renderer` trait allows adding MySQL, SQLite, etc.
2. **Custom Operations**: The `Operation` enum can be extended for new DDL.
3. **Hooks**: Pre/post operation hooks for custom logic.
4. **Dry Run**: Operation validation without execution.

### Potential Enhancements

1. **Data Migration**: Support for `UPDATE` statements during type changes.
2. **Batching**: Combine multiple `ALTER TABLE` operations.
3. **Concurrency**: `CONCURRENTLY` options for index operations.
4. **Progress Tracking**: Callbacks during execution.
5. **Checkpointing**: Resume failed migrations.
