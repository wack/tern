//! Table operation tool implementations.

use crate::db::migrate::Operation;
use crate::db::model::types::TypeInfo;
use crate::db::model::{Column, Table, TableKind};
use crate::db::schema::{CollationName, ColumnName, Oid, SchemaName, TableName, TypeName};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    CreateTableInput, CreateTableOutput, DropTableInput, DropTableOutput, RenameTableInput,
    RenameTableOutput, TernMcpService,
};

/// Implements the create_table tool.
pub async fn create_table(
    service: &TernMcpService,
    input: CreateTableInput,
) -> McpResult<CreateTableOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table already exists
    let namespace = session.current_namespace().await?;
    if namespace
        .tables
        .iter()
        .any(|t| t.name.as_ref() == input.name)
    {
        return Err(McpError::DuplicateObject {
            kind: "Table".to_string(),
            name: input.name,
        });
    }

    // Build SQL
    let mut sql = format!("CREATE TABLE {} (\n", quote_identifier(&input.name));
    let mut column_defs = Vec::new();
    let mut pk_columns: Vec<String> = Vec::new();

    for col in &input.columns {
        let mut col_def = format!("  {} {}", quote_identifier(&col.name), col.r#type);

        if !col.nullable {
            col_def.push_str(" NOT NULL");
        }

        if let Some(ref default) = col.default {
            col_def.push_str(&format!(" DEFAULT {}", default));
        }

        if col.primary_key {
            pk_columns.push(col.name.clone());
        }

        column_defs.push(col_def);
    }

    // Handle explicit primary key
    if let Some(ref pk) = input.primary_key {
        pk_columns = pk.clone();
    }

    // Add primary key constraint if specified
    if !pk_columns.is_empty() {
        let pk_cols: Vec<String> = pk_columns.iter().map(|c| quote_identifier(c)).collect();
        column_defs.push(format!("  PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    sql.push_str(&column_defs.join(",\n"));
    sql.push_str("\n)");

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;

    let table_name =
        TableName::try_new(input.name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    // Build columns for the operation
    let columns: Vec<Column> = input
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            Ok(Column {
                name: ColumnName::try_new(col.name.clone()).map_err(|e| {
                    McpError::InvalidInput {
                        message: e.to_string(),
                    }
                })?,
                position: (i + 1) as i16,
                type_info: TypeInfo {
                    name: TypeName::try_new(col.r#type.clone()).map_err(|e| {
                        McpError::InvalidInput {
                            message: e.to_string(),
                        }
                    })?,
                    schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                    formatted: col.r#type.clone(),
                    is_array: false,
                },
                is_nullable: col.nullable,
                default: col
                    .default
                    .as_ref()
                    .map(|d| crate::db::model::types::SqlExpr::new(d.clone())),
                generated: None,
                identity: None,
                collation: crate::db::model::types::QualifiedCollationName::new(
                    SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                    CollationName::try_new("default".to_string()).unwrap(),
                ),
                comment: None,
            })
        })
        .collect::<McpResult<Vec<_>>>()?;

    let table = Table {
        oid: Oid::default(),
        name: table_name,
        kind: TableKind::Regular,
        columns,
        constraints: vec![],
        indexes: vec![],
        comment: None,
    };

    let operation = Operation::CreateTable {
        schema: schema_name,
        table,
    };

    // Record the operation
    session.apply_operation(&sql, operation).await?;

    Ok(CreateTableOutput {
        success: true,
        table_name: input.name.clone(),
        sql_executed: sql,
        message: format!(
            "Created table '{}' with {} column(s)",
            input.name,
            input.columns.len()
        ),
    })
}

/// Implements the drop_table tool.
pub async fn drop_table(
    service: &TernMcpService,
    input: DropTableInput,
) -> McpResult<DropTableOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table exists
    let namespace = session.current_namespace().await?;
    if !namespace
        .tables
        .iter()
        .any(|t| t.name.as_ref() == input.name)
    {
        return Err(McpError::TableNotFound {
            name: input.name,
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        });
    }

    // Build SQL
    let cascade = if input.cascade { " CASCADE" } else { "" };
    let sql = format!("DROP TABLE {}{}", quote_identifier(&input.name), cascade);

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let operation = Operation::DropTable {
        schema: schema_name,
        name: table_name,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(DropTableOutput {
        success: true,
        table_name: input.name,
        sql_executed: sql,
        warning: "This is a destructive operation that will cause data loss.".to_string(),
    })
}

/// Implements the rename_table tool.
pub async fn rename_table(
    service: &TernMcpService,
    input: RenameTableInput,
) -> McpResult<RenameTableOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if source table exists
    let namespace = session.current_namespace().await?;
    if !namespace
        .tables
        .iter()
        .any(|t| t.name.as_ref() == input.from)
    {
        return Err(McpError::TableNotFound {
            name: input.from,
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        });
    }

    // Check if target table already exists
    if namespace.tables.iter().any(|t| t.name.as_ref() == input.to) {
        return Err(McpError::DuplicateObject {
            kind: "Table".to_string(),
            name: input.to,
        });
    }

    // Build SQL
    let sql = format!(
        "ALTER TABLE {} RENAME TO {}",
        quote_identifier(&input.from),
        quote_identifier(&input.to)
    );

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let from_name = TableName::try_new(input.from.clone()).map_err(|e| McpError::InvalidInput {
        message: e.to_string(),
    })?;
    let to_name = TableName::try_new(input.to.clone()).map_err(|e| McpError::InvalidInput {
        message: e.to_string(),
    })?;

    let operation = Operation::RenameTable {
        schema: schema_name,
        from: from_name,
        to: to_name,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(RenameTableOutput {
        success: true,
        sql_executed: sql,
        message: format!("Renamed table '{}' to '{}'", input.from, input.to),
    })
}

/// Quotes a PostgreSQL identifier if needed.
fn quote_identifier(name: &str) -> String {
    // For simplicity, always quote identifiers to handle reserved words
    // A more sophisticated implementation would check if quoting is necessary
    format!("\"{}\"", name.replace('"', "\"\""))
}
