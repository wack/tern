//! Schema loader implementation.
//!
//! This module provides the pure schema loading logic that transforms
//! catalog row data into domain model types. It depends only on the
//! `Catalog` trait, making it testable without a real database.

use std::collections::HashMap;

use crate::db::diff::{self, DiffConfig, NamespaceDiff};
use crate::db::model::column::{Column, GeneratedColumn, GeneratedStorage, IdentityKind};
use crate::db::model::constraint::{
    CheckConstraint, Constraint, ConstraintKind, ExclusionConstraint, ExclusionElement,
    ForeignKeyConstraint, PrimaryKeyConstraint, UniqueConstraint,
};
use crate::db::model::index::{Index, IndexColumn, NullsOrder, SortOrder};
use crate::db::model::namespace::{EnumType, Namespace, Sequence, View};
use crate::db::model::table::{Table, TableKind};
use crate::db::model::types::{
    Comment, ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedTableName, SqlExpr,
    TypeInfo,
};
use crate::db::schema::{
    CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName, TableName,
    TypeName,
};

use super::catalog::{Catalog, ColumnRow, ConstraintRow, IndexRow, TableRow};
use super::error::QueryError;

/// Loads a namespace (schema) and all its contained objects from a catalog.
///
/// This is the main entry point for schema introspection. It loads:
/// - Tables with columns, constraints, and indexes
/// - Views (regular and materialized)
/// - Sequences
/// - Enum types
///
/// # Arguments
///
/// * `catalog` - A catalog implementation (real database or fake for testing)
/// * `schema_name` - The name of the schema to load
///
/// # Example
///
/// ```ignore
/// use tern::db::connect;
/// use tern::db::query::{load_namespace, PostgresCatalog};
///
/// let client = connect("postgres://localhost/mydb").await?;
/// let catalog = PostgresCatalog::new(&client);
/// let namespace = load_namespace(&catalog, "public").await?;
/// println!("Loaded {} tables", namespace.tables.len());
/// ```
pub async fn load_namespace<C: Catalog>(
    catalog: &C,
    schema_name: &str,
) -> Result<Namespace, QueryError> {
    // First, get the namespace metadata
    let ns_row = catalog
        .get_namespace(schema_name)
        .await?
        .ok_or_else(|| QueryError::NamespaceNotFound(schema_name.to_string()))?;

    let namespace_oid = Oid::new(ns_row.oid);
    let schema = SchemaName::try_new(ns_row.name)?;

    // Load all objects in parallel
    let (tables, views, sequences, enums) = tokio::try_join!(
        load_tables(catalog, namespace_oid, &schema),
        load_views(catalog, namespace_oid),
        load_sequences(catalog, namespace_oid),
        load_enums(catalog, namespace_oid),
    )?;

    Ok(Namespace {
        oid: namespace_oid,
        name: schema,
        tables,
        views,
        sequences,
        enums,
        comment: ns_row.comment.map(Comment::new),
    })
}

/// Computes the diff between an empty schema and the current database schema.
///
/// This is useful for initializing a new project. The diff result shows all
/// objects that exist in the database and would need to be created in a fresh
/// database to match the current state.
///
/// When starting a new migration project, you can use this function to:
/// 1. Discover all existing database objects
/// 2. Generate an initial "baseline" migration
/// 3. Understand what the current schema looks like
///
/// # Arguments
///
/// * `catalog` - A catalog implementation (real database or fake for testing)
/// * `schema_name` - The name of the schema to compare (e.g., "public")
///
/// # Returns
///
/// Returns a `NamespaceDiff` where:
/// - `added` contains all objects in the database (they're "added" relative to empty)
/// - `removed` is empty (nothing to remove from an empty schema)
/// - `modified` is empty (nothing to modify)
/// - `potential_renames` is empty (no source objects to rename)
///
/// # Example
///
/// ```ignore
/// use tern::db::connect;
/// use tern::db::query::{diff_from_empty, PostgresCatalog};
///
/// let client = connect("postgres://localhost/mydb").await?;
/// let catalog = PostgresCatalog::new(&client);
/// let diff = diff_from_empty(&catalog, "public").await?;
///
/// println!("Tables to create: {}", diff.tables.added.len());
/// for table in &diff.tables.added {
///     println!("  - {} ({} columns)", table.name.as_ref(), table.columns.len());
/// }
/// ```
pub async fn diff_from_empty<C: Catalog>(
    catalog: &C,
    schema_name: &str,
) -> Result<NamespaceDiff, QueryError> {
    diff_from_empty_with_config(catalog, schema_name, &DiffConfig::default()).await
}

