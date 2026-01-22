//! PostgreSQL implementation of the Catalog trait.
//!
//! This module provides the real database implementation that queries
//! PostgreSQL's system catalogs using tokio-postgres.

use async_trait::async_trait;
use tokio_postgres::Client;

use crate::db::schema::Oid;

use super::catalog::{
    Catalog, ColumnNameMapping, ColumnRow, ConstraintRow, EnumRow, ExclusionElementRow,
    IndexColumnRow, IndexRow, NamespaceRow, SequenceRow, TableRow, ViewRow,
};
use super::error::QueryError;
use super::sql;

/// PostgreSQL catalog implementation using tokio-postgres.
///
/// This is the production implementation that executes real queries
/// against a PostgreSQL database.
pub struct PostgresCatalog<'a> {
    client: &'a Client,
}

impl<'a> PostgresCatalog<'a> {
    /// Creates a new PostgreSQL catalog wrapper.
    pub fn new(client: &'a Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Catalog for PostgresCatalog<'_> {
    async fn get_namespace(&self, name: &str) -> Result<Option<NamespaceRow>, QueryError> {
        let row = self
            .client
            .query_opt(sql::NAMESPACE_BY_NAME, &[&name])
            .await?;

        Ok(row.map(|r| NamespaceRow {
            oid: r.get("oid"),
            name: r.get("nspname"),
            comment: r.get("comment"),
        }))
    }

    async fn get_tables(&self, namespace_oid: Oid) -> Result<Vec<TableRow>, QueryError> {
        let oid_val: u32 = *namespace_oid.as_ref();
        let rows = self
            .client
            .query(sql::TABLES_IN_NAMESPACE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let relkind: i8 = r.get("relkind");
                TableRow {
                    oid: r.get("oid"),
                    name: r.get("relname"),
                    relkind: relkind as u8 as char,
                    comment: r.get("comment"),
                }
            })
            .collect())
    }

