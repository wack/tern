//! Builder API for ergonomic schema construction.
//!
//! This module provides builders for constructing `Namespace`, `Table`, `Column`,
//! and other schema objects without requiring a database connection. This is
//! essential for:
//!
//! - **Snapshot testing**: Define source and target schemas programmatically
//! - **Unit testing**: Create test fixtures without database setup
//! - **Schema-as-code**: Define schemas in Rust code
//!
//! # Example
//!
//! ```ignore
//! use tern::db::history::builder::NamespaceBuilder;
//!
//! let schema = NamespaceBuilder::new("public")
//!     .table("users", |t| {
//!         t.column("id", "integer").not_null()
//!          .column("email", "text").not_null()
//!          .column("created_at", "timestamptz").default("now()")
//!          .primary_key(["id"])
//!          .unique(["email"])
//!     })
//!     .table("posts", |t| {
//!         t.column("id", "integer").not_null()
//!          .column("user_id", "integer").not_null()
//!          .column("title", "text").not_null()
//!          .primary_key(["id"])
//!          .foreign_key(["user_id"], "users", ["id"])
//!     })
//!     .enum_type("status", ["pending", "active", "archived"])
//!     .build();
//! ```

use crate::db::model::column::{Column, GeneratedColumn, GeneratedStorage, IdentityKind};
use crate::db::model::constraint::{
    CheckConstraint, Constraint, ConstraintKind, ForeignKeyConstraint, PrimaryKeyConstraint,
    UniqueConstraint,
};
use crate::db::model::index::{Index, IndexColumn, NullsOrder, SortOrder};
use crate::db::model::types::{
    ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedTableName, SqlExpr, TypeInfo,
};
use crate::db::model::{EnumType, Namespace, Sequence, Table, TableKind, View};
use crate::db::schema::{
    CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName, TableName,
    TypeName,
};

use super::OidGenerator;

// =============================================================================
// NamespaceBuilder
// =============================================================================

/// Builder for constructing a `Namespace` with tables, views, sequences, and enums.
///
/// # Example
///
/// ```ignore
/// let schema = NamespaceBuilder::new("public")
///     .table("users", |t| t.column("id", "integer").not_null())
///     .enum_type("status", ["pending", "active"])
///     .build();
/// ```
pub struct NamespaceBuilder {
    name: SchemaName,
    tables: Vec<Table>,
    views: Vec<View>,
    sequences: Vec<Sequence>,
    enums: Vec<EnumType>,
    oid_gen: OidGenerator,
}

