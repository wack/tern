//! Schema inspection tool implementations.

use crate::db::model::ConstraintKind;
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    ColumnInfo, ConstraintInfo, DescribeTableInput, DescribeTableOutput, EnumInfo, ForeignKeyInfo,
    IndexDetailInfo, IndexInfo, ListEnumsOutput, ListIndexesOutput, ListTablesInput,
    ListTablesOutput, TableInfo, TernMcpService,
};

/// Implements the list_tables tool.
pub async fn list_tables(
    service: &TernMcpService,
    input: ListTablesInput,
) -> McpResult<ListTablesOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    let namespace = session.current_namespace().await?;

    let tables = namespace
        .tables
        .iter()
        .map(|table| TableInfo {
            name: table.name.as_ref().to_string(),
            columns: if input.include_columns {
                Some(
                    table
                        .columns
                        .iter()
                        .map(|c| c.name.as_ref().to_string())
                        .collect(),
                )
            } else {
                None
            },
        })
        .collect();

    Ok(ListTablesOutput { tables })
}

/// Implements the describe_table tool.
pub async fn describe_table(
    service: &TernMcpService,
    input: DescribeTableInput,
) -> McpResult<DescribeTableOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    let namespace = session.current_namespace().await?;

    // Find the table
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table_name)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table_name.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    // Get primary key columns
    let pk_columns: Vec<String> = table
        .constraints
        .iter()
        .filter_map(|c| match &c.kind {
            ConstraintKind::PrimaryKey(pk) => Some(
                pk.columns
                    .iter()
                    .map(|col| col.as_ref().to_string())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();

    // Build column info
    let columns = table
        .columns
        .iter()
        .map(|col| ColumnInfo {
            name: col.name.as_ref().to_string(),
            r#type: col.type_info.formatted.clone(),
            nullable: col.is_nullable,
            default: col.default.as_ref().map(|d| d.as_ref().to_string()),
            is_primary_key: pk_columns.contains(&col.name.as_ref().to_string()),
        })
        .collect();

    // Build constraint info
    let constraints = table
        .constraints
        .iter()
        .map(|c| {
            let (constraint_type, columns) = match &c.kind {
                ConstraintKind::PrimaryKey(pk) => (
                    "primary_key".to_string(),
                    pk.columns
                        .iter()
                        .map(|col| col.as_ref().to_string())
                        .collect(),
                ),
                ConstraintKind::ForeignKey(fk) => (
                    "foreign_key".to_string(),
                    fk.columns
                        .iter()
                        .map(|col| col.as_ref().to_string())
                        .collect(),
                ),
                ConstraintKind::Unique(u) => (
                    "unique".to_string(),
                    u.columns
                        .iter()
                        .map(|col| col.as_ref().to_string())
                        .collect(),
                ),
                ConstraintKind::Check(_) => ("check".to_string(), vec![]),
                ConstraintKind::Exclusion(_) => ("exclusion".to_string(), vec![]),
            };
            ConstraintInfo {
                name: c.name.as_ref().to_string(),
                r#type: constraint_type,
                columns,
            }
        })
        .collect();

    // Build index info
    let indexes = table
        .indexes
        .iter()
        .map(|idx| {
            let is_primary = table.constraints.iter().any(|c| {
                matches!(&c.kind, ConstraintKind::PrimaryKey(_))
                    && c.name.as_ref() == idx.name.as_ref()
            });
            IndexInfo {
                name: idx.name.as_ref().to_string(),
                columns: idx
                    .columns
                    .iter()
                    .filter_map(|col| col.column.as_ref().map(|c| c.as_ref().to_string()))
                    .collect(),
                is_unique: idx.is_unique,
                is_primary,
            }
        })
        .collect();

    // Find incoming foreign keys from other tables
    let foreign_keys_incoming = namespace
        .tables
        .iter()
        .filter(|t| t.name.as_ref() != input.table_name)
        .flat_map(|other_table| {
            other_table
                .constraints
                .iter()
                .filter_map(|c| match &c.kind {
                    ConstraintKind::ForeignKey(fk)
                        if fk.referenced_table.name.as_ref() == input.table_name =>
                    {
                        Some(ForeignKeyInfo {
                            from_table: other_table.name.as_ref().to_string(),
                            from_columns: fk
                                .columns
                                .iter()
                                .map(|col| col.as_ref().to_string())
                                .collect(),
                            to_columns: fk
                                .referenced_columns
                                .iter()
                                .map(|col| col.as_ref().to_string())
                                .collect(),
                            on_delete: fk.on_delete.as_sql().to_string(),
                        })
                    }
                    _ => None,
                })
        })
        .collect();

    Ok(DescribeTableOutput {
        name: table.name.as_ref().to_string(),
        columns,
        constraints,
        indexes,
        foreign_keys_incoming,
    })
}

/// Implements the list_enums tool.
pub async fn list_enums(service: &TernMcpService) -> McpResult<ListEnumsOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    let namespace = session.current_namespace().await?;

    let enums = namespace
        .enums
        .iter()
        .map(|e| EnumInfo {
            name: e.name.as_ref().to_string(),
            values: e.values.clone(),
        })
        .collect();

    Ok(ListEnumsOutput { enums })
}

/// Implements the list_indexes tool.
pub async fn list_indexes(service: &TernMcpService) -> McpResult<ListIndexesOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    let namespace = session.current_namespace().await?;

    let mut indexes = Vec::new();

    for table in &namespace.tables {
        for idx in &table.indexes {
            indexes.push(IndexDetailInfo {
                name: idx.name.as_ref().to_string(),
                table: table.name.as_ref().to_string(),
                columns: idx
                    .columns
                    .iter()
                    .filter_map(|col| col.column.as_ref().map(|c| c.as_ref().to_string()))
                    .collect(),
                is_unique: idx.is_unique,
                method: idx.method.as_str().to_string(),
            });
        }
    }

    Ok(ListIndexesOutput { indexes })
}