/// Computes the diff between an empty schema and the current database schema
/// with a custom diff configuration.
///
/// This is the configurable version of [`diff_from_empty`]. Since we're comparing
/// against an empty schema, most configuration options (like rename detection
/// threshold) won't have any effect, but this function is provided for consistency
/// with the rest of the diff API.
///
/// # Arguments
///
/// * `catalog` - A catalog implementation (real database or fake for testing)
/// * `schema_name` - The name of the schema to compare (e.g., "public")
/// * `config` - The diff configuration to use
///
/// # Example
///
/// ```ignore
/// use tern::db::connect;
/// use tern::db::query::{diff_from_empty_with_config, PostgresCatalog};
/// use tern::db::diff::DiffConfig;
///
/// let client = connect("postgres://localhost/mydb").await?;
/// let catalog = PostgresCatalog::new(&client);
/// let config = DiffConfig::no_rename_detection();
/// let diff = diff_from_empty_with_config(&catalog, "public", &config).await?;
/// ```
pub async fn diff_from_empty_with_config<C: Catalog>(
    catalog: &C,
    schema_name: &str,
    config: &DiffConfig,
) -> Result<NamespaceDiff, QueryError> {
    let current = load_namespace(catalog, schema_name).await?;
    let empty = Namespace::empty(schema_name);
    Ok(diff::diff_namespaces_with_config(&empty, &current, config))
}

/// Loads all tables in a namespace.
async fn load_tables<C: Catalog>(
    catalog: &C,
    namespace_oid: Oid,
    schema: &SchemaName,
) -> Result<Vec<Table>, QueryError> {
    let rows = catalog.get_tables(namespace_oid).await?;

    let mut tables = Vec::with_capacity(rows.len());
    for row in rows {
        let table = load_table(catalog, &row, schema).await?;
        tables.push(table);
    }

    Ok(tables)
}

/// Loads a single table with all its columns, constraints, and indexes.
async fn load_table<C: Catalog>(
    catalog: &C,
    row: &TableRow,
    _schema: &SchemaName,
) -> Result<Table, QueryError> {
    let table_oid = Oid::new(row.oid);
    let kind = TableKind::try_from(row.relkind)?;

    // Load columns, constraints, and indexes in parallel
    let (columns, constraints, indexes) = tokio::try_join!(
        load_columns(catalog, table_oid),
        load_constraints(catalog, table_oid),
        load_indexes(catalog, table_oid),
    )?;

    Ok(Table {
        oid: table_oid,
        name: TableName::try_new(row.name.clone())?,
        kind,
        columns,
        constraints,
        indexes,
        comment: row.comment.clone().map(Comment::new),
    })
}

/// Loads all columns for a table.
async fn load_columns<C: Catalog>(catalog: &C, table_oid: Oid) -> Result<Vec<Column>, QueryError> {
    let rows = catalog.get_columns(table_oid).await?;
    rows.into_iter().map(row_to_column).collect()
}

/// Converts a column row to a domain Column.
fn row_to_column(row: ColumnRow) -> Result<Column, QueryError> {
    // Parse generated column info
    let generated = row
        .generated_kind
        .and_then(|kind| GeneratedStorage::try_from(kind).ok())
        .and_then(|storage| {
            row.default_expr.as_ref().map(|expr| GeneratedColumn {
                expression: SqlExpr::new(expr.clone()),
                storage,
            })
        });

    // Parse identity column info
    let identity = row
        .identity_kind
        .and_then(|c| IdentityKind::try_from(c).ok());

    // For generated columns, don't include the default expression separately
    let default = if generated.is_some() {
        None
    } else {
        row.default_expr.map(SqlExpr::new)
    };

    Ok(Column {
        name: ColumnName::try_new(row.name)?,
        position: row.position,
        type_info: TypeInfo {
            name: TypeName::try_new(row.type_name)?,
            schema: SchemaName::try_new(row.type_schema)?,
            formatted: row.formatted_type,
            is_array: row.is_array,
        },
        is_nullable: row.is_nullable,
        default,
        generated,
        identity,
        collation: QualifiedCollationName::new(
            SchemaName::try_new(row.collation_schema)?,
            CollationName::try_new(row.collation_name)?,
        ),
        comment: row.comment.map(Comment::new),
    })
}