impl NamespaceBuilder {
    /// Creates a new namespace builder with the given schema name.
    ///
    /// # Panics
    ///
    /// Panics if the schema name is empty.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: SchemaName::try_new(name.to_string()).expect("schema name must not be empty"),
            tables: vec![],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            oid_gen: OidGenerator::new(),
        }
    }

    /// Adds a table to the namespace using a builder function.
    ///
    /// # Example
    ///
    /// ```ignore
    /// builder.table("users", |t| {
    ///     t.column("id", "integer").not_null()
    ///      .column("name", "text")
    ///      .primary_key(["id"])
    /// });
    /// ```
    #[must_use]
    pub fn table<F>(mut self, name: &str, f: F) -> Self
    where
        F: FnOnce(TableBuilder) -> TableBuilder,
    {
        let oid = self.oid_gen.generate();
        let builder = TableBuilder::new(name, oid, self.name.clone());
        let table = f(builder).build();
        self.tables.push(table);
        self
    }

    /// Adds a pre-built table to the namespace.
    #[must_use]
    pub fn with_table(mut self, table: Table) -> Self {
        self.tables.push(table);
        self
    }

    /// Adds a view to the namespace.
    ///
    /// # Example
    ///
    /// ```ignore
    /// builder.view("active_users", "SELECT * FROM users WHERE active = true");
    /// ```
    #[must_use]
    pub fn view(mut self, name: &str, definition: &str) -> Self {
        let view = View {
            oid: self.oid_gen.generate(),
            name: TableName::try_new(name.to_string()).unwrap(),
            definition: SqlExpr::new(definition.to_string()),
            is_materialized: false,
            comment: None,
        };
        self.views.push(view);
        self
    }

    /// Adds a materialized view to the namespace.
    #[must_use]
    pub fn materialized_view(mut self, name: &str, definition: &str) -> Self {
        let view = View {
            oid: self.oid_gen.generate(),
            name: TableName::try_new(name.to_string()).unwrap(),
            definition: SqlExpr::new(definition.to_string()),
            is_materialized: true,
            comment: None,
        };
        self.views.push(view);
        self
    }

    /// Adds a sequence to the namespace with default settings.
    ///
    /// For bigint sequence starting at 1, incrementing by 1.
    #[must_use]
    pub fn sequence(mut self, name: &str) -> Self {
        let sequence = Sequence {
            oid: self.oid_gen.generate(),
            name: SequenceName::try_new(name.to_string()).unwrap(),
            data_type: TypeInfo {
                name: TypeName::try_new("int8".to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: "bigint".to_string(),
                is_array: false,
            },
            start_value: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cache_size: 1,
            is_cyclic: false,
            comment: None,
        };
        self.sequences.push(sequence);
        self
    }

    /// Adds a sequence with custom configuration.
    #[must_use]
    pub fn sequence_with<F>(mut self, name: &str, f: F) -> Self
    where
        F: FnOnce(SequenceBuilder) -> SequenceBuilder,
    {
        let oid = self.oid_gen.generate();
        let builder = SequenceBuilder::new(name, oid);
        let sequence = f(builder).build();
        self.sequences.push(sequence);
        self
    }

    /// Adds an enum type to the namespace.
    ///
    /// # Example
    ///
    /// ```ignore
    /// builder.enum_type("status", ["pending", "active", "completed"]);
    /// ```
    #[must_use]
    pub fn enum_type<I, S>(mut self, name: &str, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let enum_type = EnumType {
            oid: self.oid_gen.generate(),
            name: TypeName::try_new(name.to_string()).unwrap(),
            values: values.into_iter().map(Into::into).collect(),
            comment: None,
        };
        self.enums.push(enum_type);
        self
    }

    /// Builds the namespace.
    #[must_use]
    pub fn build(mut self) -> Namespace {
        Namespace {
            oid: self.oid_gen.generate(),
            name: self.name,
            tables: self.tables,
            views: self.views,
            sequences: self.sequences,
            enums: self.enums,
            comment: None,
        }
    }
}

// =============================================================================
// TableBuilder
// =============================================================================

/// Builder for constructing a `Table` with columns, constraints, and indexes.
pub struct TableBuilder {
    name: TableName,
    oid: Oid,
    schema: SchemaName,
    kind: TableKind,
    columns: Vec<Column>,
    constraints: Vec<Constraint>,
    indexes: Vec<Index>,
    next_column_position: i16,
    oid_gen: OidGenerator,
}

impl TableBuilder {
    /// Creates a new table builder.
    fn new(name: &str, oid: Oid, schema: SchemaName) -> Self {
        Self {
            name: TableName::try_new(name.to_string()).unwrap(),
            oid,
            schema,
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            next_column_position: 1,
            oid_gen: OidGenerator::new(),
        }
    }

    /// Sets the table kind to partitioned.
    #[must_use]
    pub fn partitioned(mut self) -> Self {
        self.kind = TableKind::Partitioned;
        self
    }

    /// Adds a column to the table.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.column("id", "integer").not_null()
    ///      .column("name", "text");
    /// ```
    #[must_use]
    pub fn column(mut self, name: &str, type_name: &str) -> Self {
        let column = ColumnBuilder::new(name, type_name, self.next_column_position).build();
        self.next_column_position += 1;
        self.columns.push(column);
        self
    }

    /// Adds a column with custom configuration.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.column_with("id", "integer", |c| c.not_null().identity_always());
    /// ```
    #[must_use]
    pub fn column_with<F>(mut self, name: &str, type_name: &str, f: F) -> Self
    where
        F: FnOnce(ColumnBuilder) -> ColumnBuilder,
    {
        let builder = ColumnBuilder::new(name, type_name, self.next_column_position);
        let column = f(builder).build();
        self.next_column_position += 1;
        self.columns.push(column);
        self
    }

    /// Adds a primary key constraint on the specified columns.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.primary_key(["id"]);
    /// table.primary_key(["tenant_id", "id"]); // Composite PK
    /// ```
    #[must_use]
    pub fn primary_key<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns: Vec<ColumnName> = columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let pk_name = format!("{}_pkey", self.name.as_ref());
        let index_name = IndexName::try_new(pk_name.clone()).unwrap();

