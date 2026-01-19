//! Fake implementation of the Catalog trait for testing.
//!
//! This module provides an in-memory implementation of the Catalog trait
//! that can be configured with test data, enabling unit testing of the
//! schema loading logic without a real database.

use std::collections::HashMap;

use async_trait::async_trait;

use crate::db::schema::Oid;

use super::catalog::{
    Catalog, ColumnNameMapping, ColumnRow, ConstraintRow, EnumRow, ExclusionElementRow,
    IndexColumnRow, IndexRow, NamespaceRow, SequenceRow, TableRow, ViewRow,
};
use super::error::QueryError;

/// A fake catalog for testing schema loading.
///
/// This implementation stores all data in memory and can be configured
/// with builder methods to set up test scenarios.
///
/// # Example
///
/// ```
/// use tern::db::query::FakeCatalog;
/// use tern::db::query::catalog::{NamespaceRow, TableRow, ColumnRow};
///
/// let catalog = FakeCatalog::new()
///     .with_namespace(NamespaceRow {
///         oid: 12345,
///         name: "public".to_string(),
///         comment: None,
///     })
///     .with_table(12345, TableRow {
///         oid: 12346,
///         name: "users".to_string(),
///         relkind: 'r',
///         comment: None,
///     })
///     .with_column(12346, ColumnRow {
///         position: 1,
///         name: "id".to_string(),
///         type_name: "int4".to_string(),
///         type_schema: "pg_catalog".to_string(),
///         formatted_type: "integer".to_string(),
///         is_array: false,
///         is_nullable: false,
///         default_expr: None,
///         generated_kind: None,
///         identity_kind: None,
///         collation_schema: "pg_catalog".to_string(),
///         collation_name: "default".to_string(),
///         comment: None,
///     });
/// ```
#[derive(Debug, Default)]
pub struct FakeCatalog {
    namespaces: HashMap<String, NamespaceRow>,
    tables: HashMap<u32, Vec<TableRow>>,
    columns: HashMap<u32, Vec<ColumnRow>>,
    constraints: HashMap<u32, Vec<ConstraintRow>>,
    exclusion_elements: HashMap<u32, Vec<ExclusionElementRow>>,
    column_names: HashMap<u32, HashMap<i16, String>>,
    indexes: HashMap<u32, Vec<IndexRow>>,
    index_columns: HashMap<u32, Vec<IndexColumnRow>>,
    views: HashMap<u32, Vec<ViewRow>>,
    sequences: HashMap<u32, Vec<SequenceRow>>,
    enums: HashMap<u32, Vec<EnumRow>>,
}

impl FakeCatalog {
    /// Creates a new empty fake catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a namespace to the catalog.
    pub fn with_namespace(mut self, ns: NamespaceRow) -> Self {
        self.namespaces.insert(ns.name.clone(), ns);
        self
    }

    /// Adds a table to a namespace.
    pub fn with_table(mut self, namespace_oid: u32, table: TableRow) -> Self {
        self.tables.entry(namespace_oid).or_default().push(table);
        self
    }

    /// Adds a column to a table.
    pub fn with_column(mut self, table_oid: u32, column: ColumnRow) -> Self {
        // Also store the column name mapping
        self.column_names
            .entry(table_oid)
            .or_default()
            .insert(column.position, column.name.clone());

        self.columns.entry(table_oid).or_default().push(column);
        self
    }

    /// Adds a constraint to a table.
    pub fn with_constraint(mut self, table_oid: u32, constraint: ConstraintRow) -> Self {
        self.constraints
            .entry(table_oid)
            .or_default()
            .push(constraint);
        self
    }

    /// Adds exclusion elements for a constraint.
    pub fn with_exclusion_elements(
        mut self,
        constraint_oid: u32,
        elements: Vec<ExclusionElementRow>,
    ) -> Self {
        self.exclusion_elements.insert(constraint_oid, elements);
        self
    }

    /// Adds an index to a table.
    pub fn with_index(mut self, table_oid: u32, index: IndexRow) -> Self {
        self.indexes.entry(table_oid).or_default().push(index);
        self
    }

    /// Adds columns to an index.
    pub fn with_index_columns(mut self, index_oid: u32, columns: Vec<IndexColumnRow>) -> Self {
        self.index_columns.insert(index_oid, columns);
        self
    }

    /// Adds a view to a namespace.
    pub fn with_view(mut self, namespace_oid: u32, view: ViewRow) -> Self {
        self.views.entry(namespace_oid).or_default().push(view);
        self
    }

    /// Adds a sequence to a namespace.
    pub fn with_sequence(mut self, namespace_oid: u32, sequence: SequenceRow) -> Self {
        self.sequences
            .entry(namespace_oid)
            .or_default()
            .push(sequence);
        self
    }