/// Loads all constraints for a table.
async fn load_constraints<C: Catalog>(
    catalog: &C,
    table_oid: Oid,
) -> Result<Vec<Constraint>, QueryError> {
    let rows = catalog.get_constraints(table_oid).await?;
    let mut constraints = Vec::with_capacity(rows.len());

    for row in rows {
        // Get column names for this constraint
        let column_names = if let Some(ref keys) = row.columns {
            get_column_names_map(catalog, table_oid, keys).await?
        } else {
            HashMap::new()
        };

        if let Some(constraint) = row_to_constraint(catalog, &row, &column_names).await? {
            constraints.push(constraint);
        }
    }

    Ok(constraints)
}

/// Converts a constraint row to a domain Constraint.
async fn row_to_constraint<C: Catalog>(
    catalog: &C,
    row: &ConstraintRow,
    column_names: &HashMap<i16, String>,
) -> Result<Option<Constraint>, QueryError> {
    let kind = match row.constraint_type {
        'p' => {
            // Primary key
            let index_name = row
                .index_name
                .as_ref()
                .ok_or(QueryError::MissingField("index_name for primary key"))?;
            let columns =
                keys_to_column_names(&row.columns.clone().unwrap_or_default(), column_names)?;

            ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns,
                index_name: IndexName::try_new(index_name.clone())?,
            })
        }
        'u' => {
            // Unique constraint
            let index_name = row
                .index_name
                .as_ref()
                .ok_or(QueryError::MissingField("index_name for unique constraint"))?;
            let columns =
                keys_to_column_names(&row.columns.clone().unwrap_or_default(), column_names)?;

            ConstraintKind::Unique(UniqueConstraint {
                columns,
                index_name: IndexName::try_new(index_name.clone())?,
                nulls_not_distinct: false,
            })
        }
        'f' => {
            // Foreign key
            let ref_schema = row
                .foreign_schema
                .as_ref()
                .ok_or(QueryError::MissingField("ref_schema for foreign key"))?;
            let ref_table = row
                .foreign_table
                .as_ref()
                .ok_or(QueryError::MissingField("ref_table for foreign key"))?;
            let conf_key = row
                .foreign_columns
                .as_ref()
                .ok_or(QueryError::MissingField("foreign_columns for foreign key"))?;
            let conf_relid = row.foreign_table_oid.ok_or(QueryError::MissingField(
                "foreign_table_oid for foreign key",
            ))?;
            let del_type = row
                .on_delete
                .ok_or(QueryError::MissingField("on_delete for foreign key"))?;
            let upd_type = row
                .on_update
                .ok_or(QueryError::MissingField("on_update for foreign key"))?;

            // Get referenced column names
            let ref_column_names =
                get_column_names_map(catalog, Oid::new(conf_relid), conf_key).await?;

            let columns =
                keys_to_column_names(&row.columns.clone().unwrap_or_default(), column_names)?;
            let referenced_columns = keys_to_column_names(conf_key, &ref_column_names)?;

            ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns,
                referenced_table: QualifiedTableName::new(
                    SchemaName::try_new(ref_schema.clone())?,
                    TableName::try_new(ref_table.clone())?,
                ),
                referenced_columns,
                on_delete: ForeignKeyAction::try_from(del_type)?,
                on_update: ForeignKeyAction::try_from(upd_type)?,
                is_deferrable: row.is_deferrable,
                is_initially_deferred: row.is_initially_deferred,
            })
        }
        'c' => {
            // Check constraint
            let check_expr = row
                .check_expression
                .as_ref()
                .ok_or(QueryError::MissingField("check_expr for check constraint"))?;

            ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new(check_expr.clone()),
                is_no_inherit: row.is_no_inherit,
            })
        }
        'x' => {
            // Exclusion constraint
            let index_name = row.index_name.as_ref().ok_or(QueryError::MissingField(
                "index_name for exclusion constraint",
            ))?;
            let index_method = row.index_method.as_ref().ok_or(QueryError::MissingField(
                "index_method for exclusion constraint",
            ))?;

            // Get exclusion elements
            let elements = load_exclusion_elements(catalog, Oid::new(row.oid)).await?;

            ConstraintKind::Exclusion(ExclusionConstraint {
                elements,
                index_method: IndexMethod::try_from(index_method.as_str())?,
                index_name: IndexName::try_new(index_name.clone())?,
                predicate: row.index_predicate.clone().map(SqlExpr::new),
            })
        }
        other => {
            tracing::warn!("Unknown constraint type: {}", other);
            return Ok(None);
        }
    };

    Ok(Some(Constraint {
        name: ConstraintName::try_new(row.name.clone())?,
        kind,
        comment: row.comment.clone().map(Comment::new),
    }))
}