        // Create the constraint index
        let index = Index {
            oid: self.oid_gen.generate(),
            name: index_name.clone(),
            method: IndexMethod::BTree,
            is_unique: true,
            is_constraint_index: true,
            columns: columns
                .iter()
                .map(|c| IndexColumn {
                    column: Some(c.clone()),
                    expression: None,
                    order: SortOrder::default(),
                    nulls: NullsOrder::default(),
                })
                .collect(),
            predicate: None,
            comment: None,
        };
        self.indexes.push(index);

        let constraint = Constraint {
            name: ConstraintName::try_new(pk_name).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns,
                index_name,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds a unique constraint on the specified columns.
    #[must_use]
    pub fn unique<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns: Vec<ColumnName> = columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let col_names: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
        let unique_name = format!("{}_{}_key", self.name.as_ref(), col_names.join("_"));
        let index_name = IndexName::try_new(unique_name.clone()).unwrap();

        // Create the constraint index
        let index = Index {
            oid: self.oid_gen.generate(),
            name: index_name.clone(),
            method: IndexMethod::BTree,
            is_unique: true,
            is_constraint_index: true,
            columns: columns
                .iter()
                .map(|c| IndexColumn {
                    column: Some(c.clone()),
                    expression: None,
                    order: SortOrder::default(),
                    nulls: NullsOrder::default(),
                })
                .collect(),
            predicate: None,
            comment: None,
        };
        self.indexes.push(index);

        let constraint = Constraint {
            name: ConstraintName::try_new(unique_name).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns,
                index_name,
                nulls_not_distinct: false,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds a foreign key constraint.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.foreign_key(["user_id"], "users", ["id"]);
    /// ```
    #[must_use]
    pub fn foreign_key<I1, I2, S1, S2>(
        mut self,
        columns: I1,
        referenced_table: &str,
        referenced_columns: I2,
    ) -> Self
    where
        I1: IntoIterator<Item = S1>,
        I2: IntoIterator<Item = S2>,
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        let columns: Vec<ColumnName> = columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let referenced_columns: Vec<ColumnName> = referenced_columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let col_names: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
        let fk_name = format!("{}_{}_fkey", self.name.as_ref(), col_names.join("_"));

        let constraint = Constraint {
            name: ConstraintName::try_new(fk_name).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns,
                referenced_table: QualifiedTableName::new(
                    self.schema.clone(),
                    TableName::try_new(referenced_table.to_string()).unwrap(),
                ),
                referenced_columns,
                on_delete: ForeignKeyAction::default(),
                on_update: ForeignKeyAction::default(),
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds a foreign key constraint with custom actions.
    #[must_use]
    pub fn foreign_key_with<I1, I2, S1, S2>(
        mut self,
        columns: I1,
        referenced_table: &str,
        referenced_columns: I2,
        on_delete: ForeignKeyAction,
        on_update: ForeignKeyAction,
    ) -> Self
    where
        I1: IntoIterator<Item = S1>,
        I2: IntoIterator<Item = S2>,
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        let columns: Vec<ColumnName> = columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let referenced_columns: Vec<ColumnName> = referenced_columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let col_names: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
        let fk_name = format!("{}_{}_fkey", self.name.as_ref(), col_names.join("_"));

        let constraint = Constraint {
            name: ConstraintName::try_new(fk_name).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns,
                referenced_table: QualifiedTableName::new(
                    self.schema.clone(),
                    TableName::try_new(referenced_table.to_string()).unwrap(),
                ),
                referenced_columns,
                on_delete,
                on_update,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds a check constraint.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.check("age >= 0");
    /// ```
    #[must_use]
    pub fn check(mut self, expression: &str) -> Self {
        let check_name = format!("{}_check", self.name.as_ref());
        let constraint = Constraint {
            name: ConstraintName::try_new(check_name).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new(expression.to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds a named check constraint.
    #[must_use]
    pub fn check_named(mut self, name: &str, expression: &str) -> Self {
        let constraint = Constraint {
            name: ConstraintName::try_new(name.to_string()).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new(expression.to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        };
        self.constraints.push(constraint);
        self
    }

    /// Adds an index on the specified columns.
    ///
    /// # Example
    ///
    /// ```ignore
    /// table.index(["email"]);
    /// table.index(["last_name", "first_name"]);
    /// ```
    #[must_use]
    pub fn index<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let columns: Vec<ColumnName> = columns
            .into_iter()
            .map(|s| ColumnName::try_new(s.as_ref().to_string()).unwrap())
            .collect();

        let col_names: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
        let index_name = format!("{}_{}_idx", self.name.as_ref(), col_names.join("_"));

        let index = Index {
            oid: self.oid_gen.generate(),
            name: IndexName::try_new(index_name).unwrap(),
            method: IndexMethod::BTree,
            is_unique: false,
            is_constraint_index: false,
            columns: columns
                .into_iter()
                .map(|c| IndexColumn {
                    column: Some(c),
                    expression: None,
                    order: SortOrder::default(),
                    nulls: NullsOrder::default(),
                })
                .collect(),
            predicate: None,
            comment: None,
        };
        self.indexes.push(index);
        self
    }

    /// Adds an index with custom configuration.
    #[must_use]
    pub fn index_with<F>(mut self, name: &str, f: F) -> Self
    where
        F: FnOnce(IndexBuilder) -> IndexBuilder,
    {
        let oid = self.oid_gen.generate();
        let builder = IndexBuilder::new(name, oid);
        let index = f(builder).build();
        self.indexes.push(index);
        self
    }

    /// Builds the table.
    fn build(self) -> Table {
        Table {
            oid: self.oid,
            name: self.name,
            kind: self.kind,
            columns: self.columns,
            constraints: self.constraints,
            indexes: self.indexes,
            comment: None,
        }
    }
}

// =============================================================================
// ColumnBuilder
// =============================================================================

/// Builder for constructing a `Column`.
pub struct ColumnBuilder {
    name: ColumnName,
    position: i16,
    type_info: TypeInfo,
    is_nullable: bool,
    default: Option<SqlExpr>,
    generated: Option<GeneratedColumn>,
    identity: Option<IdentityKind>,
    collation: QualifiedCollationName,
}

impl ColumnBuilder {
    /// Creates a new column builder.
    fn new(name: &str, type_name: &str, position: i16) -> Self {
        Self {
            name: ColumnName::try_new(name.to_string()).unwrap(),
            position,
            type_info: parse_type_info(type_name),
            is_nullable: true,
            default: None,
            generated: None,
            identity: None,
            collation: QualifiedCollationName::new(
                SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                CollationName::try_new("default".to_string()).unwrap(),
            ),
        }
    }

    /// Makes the column NOT NULL.
    #[must_use]
    pub fn not_null(mut self) -> Self {
        self.is_nullable = false;
        self
    }

    /// Makes the column nullable (default).
    #[must_use]
    pub fn nullable(mut self) -> Self {
        self.is_nullable = true;
        self
    }

    /// Sets a default value for the column.
    ///
    /// # Example
    ///
    /// ```ignore
    /// column.default("now()")
    /// column.default("0")
    /// column.default("'pending'::status")
    /// ```
    #[must_use]
    pub fn default(mut self, expression: &str) -> Self {
        self.default = Some(SqlExpr::new(expression.to_string()));
        self
    }

    /// Makes the column an identity column (GENERATED ALWAYS AS IDENTITY).
    #[must_use]
    pub fn identity_always(mut self) -> Self {
        self.identity = Some(IdentityKind::Always);
        self
    }

    /// Makes the column an identity column (GENERATED BY DEFAULT AS IDENTITY).
    #[must_use]
    pub fn identity_by_default(mut self) -> Self {
        self.identity = Some(IdentityKind::ByDefault);
        self
    }

    /// Makes the column a generated column (GENERATED ALWAYS AS ... STORED).
    #[must_use]
    pub fn generated(mut self, expression: &str) -> Self {
        self.generated = Some(GeneratedColumn {
            expression: SqlExpr::new(expression.to_string()),
            storage: GeneratedStorage::Stored,
        });
        self
    }

    /// Sets the collation for the column.
    #[must_use]
    pub fn collation(mut self, schema: &str, name: &str) -> Self {
        self.collation = QualifiedCollationName::new(
            SchemaName::try_new(schema.to_string()).unwrap(),
            CollationName::try_new(name.to_string()).unwrap(),
        );
        self
    }

    /// Builds the column.
    fn build(self) -> Column {
        Column {
            name: self.name,
            position: self.position,
            type_info: self.type_info,
            is_nullable: self.is_nullable,
            default: self.default,
            generated: self.generated,
            identity: self.identity,
            collation: self.collation,
            comment: None,
        }
    }
}

// =============================================================================
// IndexBuilder
// =============================================================================

/// Builder for constructing an `Index`.
pub struct IndexBuilder {
    name: IndexName,
    oid: Oid,
    method: IndexMethod,
    is_unique: bool,
    columns: Vec<IndexColumn>,
    predicate: Option<SqlExpr>,
}

impl IndexBuilder {
    /// Creates a new index builder.
    fn new(name: &str, oid: Oid) -> Self {
        Self {
            name: IndexName::try_new(name.to_string()).unwrap(),
            oid,
            method: IndexMethod::BTree,
            is_unique: false,
            columns: vec![],
            predicate: None,
        }
    }

    /// Sets the index method.
    #[must_use]
    pub fn method(mut self, method: IndexMethod) -> Self {
        self.method = method;
        self
    }

    /// Makes the index unique.
    #[must_use]
    pub fn unique(mut self) -> Self {
        self.is_unique = true;
        self
    }

    /// Adds a column to the index.
    #[must_use]
    pub fn column(mut self, name: &str) -> Self {
        self.columns.push(IndexColumn {
            column: Some(ColumnName::try_new(name.to_string()).unwrap()),
            expression: None,
            order: SortOrder::default(),
            nulls: NullsOrder::default(),
        });
        self
    }

    /// Adds a column with descending order.
    #[must_use]
    pub fn column_desc(mut self, name: &str) -> Self {
        self.columns.push(IndexColumn {
            column: Some(ColumnName::try_new(name.to_string()).unwrap()),
            expression: None,
            order: SortOrder::Descending,
            nulls: NullsOrder::First,
        });
        self
    }

    /// Adds an expression to the index.
    #[must_use]
    pub fn expression(mut self, expr: &str) -> Self {
        self.columns.push(IndexColumn {
            column: None,
            expression: Some(SqlExpr::new(expr.to_string())),
            order: SortOrder::default(),
            nulls: NullsOrder::default(),
        });
        self
    }

    /// Sets a partial index predicate (WHERE clause).
    #[must_use]
    pub fn where_clause(mut self, predicate: &str) -> Self {
        self.predicate = Some(SqlExpr::new(predicate.to_string()));
        self
    }

    /// Builds the index.
    fn build(self) -> Index {
        Index {
            oid: self.oid,
            name: self.name,
            method: self.method,
            is_unique: self.is_unique,
            is_constraint_index: false,
            columns: self.columns,
            predicate: self.predicate,
            comment: None,
        }
    }
}

// =============================================================================
// SequenceBuilder
// =============================================================================

/// Builder for constructing a `Sequence`.
pub struct SequenceBuilder {
    name: SequenceName,
    oid: Oid,
    data_type: TypeInfo,
    start_value: i64,
    increment: i64,
    min_value: i64,
    max_value: i64,
    cache_size: i64,
    is_cyclic: bool,
}

impl SequenceBuilder {
    /// Creates a new sequence builder with defaults for bigint.
    fn new(name: &str, oid: Oid) -> Self {
        Self {
            name: SequenceName::try_new(name.to_string()).unwrap(),
            oid,
            data_type: TypeInfo {
                name: TypeName::try_new("int8".to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: "bigint".to_string(),
                is_array: false,
            },
            start_value: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cache_size: 1,
            is_cyclic: false,
        }
    }

    /// Sets the data type to smallint.
    #[must_use]
    pub fn smallint(mut self) -> Self {
        self.data_type = TypeInfo {
            name: TypeName::try_new("int2".to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: "smallint".to_string(),
            is_array: false,
        };
        self.max_value = i16::MAX as i64;
        self
    }

    /// Sets the data type to integer.
    #[must_use]
    pub fn integer(mut self) -> Self {
        self.data_type = TypeInfo {
            name: TypeName::try_new("int4".to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: "integer".to_string(),
            is_array: false,
        };
        self.max_value = i32::MAX as i64;
        self
    }

    /// Sets the start value.
    #[must_use]
    pub fn start(mut self, value: i64) -> Self {
        self.start_value = value;
        self
    }

    /// Sets the increment.
    #[must_use]
    pub fn increment(mut self, value: i64) -> Self {
        self.increment = value;
        self
    }

    /// Sets the minimum value.
    #[must_use]
    pub fn min(mut self, value: i64) -> Self {
        self.min_value = value;
        self
    }

    /// Sets the maximum value.
    #[must_use]
    pub fn max(mut self, value: i64) -> Self {
        self.max_value = value;
        self
    }

    /// Sets the cache size.
    #[must_use]
    pub fn cache(mut self, size: i64) -> Self {
        self.cache_size = size;
        self
    }

    /// Makes the sequence cyclic.
    #[must_use]
    pub fn cycle(mut self) -> Self {
        self.is_cyclic = true;
        self
    }

    /// Builds the sequence.
    fn build(self) -> Sequence {
        Sequence {
            oid: self.oid,
            name: self.name,
            data_type: self.data_type,
            start_value: self.start_value,
            increment: self.increment,
            min_value: self.min_value,
            max_value: self.max_value,
            cache_size: self.cache_size,
            is_cyclic: self.is_cyclic,
            comment: None,
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Parses a type name string into TypeInfo.
///
/// Handles common PostgreSQL type names and their aliases.
fn parse_type_info(type_name: &str) -> TypeInfo {
    let (base_name, formatted, is_array) = parse_type_name(type_name);

    TypeInfo {
        name: TypeName::try_new(base_name).unwrap(),
        schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        formatted,
        is_array,
    }
}

/// Parses a type name, returning (internal_name, formatted_name, is_array).
fn parse_type_name(type_name: &str) -> (String, String, bool) {
    let is_array = type_name.ends_with("[]");
    let base = if is_array {
        &type_name[..type_name.len() - 2]
    } else {
        type_name
    };

    let (internal, formatted) = match base.to_lowercase().as_str() {
        // Integer types
        "smallint" | "int2" => ("int2", "smallint"),
        "integer" | "int" | "int4" => ("int4", "integer"),
        "bigint" | "int8" => ("int8", "bigint"),

        // Serial types (aliases for integer + sequence)
        "smallserial" | "serial2" => ("int2", "smallint"),
        "serial" | "serial4" => ("int4", "integer"),
        "bigserial" | "serial8" => ("int8", "bigint"),

        // Floating point
        "real" | "float4" => ("float4", "real"),
        "double precision" | "float8" => ("float8", "double precision"),

        // Numeric
        "numeric" | "decimal" => ("numeric", "numeric"),

        // Character types
        "text" => ("text", "text"),
        "varchar" | "character varying" => ("varchar", "character varying"),
        "char" | "character" => ("bpchar", "character"),

        // Boolean
        "boolean" | "bool" => ("bool", "boolean"),

        // Date/time
        "timestamp" | "timestamp without time zone" => ("timestamp", "timestamp without time zone"),
        "timestamptz" | "timestamp with time zone" => ("timestamptz", "timestamp with time zone"),
        "date" => ("date", "date"),
        "time" | "time without time zone" => ("time", "time without time zone"),
        "timetz" | "time with time zone" => ("timetz", "time with time zone"),
        "interval" => ("interval", "interval"),

        // Binary
        "bytea" => ("bytea", "bytea"),

        // UUID
        "uuid" => ("uuid", "uuid"),

        // JSON
        "json" => ("json", "json"),
        "jsonb" => ("jsonb", "jsonb"),

        // Network
        "inet" => ("inet", "inet"),
        "cidr" => ("cidr", "cidr"),
        "macaddr" => ("macaddr", "macaddr"),

        // Other
        "money" => ("money", "money"),
        "xml" => ("xml", "xml"),

        // Default: use as-is (for custom types)
        _ => (base, base),
    };

    let formatted = if is_array {
        format!("{}[]", formatted)
    } else {
        formatted.to_string()
    };

    (internal.to_string(), formatted, is_array)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod namespace_builder_tests {
        use super::*;

        #[test]
        fn empty_namespace() {
            let ns = NamespaceBuilder::new("public").build();
            assert_eq!(ns.name.as_ref(), "public");
            assert!(ns.tables.is_empty());
            assert!(ns.views.is_empty());
            assert!(ns.sequences.is_empty());
            assert!(ns.enums.is_empty());
        }

        #[test]
        fn namespace_with_table() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("id", "integer").column("name", "text")
                })
                .build();

            assert_eq!(ns.tables.len(), 1);
            assert_eq!(ns.tables[0].name.as_ref(), "users");
            assert_eq!(ns.tables[0].columns.len(), 2);
        }

        #[test]
        fn namespace_with_enum() {
            let ns = NamespaceBuilder::new("public")
                .enum_type("status", ["pending", "active", "completed"])
                .build();

            assert_eq!(ns.enums.len(), 1);
            assert_eq!(ns.enums[0].name.as_ref(), "status");
            assert_eq!(ns.enums[0].values, vec!["pending", "active", "completed"]);
        }

        #[test]
        fn namespace_with_view() {
            let ns = NamespaceBuilder::new("public")
                .view("active_users", "SELECT * FROM users WHERE active")
                .materialized_view("user_stats", "SELECT COUNT(*) FROM users")
                .build();

            assert_eq!(ns.views.len(), 2);
            assert!(!ns.views[0].is_materialized);
            assert!(ns.views[1].is_materialized);
        }

        #[test]
        fn namespace_with_sequence() {
            let ns = NamespaceBuilder::new("public")
                .sequence("users_id_seq")
                .sequence_with("orders_id_seq", |s| s.start(1000).increment(10))
                .build();

            assert_eq!(ns.sequences.len(), 2);
            assert_eq!(ns.sequences[1].start_value, 1000);
            assert_eq!(ns.sequences[1].increment, 10);
        }
    }

    mod table_builder_tests {
        use super::*;

        #[test]
        fn table_with_columns() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("id", "integer")
                        .column_with("name", "text", |c| c.not_null())
                        .column_with("email", "text", |c| c.not_null())
                        .column_with("created_at", "timestamptz", |c| c.default("now()"))
                })
                .build();

            let table = &ns.tables[0];
            assert_eq!(table.columns.len(), 4);
            assert!(table.columns[0].is_nullable);
            assert!(!table.columns[1].is_nullable);
            assert!(table.columns[3].default.is_some());
        }

        #[test]
        fn table_with_primary_key() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("id", "integer")
                        .column("name", "text")
                        .primary_key(["id"])
                })
                .build();

            let table = &ns.tables[0];
            assert_eq!(table.constraints.len(), 1);
            assert!(matches!(
                &table.constraints[0].kind,
                ConstraintKind::PrimaryKey(_)
            ));
            // Should also have the backing index
            assert_eq!(table.indexes.len(), 1);
            assert!(table.indexes[0].is_constraint_index);
        }

        #[test]
        fn table_with_composite_primary_key() {
            let ns = NamespaceBuilder::new("public")
                .table("user_roles", |t| {
                    t.column("user_id", "integer")
                        .column("role_id", "integer")
                        .primary_key(["user_id", "role_id"])
                })
                .build();

            let table = &ns.tables[0];
            if let ConstraintKind::PrimaryKey(pk) = &table.constraints[0].kind {
                assert_eq!(pk.columns.len(), 2);
            } else {
                panic!("Expected PrimaryKey");
            }
        }

        #[test]
        fn table_with_foreign_key() {
            let ns = NamespaceBuilder::new("public")
                .table("posts", |t| {
                    t.column("id", "integer")
                        .column("user_id", "integer")
                        .foreign_key(["user_id"], "users", ["id"])
                })
                .build();

            let table = &ns.tables[0];
            assert_eq!(table.constraints.len(), 1);
            if let ConstraintKind::ForeignKey(fk) = &table.constraints[0].kind {
                assert_eq!(fk.referenced_table.name.as_ref(), "users");
            } else {
                panic!("Expected ForeignKey");
            }
        }

        #[test]
        fn table_with_foreign_key_cascade() {
            let ns = NamespaceBuilder::new("public")
                .table("posts", |t| {
                    t.column("id", "integer")
                        .column("user_id", "integer")
                        .foreign_key_with(
                            ["user_id"],
                            "users",
                            ["id"],
                            ForeignKeyAction::Cascade,
                            ForeignKeyAction::NoAction,
                        )
                })
                .build();

            let table = &ns.tables[0];
            if let ConstraintKind::ForeignKey(fk) = &table.constraints[0].kind {
                assert_eq!(fk.on_delete, ForeignKeyAction::Cascade);
            } else {
                panic!("Expected ForeignKey");
            }
        }

        #[test]
        fn table_with_check_constraint() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("age", "integer")
                        .check("age >= 0")
                        .check_named("users_age_max", "age <= 150")
                })
                .build();

            let table = &ns.tables[0];
            assert_eq!(table.constraints.len(), 2);
        }

        #[test]
        fn table_with_index() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("email", "text")
                        .column("last_name", "text")
                        .column("first_name", "text")
                        .index(["email"])
                        .index(["last_name", "first_name"])
                })
                .build();

