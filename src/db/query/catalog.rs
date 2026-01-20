//! Catalog trait and row types for schema introspection.
//!
//! This module defines the `Catalog` trait which abstracts database queries,
//! following the sans-I/O design principle. This allows the schema loading
//! logic to be tested without a real database connection.

use async_trait::async_trait;

use crate::db::schema::Oid;

use super::error::QueryError;

// =============================================================================
// Row Types (Data Transfer Objects)
// =============================================================================

/// Namespace (schema) metadata from the catalog.
#[derive(Debug, Clone)]
pub struct NamespaceRow {
    pub oid: u32,
    pub name: String,
    pub comment: Option<String>,
}

/// Table metadata from the catalog.
#[derive(Debug, Clone)]
pub struct TableRow {
    pub oid: u32,
    pub name: String,
    pub relkind: char,
    pub comment: Option<String>,
}

/// Column metadata from the catalog.
#[derive(Debug, Clone)]
pub struct ColumnRow {
    pub position: i16,
    pub name: String,
    pub type_name: String,
    pub type_schema: String,
    pub formatted_type: String,
    pub is_array: bool,
    pub is_nullable: bool,
    pub default_expr: Option<String>,
    pub generated_kind: Option<char>,
    pub identity_kind: Option<char>,
    pub collation_schema: String,
    pub collation_name: String,
    pub comment: Option<String>,
}

/// Constraint metadata from the catalog.
#[derive(Debug, Clone)]
pub struct ConstraintRow {
    pub oid: u32,
    pub name: String,
    pub constraint_type: char,
    pub columns: Option<Vec<i16>>,
    pub foreign_columns: Option<Vec<i16>>,
    pub foreign_table_oid: Option<u32>,
    pub foreign_schema: Option<String>,
    pub foreign_table: Option<String>,
    pub on_delete: Option<char>,
    pub on_update: Option<char>,
    pub is_deferrable: bool,
    pub is_initially_deferred: bool,
    pub check_expression: Option<String>,
    pub is_no_inherit: bool,
    pub index_name: Option<String>,
    pub index_method: Option<String>,
    pub index_predicate: Option<String>,
    pub comment: Option<String>,
}

/// Exclusion constraint element from the catalog.
#[derive(Debug, Clone)]
pub struct ExclusionElementRow {
    pub expression: String,
    pub operator: String,
}

/// Index metadata from the catalog.
#[derive(Debug, Clone)]
pub struct IndexRow {
    pub oid: u32,
    pub name: String,
    pub method: String,
    pub is_unique: bool,
    pub is_constraint_index: bool,
    pub predicate: Option<String>,
    pub comment: Option<String>,
}

/// Index column metadata from the catalog.
#[derive(Debug, Clone)]
pub struct IndexColumnRow {
    pub column_name: Option<String>,
    pub expression: Option<String>,
    pub is_descending: bool,
    pub nulls_first: bool,
}

/// View metadata from the catalog.
#[derive(Debug, Clone)]
pub struct ViewRow {
    pub oid: u32,
    pub name: String,
    pub is_materialized: bool,
    pub definition: Option<String>,
    pub comment: Option<String>,
}

/// Sequence metadata from the catalog.
#[derive(Debug, Clone)]
pub struct SequenceRow {
    pub oid: u32,
    pub name: String,
    pub type_name: String,
    pub type_schema: String,
    pub formatted_type: String,
    pub start_value: i64,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cache_size: i64,
    pub is_cyclic: bool,
    pub comment: Option<String>,
}

/// Enum type metadata from the catalog.
#[derive(Debug, Clone)]
pub struct EnumRow {
    pub oid: u32,
    pub name: String,
    pub values: Vec<String>,
    pub comment: Option<String>,
}

/// Column name mapping for constraint resolution.
#[derive(Debug, Clone)]
pub struct ColumnNameMapping {
    pub attnum: i16,
    pub name: String,
}

// =============================================================================
// Catalog Trait
// =============================================================================

/// Abstraction over PostgreSQL catalog queries.
///
/// This trait defines the interface for loading schema metadata from the
/// database catalog. It allows the schema loading logic to be tested with
/// fake implementations that don't require a real database.
///
/// # Implementation Notes
///
/// Implementors should return data exactly as it appears in the PostgreSQL
/// catalog tables, letting the loader handle all business logic and
/// transformation to domain types.
#[async_trait]
pub trait Catalog: Send + Sync {
    /// Fetches namespace (schema) metadata by name.
    ///
    /// Returns `None` if the namespace doesn't exist.
    async fn get_namespace(&self, name: &str) -> Result<Option<NamespaceRow>, QueryError>;

    /// Fetches all tables in a namespace.
    async fn get_tables(&self, namespace_oid: Oid) -> Result<Vec<TableRow>, QueryError>;

    /// Fetches all columns for a table.
    async fn get_columns(&self, table_oid: Oid) -> Result<Vec<ColumnRow>, QueryError>;

    /// Fetches all constraints for a table.
    async fn get_constraints(&self, table_oid: Oid) -> Result<Vec<ConstraintRow>, QueryError>;

    /// Fetches exclusion constraint elements.
    async fn get_exclusion_elements(
        &self,
        constraint_oid: Oid,
    ) -> Result<Vec<ExclusionElementRow>, QueryError>;

    /// Fetches column names by attribute numbers for constraint resolution.
    async fn get_column_names(
        &self,
        table_oid: Oid,
        attnums: &[i16],
    ) -> Result<Vec<ColumnNameMapping>, QueryError>;

    /// Fetches all indexes for a table.
    async fn get_indexes(&self, table_oid: Oid) -> Result<Vec<IndexRow>, QueryError>;

    /// Fetches column details for an index.
    async fn get_index_columns(&self, index_oid: Oid) -> Result<Vec<IndexColumnRow>, QueryError>;

    /// Fetches all views in a namespace.
    async fn get_views(&self, namespace_oid: Oid) -> Result<Vec<ViewRow>, QueryError>;

    /// Fetches all sequences in a namespace.
    async fn get_sequences(&self, namespace_oid: Oid) -> Result<Vec<SequenceRow>, QueryError>;

    /// Fetches all enum types in a namespace.
    async fn get_enums(&self, namespace_oid: Oid) -> Result<Vec<EnumRow>, QueryError>;
}