/// Loads exclusion constraint elements.
async fn load_exclusion_elements<C: Catalog>(
    catalog: &C,
    constraint_oid: Oid,
) -> Result<Vec<ExclusionElement>, QueryError> {
    let rows = catalog.get_exclusion_elements(constraint_oid).await?;

    Ok(rows
        .into_iter()
        .map(|r| ExclusionElement {
            expression: SqlExpr::new(r.expression),
            operator: r.operator,
        })
        .collect())
}

/// Loads all indexes for a table.
async fn load_indexes<C: Catalog>(catalog: &C, table_oid: Oid) -> Result<Vec<Index>, QueryError> {
    let rows = catalog.get_indexes(table_oid).await?;

    let mut indexes = Vec::with_capacity(rows.len());
    for row in rows {
        let index = load_index(catalog, &row).await?;
        indexes.push(index);
    }

    Ok(indexes)
}

/// Loads a single index with its columns.
async fn load_index<C: Catalog>(catalog: &C, row: &IndexRow) -> Result<Index, QueryError> {
    let index_oid = Oid::new(row.oid);
    let columns = load_index_columns(catalog, index_oid).await?;

    Ok(Index {
        oid: index_oid,
        name: IndexName::try_new(row.name.clone())?,
        method: IndexMethod::try_from(row.method.as_str())?,
        is_unique: row.is_unique,
        is_constraint_index: row.is_constraint_index,
        columns,
        predicate: row.predicate.clone().map(SqlExpr::new),
        comment: row.comment.clone().map(Comment::new),
    })
}

/// Loads column details for an index.
async fn load_index_columns<C: Catalog>(
    catalog: &C,
    index_oid: Oid,
) -> Result<Vec<IndexColumn>, QueryError> {
    let rows = catalog.get_index_columns(index_oid).await?;

    rows.into_iter()
        .map(|r| {
            let order = if r.is_descending {
                SortOrder::Descending
            } else {
                SortOrder::Ascending
            };

            let nulls = if r.nulls_first {
                NullsOrder::First
            } else {
                NullsOrder::Last
            };

            Ok(IndexColumn {
                column: r.column_name.map(ColumnName::try_new).transpose()?,
                expression: r.expression.map(SqlExpr::new),
                order,
                nulls,
            })
        })
        .collect()
}

/// Loads all views in a namespace.
async fn load_views<C: Catalog>(catalog: &C, namespace_oid: Oid) -> Result<Vec<View>, QueryError> {
    let rows = catalog.get_views(namespace_oid).await?;

    rows.into_iter()
        .map(|r| {
            Ok(View {
                oid: Oid::new(r.oid),
                name: TableName::try_new(r.name)?,
                definition: SqlExpr::new(r.definition.unwrap_or_default()),
                is_materialized: r.is_materialized,
                comment: r.comment.map(Comment::new),
            })
        })
        .collect()
}

/// Loads all sequences in a namespace.
async fn load_sequences<C: Catalog>(
    catalog: &C,
    namespace_oid: Oid,
) -> Result<Vec<Sequence>, QueryError> {
    let rows = catalog.get_sequences(namespace_oid).await?;

    rows.into_iter()
        .map(|r| {
            Ok(Sequence {
                oid: Oid::new(r.oid),
                name: SequenceName::try_new(r.name)?,
                data_type: TypeInfo {
                    name: TypeName::try_new(r.type_name)?,
                    schema: SchemaName::try_new(r.type_schema)?,
                    formatted: r.formatted_type,
                    is_array: false,
                },
                start_value: r.start_value,
                increment: r.increment,
                min_value: r.min_value,
                max_value: r.max_value,
                cache_size: r.cache_size,
                is_cyclic: r.is_cyclic,
                comment: r.comment.map(Comment::new),
            })
        })
        .collect()
}