            let table = &ns.tables[0];
            assert_eq!(table.indexes.len(), 2);
            assert!(!table.indexes[0].is_unique);
            assert!(!table.indexes[0].is_constraint_index);
        }

        #[test]
        fn table_with_custom_index() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("email", "text")
                        .column("active", "boolean")
                        .index_with("users_active_email_idx", |i| {
                            i.column("email").where_clause("active = true")
                        })
                })
                .build();

            let table = &ns.tables[0];
            assert!(table.indexes[0].predicate.is_some());
        }
    }

    mod column_builder_tests {
        use super::*;

        #[test]
        fn column_with_identity() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column_with("id", "integer", |c| c.not_null().identity_always())
                })
                .build();

            let column = &ns.tables[0].columns[0];
            assert_eq!(column.identity, Some(IdentityKind::Always));
        }

        #[test]
        fn column_with_generated() {
            let ns = NamespaceBuilder::new("public")
                .table("users", |t| {
                    t.column("first_name", "text")
                        .column("last_name", "text")
                        .column_with("full_name", "text", |c| {
                            c.generated("first_name || ' ' || last_name")
                        })
                })
                .build();

            let column = &ns.tables[0].columns[2];
            assert!(column.generated.is_some());
        }
    }

    mod type_parsing_tests {
        use super::*;

        #[test]
        fn parses_integer_types() {
            let ti = parse_type_info("integer");
            assert_eq!(ti.name.as_ref(), "int4");
            assert_eq!(ti.formatted, "integer");
            assert!(!ti.is_array);

            let ti = parse_type_info("bigint");
            assert_eq!(ti.name.as_ref(), "int8");
            assert_eq!(ti.formatted, "bigint");
        }

        #[test]
        fn parses_array_types() {
            let ti = parse_type_info("integer[]");
            assert_eq!(ti.name.as_ref(), "int4");
            assert_eq!(ti.formatted, "integer[]");
            assert!(ti.is_array);
        }

        #[test]
        fn parses_timestamp_types() {
            let ti = parse_type_info("timestamptz");
            assert_eq!(ti.name.as_ref(), "timestamptz");
            assert_eq!(ti.formatted, "timestamp with time zone");
        }

        #[test]
        fn parses_custom_types() {
            let ti = parse_type_info("my_custom_type");
            assert_eq!(ti.name.as_ref(), "my_custom_type");
            assert_eq!(ti.formatted, "my_custom_type");
        }
    }

    mod complex_schema_tests {
        use super::*;

        #[test]
        fn builds_complete_schema() {
            let schema = NamespaceBuilder::new("public")
                .enum_type(
                    "order_status",
                    ["pending", "processing", "shipped", "delivered"],
                )
                .table("users", |t| {
                    t.column_with("id", "integer", |c| c.not_null().identity_always())
                        .column_with("email", "text", |c| c.not_null())
                        .column_with("created_at", "timestamptz", |c| {
                            c.not_null().default("now()")
                        })
                        .primary_key(["id"])
                        .unique(["email"])
                })
                .table("orders", |t| {
                    t.column_with("id", "integer", |c| c.not_null().identity_always())
                        .column_with("user_id", "integer", |c| c.not_null())
                        .column("status", "order_status")
                        .column_with("total", "numeric", |c| c.not_null())
                        .column_with("created_at", "timestamptz", |c| c.default("now()"))
                        .primary_key(["id"])
                        .foreign_key_with(
                            ["user_id"],
                            "users",
                            ["id"],
                            ForeignKeyAction::Cascade,
                            ForeignKeyAction::NoAction,
                        )
                        .check_named("orders_total_positive", "total >= 0")
                        .index(["user_id"])
                        .index(["created_at"])
                })
                .view(
                    "pending_orders",
                    "SELECT * FROM orders WHERE status = 'pending'",
                )
                .sequence("invoice_number_seq")
                .build();

            // Verify structure
            assert_eq!(schema.enums.len(), 1);
            assert_eq!(schema.tables.len(), 2);
            assert_eq!(schema.views.len(), 1);
            assert_eq!(schema.sequences.len(), 1);

            // Verify users table
            let users = &schema.tables[0];
            assert_eq!(users.columns.len(), 3);
            assert_eq!(users.constraints.len(), 2); // PK + Unique
            assert_eq!(users.indexes.len(), 2); // PK index + Unique index

            // Verify orders table
            let orders = &schema.tables[1];
            assert_eq!(orders.columns.len(), 5);
            assert_eq!(orders.constraints.len(), 3); // PK + FK + Check
            assert_eq!(orders.indexes.len(), 3); // PK index + 2 regular indexes
        }
    }
}
