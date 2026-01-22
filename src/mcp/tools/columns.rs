//! Column operation tool implementations.

use crate::db::migrate::{ColumnChanges, DefaultChange, Operation, SetColumnType};
use crate::db::model::Column;
use crate::db::model::types::{QualifiedCollationName, SqlExpr, TypeInfo};
use crate::db::schema::{CollationName, ColumnName, SchemaName, TableName, TypeName};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    AddColumnInput, AddColumnOutput, AlterColumnDefaultInput, AlterColumnDefaultOutput,
    AlterColumnNullableInput, AlterColumnNullableOutput, AlterColumnTypeInput,
    AlterColumnTypeOutput, DropColumnInput, DropColumnOutput, RenameColumnInput,
    RenameColumnOutput, TernMcpService,
};

/// Quotes a PostgreSQL identifier if needed.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Implements the add_column tool.
pub async fn add_column(
    service: &TernMcpService,
    input: AddColumnInput,
) -> McpResult<AddColumnOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table exists
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    // Check if column already exists
    if table
        .columns
        .iter()
        .any(|c| c.name.as_ref() == input.column.name)
    {
        return Err(McpError::DuplicateObject {
            kind: "Column".to_string(),
            name: format!("{}.{}", input.table, input.column.name),
        });
    }

    // Build SQL
    let mut sql = format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        quote_identifier(&input.table),
        quote_identifier(&input.column.name),
        input.column.r#type
    );

    if !input.column.nullable {
        sql.push_str(" NOT NULL");
    }

    if let Some(ref default) = input.column.default {
        sql.push_str(&format!(" DEFAULT {}", default));
    }

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let column = Column {
        name: ColumnName::try_new(input.column.name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        position: (table.columns.len() + 1) as i16,
        type_info: TypeInfo {
            name: TypeName::try_new(input.column.r#type.clone()).map_err(|e| {
                McpError::InvalidInput {
                    message: e.to_string(),
                }
            })?,
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: input.column.r#type.clone(),
            is_array: false,
        },
        is_nullable: input.column.nullable,
        default: input
            .column
            .default
            .as_ref()
            .map(|d| SqlExpr::new(d.clone())),
        generated: None,
        identity: None,
        collation: QualifiedCollationName::new(
            SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            CollationName::try_new("default".to_string()).unwrap(),
        ),
        comment: None,
    };

    let operation = Operation::AddColumn {
        schema: schema_name,
        table: table_name,
        column,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddColumnOutput {
        success: true,
        sql_executed: sql,
        message: format!(
            "Added column '{}' to table '{}'",
            input.column.name, input.table
        ),
    })
}