/// Loads all enum types in a namespace.
async fn load_enums<C: Catalog>(
    catalog: &C,
    namespace_oid: Oid,
) -> Result<Vec<EnumType>, QueryError> {
    let rows = catalog.get_enums(namespace_oid).await?;

    rows.into_iter()
        .map(|r| {
            Ok(EnumType {
                oid: Oid::new(r.oid),
                name: TypeName::try_new(r.name)?,
                values: r.values,
                comment: r.comment.map(Comment::new),
            })
        })
        .collect()
}

/// Helper: Get column names as a map from attribute numbers.
async fn get_column_names_map<C: Catalog>(
    catalog: &C,
    table_oid: Oid,
    attnums: &[i16],
) -> Result<HashMap<i16, String>, QueryError> {
    let mappings = catalog.get_column_names(table_oid, attnums).await?;
    Ok(mappings.into_iter().map(|m| (m.attnum, m.name)).collect())
}

/// Helper: Convert attribute numbers to column names.
fn keys_to_column_names(
    keys: &[i16],
    names: &HashMap<i16, String>,
) -> Result<Vec<ColumnName>, QueryError> {
    keys.iter()
        .map(|&k| {
            names
                .get(&k)
                .ok_or(QueryError::MissingField("column name for attnum"))
                .and_then(|n| ColumnName::try_new(n.clone()).map_err(QueryError::from))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::query::FakeCatalog;
    use crate::db::query::catalog::{ColumnRow, NamespaceRow, TableRow};

    #[tokio::test]
    async fn load_empty_namespace() {
        let catalog = FakeCatalog::new().with_namespace(NamespaceRow {
            oid: 100,
            name: "public".to_string(),
            comment: None,
        });

        let ns = load_namespace(&catalog, "public").await.unwrap();

        assert_eq!(ns.oid, Oid::new(100));
        assert_eq!(ns.name.as_ref(), "public");
        assert!(ns.tables.is_empty());
        assert!(ns.views.is_empty());
        assert!(ns.sequences.is_empty());
        assert!(ns.enums.is_empty());
    }

    #[tokio::test]
    async fn load_namespace_not_found() {
        let catalog = FakeCatalog::new();
        let result = load_namespace(&catalog, "nonexistent").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            QueryError::NamespaceNotFound(_)
        ));
    }

    #[tokio::test]
    async fn load_namespace_with_table() {
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: Some("User accounts".to_string()),
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "integer".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 2,
                    name: "email".to_string(),
                    type_name: "text".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "text".to_string(),
                    is_array: false,
                    is_nullable: true,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            );

        let ns = load_namespace(&catalog, "public").await.unwrap();

        assert_eq!(ns.tables.len(), 1);

        let table = &ns.tables[0];
        assert_eq!(table.name.as_ref(), "users");
        assert_eq!(
            table.comment.as_ref().map(|c| c.as_ref()),
            Some("User accounts")
        );
        assert_eq!(table.columns.len(), 2);

        let id_col = &table.columns[0];
        assert_eq!(id_col.name.as_ref(), "id");
        assert_eq!(id_col.type_info.formatted, "integer");
        assert!(!id_col.is_nullable);

        let email_col = &table.columns[1];
        assert_eq!(email_col.name.as_ref(), "email");
        assert!(email_col.is_nullable);
    }

    #[tokio::test]
    async fn load_column_with_default() {
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "status".to_string(),
                    type_name: "text".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "text".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: Some("'active'::text".to_string()),
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            );

        let ns = load_namespace(&catalog, "public").await.unwrap();
        let col = &ns.tables[0].columns[0];

        assert!(col.default.is_some());
        assert_eq!(col.default.as_ref().unwrap().as_ref(), "'active'::text");
    }

    #[tokio::test]
    async fn load_identity_column() {
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "id".to_string(),
                    type_name: "int8".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "bigint".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: Some('a'), // GENERATED ALWAYS
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            );

        let ns = load_namespace(&catalog, "public").await.unwrap();
        let col = &ns.tables[0].columns[0];

        assert_eq!(col.identity, Some(IdentityKind::Always));
    }

    #[tokio::test]
    async fn load_generated_column() {
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "products".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "total".to_string(),
                    type_name: "numeric".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "numeric".to_string(),
                    is_array: false,
                    is_nullable: true,
                    default_expr: Some("(price * quantity)".to_string()),
                    generated_kind: Some('s'), // STORED
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            );

        let ns = load_namespace(&catalog, "public").await.unwrap();
        let col = &ns.tables[0].columns[0];

        assert!(col.generated.is_some());
        let generated = col.generated.as_ref().unwrap();
        assert_eq!(generated.expression.as_ref(), "(price * quantity)");
        assert_eq!(generated.storage, GeneratedStorage::Stored);
        assert!(col.default.is_none()); // default should be None for generated columns
    }

    // =========================================================================
    // diff_from_empty tests
    // =========================================================================

    #[tokio::test]
    async fn diff_from_empty_with_empty_database() {
        let catalog = FakeCatalog::new().with_namespace(NamespaceRow {
            oid: 100,
            name: "public".to_string(),
            comment: None,
        });

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        // When the database is empty, the diff should show no changes
        assert!(diff.is_empty());
        assert!(diff.tables.added.is_empty());
        assert!(diff.tables.removed.is_empty());
        assert!(diff.tables.modified.is_empty());
        assert!(diff.tables.potential_renames.is_empty());
    }

    #[tokio::test]
    async fn diff_from_empty_shows_tables_as_added() {
        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "integer".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            )
            .with_table(
                100,
                TableRow {
                    oid: 201,
                    name: "orders".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            )
            .with_column(
                201,
                ColumnRow {
                    position: 1,
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "integer".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            );

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        // Tables should appear as "added" (relative to empty schema)
        assert!(!diff.is_empty());
        assert_eq!(diff.tables.added.len(), 2);
        assert!(diff.tables.removed.is_empty());
        assert!(diff.tables.modified.is_empty());
        assert!(diff.tables.potential_renames.is_empty());

        // Verify the table details
        let table_names: Vec<_> = diff.tables.added.iter().map(|t| t.name.as_ref()).collect();
        assert!(table_names.contains(&"users"));
        assert!(table_names.contains(&"orders"));
    }

    #[tokio::test]
    async fn diff_from_empty_shows_views_as_added() {
        use crate::db::query::catalog::ViewRow;

        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_view(
                100,
                ViewRow {
                    oid: 300,
                    name: "active_users".to_string(),
                    is_materialized: false,
                    definition: Some("SELECT * FROM users WHERE active".to_string()),
                    comment: None,
                },
            );

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        assert!(!diff.is_empty());
        assert_eq!(diff.views.added.len(), 1);
        assert!(diff.views.removed.is_empty());
        assert_eq!(diff.views.added[0].name.as_ref(), "active_users");
        assert!(!diff.views.added[0].is_materialized);
    }

    #[tokio::test]
    async fn diff_from_empty_shows_sequences_as_added() {
        use crate::db::query::catalog::SequenceRow;

        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_sequence(
                100,
                SequenceRow {
                    oid: 400,
                    name: "users_id_seq".to_string(),
                    type_name: "int8".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "bigint".to_string(),
                    start_value: 1,
                    increment: 1,
                    min_value: 1,
                    max_value: i64::MAX,
                    cache_size: 1,
                    is_cyclic: false,
                    comment: None,
                },
            );

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        assert!(!diff.is_empty());
        assert_eq!(diff.sequences.added.len(), 1);
        assert!(diff.sequences.removed.is_empty());
        assert_eq!(diff.sequences.added[0].name.as_ref(), "users_id_seq");
        assert_eq!(diff.sequences.added[0].increment, 1);
    }

    #[tokio::test]
    async fn diff_from_empty_shows_enums_as_added() {
        use crate::db::query::catalog::EnumRow;

        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: None,
            })
            .with_enum(
                100,
                EnumRow {
                    oid: 500,
                    name: "status".to_string(),
                    values: vec![
                        "pending".to_string(),
                        "active".to_string(),
                        "completed".to_string(),
                    ],
                    comment: None,
                },
            );

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        assert!(!diff.is_empty());
        assert_eq!(diff.enums.added.len(), 1);
        assert!(diff.enums.removed.is_empty());
        assert_eq!(diff.enums.added[0].name.as_ref(), "status");
        assert_eq!(diff.enums.added[0].values.len(), 3);
    }

    #[tokio::test]
    async fn diff_from_empty_with_complex_schema() {
        use crate::db::query::catalog::{EnumRow, SequenceRow, ViewRow};

        let catalog = FakeCatalog::new()
            .with_namespace(NamespaceRow {
                oid: 100,
                name: "public".to_string(),
                comment: Some("Main schema".to_string()),
            })
            // Table with columns
            .with_table(
                100,
                TableRow {
                    oid: 200,
                    name: "users".to_string(),
                    relkind: 'r',
                    comment: Some("User accounts".to_string()),
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 1,
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "integer".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            )
            .with_column(
                200,
                ColumnRow {
                    position: 2,
                    name: "email".to_string(),
                    type_name: "text".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "text".to_string(),
                    is_array: false,
                    is_nullable: false,
                    default_expr: None,
                    generated_kind: None,
                    identity_kind: None,
                    collation_schema: "pg_catalog".to_string(),
                    collation_name: "default".to_string(),
                    comment: None,
                },
            )
            // View
            .with_view(
                100,
                ViewRow {
                    oid: 300,
                    name: "active_users".to_string(),
                    is_materialized: false,
                    definition: Some("SELECT * FROM users WHERE active".to_string()),
                    comment: None,
                },
            )
            // Sequence
            .with_sequence(
                100,
                SequenceRow {
                    oid: 400,
                    name: "users_id_seq".to_string(),
                    type_name: "int8".to_string(),
                    type_schema: "pg_catalog".to_string(),
                    formatted_type: "bigint".to_string(),
                    start_value: 1,
                    increment: 1,
                    min_value: 1,
                    max_value: i64::MAX,
                    cache_size: 1,
                    is_cyclic: false,
                    comment: None,
                },
            )
            // Enum
            .with_enum(
                100,
                EnumRow {
                    oid: 500,
                    name: "user_status".to_string(),
                    values: vec!["active".to_string(), "inactive".to_string()],
                    comment: None,
                },
            );

        let diff = diff_from_empty(&catalog, "public").await.unwrap();

        // All objects should appear as added
        assert!(!diff.is_empty());
        assert_eq!(diff.tables.added.len(), 1);
        assert_eq!(diff.views.added.len(), 1);
        assert_eq!(diff.sequences.added.len(), 1);
        assert_eq!(diff.enums.added.len(), 1);

        // No removed, modified, or potential renames
        assert!(diff.tables.removed.is_empty());
        assert!(diff.tables.modified.is_empty());
        assert!(diff.tables.potential_renames.is_empty());

        // Verify the table has its columns
        let users_table = &diff.tables.added[0];
        assert_eq!(users_table.name.as_ref(), "users");
        assert_eq!(users_table.columns.len(), 2);
    }

    #[tokio::test]
    async fn diff_from_empty_namespace_not_found() {
        let catalog = FakeCatalog::new();
        let result = diff_from_empty(&catalog, "nonexistent").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            QueryError::NamespaceNotFound(_)
        ));
    }

    // =========================================================================
    // Namespace::empty tests
    // =========================================================================

    #[test]
    fn namespace_empty_creates_valid_namespace() {
        let ns = Namespace::empty("public");

        assert_eq!(ns.oid, Oid::default());
        assert_eq!(ns.name.as_ref(), "public");
        assert!(ns.tables.is_empty());
        assert!(ns.views.is_empty());
        assert!(ns.sequences.is_empty());
        assert!(ns.enums.is_empty());
        assert!(ns.comment.is_none());
    }

    #[test]
    fn namespace_empty_works_with_different_schema_names() {
        let ns1 = Namespace::empty("public");
        let ns2 = Namespace::empty("myschema");
        let ns3 = Namespace::empty("my_schema_123");

        assert_eq!(ns1.name.as_ref(), "public");
        assert_eq!(ns2.name.as_ref(), "myschema");
        assert_eq!(ns3.name.as_ref(), "my_schema_123");
    }

    #[test]
    #[should_panic(expected = "schema name must not be empty")]
    fn namespace_empty_panics_on_empty_name() {
        let _ = Namespace::empty("");
    }
}