    async fn get_columns(&self, table_oid: Oid) -> Result<Vec<ColumnRow>, QueryError> {
        let oid_val: u32 = *table_oid.as_ref();
        let rows = self
            .client
            .query(sql::COLUMNS_FOR_TABLE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                // PostgreSQL `char` type (single byte) maps to i8 in Rust
                let generated_kind_i8: Option<i8> = r.get("generated_kind");
                let identity_kind_i8: Option<i8> = r.get("identity_kind");

                ColumnRow {
                    position: r.get("position"),
                    name: r.get("name"),
                    type_name: r.get("type_name"),
                    type_schema: r.get("type_schema"),
                    formatted_type: r.get("formatted_type"),
                    is_array: r.get("is_array"),
                    is_nullable: r.get("is_nullable"),
                    default_expr: r.get("default_expr"),
                    generated_kind: generated_kind_i8.and_then(|i| {
                        if i == 0 {
                            None // Empty char (NULL byte) means not generated
                        } else {
                            Some(i as u8 as char)
                        }
                    }),
                    identity_kind: identity_kind_i8.and_then(|i| {
                        if i == 0 {
                            None // Empty char (NULL byte) means no identity
                        } else {
                            Some(i as u8 as char)
                        }
                    }),
                    collation_schema: r.get("collation_schema"),
                    collation_name: r.get("collation_name"),
                    comment: r.get("comment"),
                }
            })
            .collect())
    }

    async fn get_constraints(&self, table_oid: Oid) -> Result<Vec<ConstraintRow>, QueryError> {
        let oid_val: u32 = *table_oid.as_ref();
        let rows = self
            .client
            .query(sql::CONSTRAINTS_FOR_TABLE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| {
                let con_type: i8 = r.get("contype");
                let del_type: Option<i8> = r.get::<_, Option<i8>>("confdeltype");
                let upd_type: Option<i8> = r.get::<_, Option<i8>>("confupdtype");

                ConstraintRow {
                    oid: r.get("oid"),
                    name: r.get("conname"),
                    constraint_type: con_type as u8 as char,
                    columns: r.get("conkey"),
                    foreign_columns: r.get("confkey"),
                    foreign_table_oid: r.get::<_, Option<u32>>("confrelid"),
                    foreign_schema: r.get("ref_schema"),
                    foreign_table: r.get("ref_table"),
                    on_delete: del_type.map(|c| c as u8 as char),
                    on_update: upd_type.map(|c| c as u8 as char),
                    is_deferrable: r.get("condeferrable"),
                    is_initially_deferred: r.get("condeferred"),
                    check_expression: r.get("check_expr"),
                    is_no_inherit: r.get("connoinherit"),
                    index_name: r.get("index_name"),
                    index_method: r.get("index_method"),
                    index_predicate: r.get("index_predicate"),
                    comment: r.get("comment"),
                }
            })
            .collect())
    }

    async fn get_exclusion_elements(
        &self,
        constraint_oid: Oid,
    ) -> Result<Vec<ExclusionElementRow>, QueryError> {
        let oid_val: u32 = *constraint_oid.as_ref();
        let rows = self
            .client
            .query(sql::EXCLUSION_ELEMENTS, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| ExclusionElementRow {
                expression: r.get("expression"),
                operator: r.get("operator"),
            })
            .collect())
    }

    async fn get_column_names(
        &self,
        table_oid: Oid,
        attnums: &[i16],
    ) -> Result<Vec<ColumnNameMapping>, QueryError> {
        let oid_val: u32 = *table_oid.as_ref();
        let rows = self
            .client
            .query(sql::COLUMN_NAMES_BY_ATTNUM, &[&oid_val, &attnums])
            .await?;

        Ok(rows
            .iter()
            .map(|r| ColumnNameMapping {
                attnum: r.get("attnum"),
                name: r.get("attname"),
            })
            .collect())
    }

    async fn get_indexes(&self, table_oid: Oid) -> Result<Vec<IndexRow>, QueryError> {
        let oid_val: u32 = *table_oid.as_ref();
        let rows = self
            .client
            .query(sql::INDEXES_FOR_TABLE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| IndexRow {
                oid: r.get("oid"),
                name: r.get("name"),
                method: r.get("method"),
                is_unique: r.get("is_unique"),
                is_constraint_index: r.get("is_constraint_index"),
                predicate: r.get("predicate"),
                comment: r.get("comment"),
            })
            .collect())
    }

    async fn get_index_columns(&self, index_oid: Oid) -> Result<Vec<IndexColumnRow>, QueryError> {
        let oid_val: u32 = *index_oid.as_ref();
        let rows = self.client.query(sql::INDEX_COLUMNS, &[&oid_val]).await?;

        Ok(rows
            .iter()
            .map(|r| IndexColumnRow {
                column_name: r.get("column_name"),
                expression: r.get("expression"),
                is_descending: r.get("is_desc"),
                nulls_first: r.get("nulls_first"),
            })
            .collect())
    }

    async fn get_views(&self, namespace_oid: Oid) -> Result<Vec<ViewRow>, QueryError> {
        let oid_val: u32 = *namespace_oid.as_ref();
        let rows = self
            .client
            .query(sql::VIEWS_IN_NAMESPACE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| ViewRow {
                oid: r.get("oid"),
                name: r.get("relname"),
                is_materialized: r.get("is_materialized"),
                definition: r.get("definition"),
                comment: r.get("comment"),
            })
            .collect())
    }

    async fn get_sequences(&self, namespace_oid: Oid) -> Result<Vec<SequenceRow>, QueryError> {
        let oid_val: u32 = *namespace_oid.as_ref();
        let rows = self
            .client
            .query(sql::SEQUENCES_IN_NAMESPACE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| SequenceRow {
                oid: r.get("oid"),
                name: r.get("relname"),
                type_name: r.get("type_name"),
                type_schema: r.get("type_schema"),
                formatted_type: r.get("formatted_type"),
                start_value: r.get("start_value"),
                increment: r.get("increment"),
                min_value: r.get("min_value"),
                max_value: r.get("max_value"),
                cache_size: r.get("cache_size"),
                is_cyclic: r.get("is_cyclic"),
                comment: r.get("comment"),
            })
            .collect())
    }

    async fn get_enums(&self, namespace_oid: Oid) -> Result<Vec<EnumRow>, QueryError> {
        let oid_val: u32 = *namespace_oid.as_ref();
        let rows = self
            .client
            .query(sql::ENUMS_IN_NAMESPACE, &[&oid_val])
            .await?;

        Ok(rows
            .iter()
            .map(|r| EnumRow {
                oid: r.get("oid"),
                name: r.get("typname"),
                values: r.get("values"),
                comment: r.get("comment"),
            })
            .collect())
    }
}