/// Implements the drop_column tool.
pub async fn drop_column(
    service: &TernMcpService,
    input: DropColumnInput,
) -> McpResult<DropColumnOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table exists
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    // Check if column exists
    if !table
        .columns
        .iter()
        .any(|c| c.name.as_ref() == input.column)
    {
        return Err(McpError::ColumnNotFound {
            table: input.table,
            column: input.column,
        });
    }

    // Build SQL
    let sql = format!(
        "ALTER TABLE {} DROP COLUMN {}",
        quote_identifier(&input.table),
        quote_identifier(&input.column)
    );

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let column_name =
        ColumnName::try_new(input.column.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let operation = Operation::DropColumn {
        schema: schema_name,
        table: table_name,
        name: column_name,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(DropColumnOutput {
        success: true,
        sql_executed: sql,
        warning: "This is a destructive operation that will cause data loss.".to_string(),
    })
}

/// Implements the rename_column tool.
pub async fn rename_column(
    service: &TernMcpService,
    input: RenameColumnInput,
) -> McpResult<RenameColumnOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table exists
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    // Check if source column exists
    if !table.columns.iter().any(|c| c.name.as_ref() == input.from) {
        return Err(McpError::ColumnNotFound {
            table: input.table,
            column: input.from,
        });
    }

    // Check if target column already exists
    if table.columns.iter().any(|c| c.name.as_ref() == input.to) {
        return Err(McpError::DuplicateObject {
            kind: "Column".to_string(),
            name: format!("{}.{}", input.table, input.to),
        });
    }

    // Build SQL
    let sql = format!(
        "ALTER TABLE {} RENAME COLUMN {} TO {}",
        quote_identifier(&input.table),
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
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let from_name =
        ColumnName::try_new(input.from.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let to_name = ColumnName::try_new(input.to.clone()).map_err(|e| McpError::InvalidInput {
        message: e.to_string(),
    })?;

    let operation = Operation::RenameColumn {
        schema: schema_name,
        table: table_name,
        from: from_name,
        to: to_name,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(RenameColumnOutput {
        success: true,
        sql_executed: sql,
        message: format!(
            "Renamed column '{}.{}' to '{}'",
            input.table, input.from, input.to
        ),
    })
}

/// Implements the alter_column_type tool.
pub async fn alter_column_type(
    service: &TernMcpService,
    input: AlterColumnTypeInput,
) -> McpResult<AlterColumnTypeOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table and column exist
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    if !table
        .columns
        .iter()
        .any(|c| c.name.as_ref() == input.column)
    {
        return Err(McpError::ColumnNotFound {
            table: input.table,
            column: input.column,
        });
    }

    // Build SQL
    let mut sql = format!(
        "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
        quote_identifier(&input.table),
        quote_identifier(&input.column),
        input.new_type
    );

    if let Some(ref using) = input.using {
        sql.push_str(&format!(" USING {}", using));
    }

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let column_name =
        ColumnName::try_new(input.column.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let changes = ColumnChanges {
        set_type: Some(SetColumnType {
            type_info: TypeInfo {
                name: TypeName::try_new(input.new_type.clone()).map_err(|e| {
                    McpError::InvalidInput {
                        message: e.to_string(),
                    }
                })?,
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: input.new_type,
                is_array: false,
            },
            using: input.using.map(SqlExpr::new),
        }),
        ..Default::default()
    };

    let operation = Operation::AlterColumn {
        schema: schema_name,
        table: table_name,
        name: column_name,
        changes,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AlterColumnTypeOutput {
        success: true,
        sql_executed: sql,
        warning: Some(
            "Type changes may require data conversion. Verify with get_breaking_changes."
                .to_string(),
        ),
    })
}

/// Implements the alter_column_default tool.
pub async fn alter_column_default(
    service: &TernMcpService,
    input: AlterColumnDefaultInput,
) -> McpResult<AlterColumnDefaultOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table and column exist
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    if !table
        .columns
        .iter()
        .any(|c| c.name.as_ref() == input.column)
    {
        return Err(McpError::ColumnNotFound {
            table: input.table,
            column: input.column,
        });
    }

    // Build SQL
    let sql = match &input.default {
        Some(default) => format!(
            "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
            quote_identifier(&input.table),
            quote_identifier(&input.column),
            default
        ),
        None => format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
            quote_identifier(&input.table),
            quote_identifier(&input.column)
        ),
    };

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let column_name =
        ColumnName::try_new(input.column.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let has_default = input.default.is_some();
    let changes = ColumnChanges {
        set_default: Some(match input.default {
            Some(d) => DefaultChange::Set(SqlExpr::new(d)),
            None => DefaultChange::Drop,
        }),
        ..Default::default()
    };

    let operation = Operation::AlterColumn {
        schema: schema_name,
        table: table_name,
        name: column_name,
        changes,
    };

    session.apply_operation(&sql, operation).await?;

    let message = if has_default {
        format!("Set default for column '{}.{}'", input.table, input.column)
    } else {
        format!(
            "Dropped default for column '{}.{}'",
            input.table, input.column
        )
    };

    Ok(AlterColumnDefaultOutput {
        success: true,
        sql_executed: sql,
        message,
    })
}

/// Implements the alter_column_nullable tool.
pub async fn alter_column_nullable(
    service: &TernMcpService,
    input: AlterColumnNullableInput,
) -> McpResult<AlterColumnNullableOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table and column exist
    let namespace = session.current_namespace().await?;
    let table = namespace
        .tables
        .iter()
        .find(|t| t.name.as_ref() == input.table)
        .ok_or_else(|| McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        })?;

    if !table
        .columns
        .iter()
        .any(|c| c.name.as_ref() == input.column)
    {
        return Err(McpError::ColumnNotFound {
            table: input.table,
            column: input.column,
        });
    }

    // Build SQL
    let sql = if input.nullable {
        format!(
            "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
            quote_identifier(&input.table),
            quote_identifier(&input.column)
        )
    } else {
        format!(
            "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
            quote_identifier(&input.table),
            quote_identifier(&input.column)
        )
    };

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let table_name =
        TableName::try_new(input.table.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;
    let column_name =
        ColumnName::try_new(input.column.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let changes = ColumnChanges {
        set_not_null: Some(!input.nullable),
        ..Default::default()
    };

    let operation = Operation::AlterColumn {
        schema: schema_name,
        table: table_name,
        name: column_name,
        changes,
    };

    session.apply_operation(&sql, operation).await?;

    let message = if input.nullable {
        format!("Column '{}.{}' is now nullable", input.table, input.column)
    } else {
        format!("Column '{}.{}' is now NOT NULL", input.table, input.column)
    };

    Ok(AlterColumnNullableOutput {
        success: true,
        sql_executed: sql,
        message,
    })
}