    /// Adds an enum type to a namespace.
    pub fn with_enum(mut self, namespace_oid: u32, enum_type: EnumRow) -> Self {
        self.enums.entry(namespace_oid).or_default().push(enum_type);
        self
    }

    /// Adds column name mappings for a table (useful for foreign key resolution).
    pub fn with_column_names(
        mut self,
        table_oid: u32,
        mappings: impl IntoIterator<Item = (i16, String)>,
    ) -> Self {
        let entry = self.column_names.entry(table_oid).or_default();
        for (attnum, name) in mappings {
            entry.insert(attnum, name);
        }
        self
    }
}

#[async_trait]
impl Catalog for FakeCatalog {
    async fn get_namespace(&self, name: &str) -> Result<Option<NamespaceRow>, QueryError> {
        Ok(self.namespaces.get(name).cloned())
    }

    async fn get_tables(&self, namespace_oid: Oid) -> Result<Vec<TableRow>, QueryError> {
        let oid = *namespace_oid.as_ref();
        Ok(self.tables.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_columns(&self, table_oid: Oid) -> Result<Vec<ColumnRow>, QueryError> {
        let oid = *table_oid.as_ref();
        Ok(self.columns.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_constraints(&self, table_oid: Oid) -> Result<Vec<ConstraintRow>, QueryError> {
        let oid = *table_oid.as_ref();
        Ok(self.constraints.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_exclusion_elements(
        &self,
        constraint_oid: Oid,
    ) -> Result<Vec<ExclusionElementRow>, QueryError> {
        let oid = *constraint_oid.as_ref();
        Ok(self
            .exclusion_elements
            .get(&oid)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_column_names(
        &self,
        table_oid: Oid,
        attnums: &[i16],
    ) -> Result<Vec<ColumnNameMapping>, QueryError> {
        let oid = *table_oid.as_ref();
        let names = self.column_names.get(&oid);

        Ok(attnums
            .iter()
            .filter_map(|&attnum| {
                names
                    .and_then(|m| m.get(&attnum))
                    .map(|name| ColumnNameMapping {
                        attnum,
                        name: name.clone(),
                    })
            })
            .collect())
    }

    async fn get_indexes(&self, table_oid: Oid) -> Result<Vec<IndexRow>, QueryError> {
        let oid = *table_oid.as_ref();
        Ok(self.indexes.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_index_columns(&self, index_oid: Oid) -> Result<Vec<IndexColumnRow>, QueryError> {
        let oid = *index_oid.as_ref();
        Ok(self.index_columns.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_views(&self, namespace_oid: Oid) -> Result<Vec<ViewRow>, QueryError> {
        let oid = *namespace_oid.as_ref();
        Ok(self.views.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_sequences(&self, namespace_oid: Oid) -> Result<Vec<SequenceRow>, QueryError> {
        let oid = *namespace_oid.as_ref();
        Ok(self.sequences.get(&oid).cloned().unwrap_or_default())
    }

    async fn get_enums(&self, namespace_oid: Oid) -> Result<Vec<EnumRow>, QueryError> {
        let oid = *namespace_oid.as_ref();
        Ok(self.enums.get(&oid).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_catalog_returns_none_for_namespace() {
        let catalog = FakeCatalog::new();
        let result = catalog.get_namespace("public").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn can_add_and_retrieve_namespace() {
        let catalog = FakeCatalog::new().with_namespace(NamespaceRow {
            oid: 12345,
            name: "public".to_string(),
            comment: Some("public schema".to_string()),
        });

        let result = catalog.get_namespace("public").await.unwrap();
        assert!(result.is_some());

        let ns = result.unwrap();
        assert_eq!(ns.oid, 12345);
        assert_eq!(ns.name, "public");
        assert_eq!(ns.comment, Some("public schema".to_string()));
    }

    #[tokio::test]
    async fn can_add_and_retrieve_tables() {
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
            .with_table(
                100,
                TableRow {
                    oid: 201,
                    name: "orders".to_string(),
                    relkind: 'r',
                    comment: None,
                },
            );

        let tables = catalog.get_tables(Oid::new(100)).await.unwrap();
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].name, "users");
        assert_eq!(tables[1].name, "orders");
    }

    #[tokio::test]
    async fn can_add_and_retrieve_columns() {
        let catalog = FakeCatalog::new().with_column(
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
        );

        let columns = catalog.get_columns(Oid::new(200)).await.unwrap();
        assert_eq!(columns.len(), 1);
        assert_eq!(columns[0].name, "id");
        assert!(!columns[0].is_nullable);
    }

    #[tokio::test]
    async fn column_names_are_tracked_automatically() {
        let catalog = FakeCatalog::new()
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

        let names = catalog
            .get_column_names(Oid::new(200), &[1, 2])
            .await
            .unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[0].attnum, 1);
        assert_eq!(names[0].name, "id");
        assert_eq!(names[1].attnum, 2);
        assert_eq!(names[1].name, "email");
    }
}
