//! Constraint operation tool implementations.

use crate::db::migrate::Operation;
use crate::db::model::constraint::{
    CheckConstraint, Constraint, ConstraintKind, ForeignKeyConstraint, PrimaryKeyConstraint,
    UniqueConstraint,
};
use crate::db::model::types::{ForeignKeyAction, QualifiedName, SqlExpr};
use crate::db::schema::{ColumnName, ConstraintName, IndexName, SchemaName, TableName};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    AddCheckConstraintInput, AddCheckConstraintOutput, AddForeignKeyInput, AddForeignKeyOutput,
    AddPrimaryKeyInput, AddPrimaryKeyOutput, AddUniqueConstraintInput, AddUniqueConstraintOutput,
    DropConstraintInput, DropConstraintOutput, TernMcpService,
};

/// Quotes a PostgreSQL identifier if needed.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Parses a foreign key action string.
fn parse_fk_action(action: &str) -> Result<ForeignKeyAction, McpError> {
    match action.to_uppercase().as_str() {
        "NO ACTION" | "" => Ok(ForeignKeyAction::NoAction),
        "RESTRICT" => Ok(ForeignKeyAction::Restrict),
        "CASCADE" => Ok(ForeignKeyAction::Cascade),
        "SET NULL" => Ok(ForeignKeyAction::SetNull),
        "SET DEFAULT" => Ok(ForeignKeyAction::SetDefault),
        _ => Err(McpError::InvalidInput {
            message: format!(
                "Invalid foreign key action: {}. Valid options: NO ACTION, RESTRICT, CASCADE, SET NULL, SET DEFAULT",
                action
            ),
        }),
    }
}

/// Implements the add_primary_key tool.
pub async fn add_primary_key(
    service: &TernMcpService,
    input: AddPrimaryKeyInput,
) -> McpResult<AddPrimaryKeyOutput> {
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

    // Check if primary key already exists
    if table
        .constraints
        .iter()
        .any(|c| matches!(c.kind, ConstraintKind::PrimaryKey(_)))
    {
        return Err(McpError::DuplicateObject {
            kind: "Primary key".to_string(),
            name: input.table.clone(),
        });
    }

    // Verify all columns exist
    for col in &input.columns {
        if !table.columns.iter().any(|c| c.name.as_ref() == col) {
            return Err(McpError::ColumnNotFound {
                table: input.table.clone(),
                column: col.clone(),
            });
        }
    }

    // Generate constraint name
    let constraint_name = input
        .constraint_name
        .unwrap_or_else(|| format!("{}_pkey", input.table));

    // Build SQL
    let cols: Vec<String> = input.columns.iter().map(|c| quote_identifier(c)).collect();
    let sql = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} PRIMARY KEY ({})",
        quote_identifier(&input.table),
        quote_identifier(&constraint_name),
        cols.join(", ")
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

    let columns: Vec<ColumnName> = input
        .columns
        .iter()
        .map(|c| {
            ColumnName::try_new(c.clone()).map_err(|e| McpError::InvalidInput {
                message: e.to_string(),
            })
        })
        .collect::<McpResult<Vec<_>>>()?;

    let constraint = Constraint {
        name: ConstraintName::try_new(constraint_name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns,
            index_name: IndexName::try_new(constraint_name.clone()).map_err(|e| {
                McpError::InvalidInput {
                    message: e.to_string(),
                }
            })?,
        }),
        comment: None,
    };

    let operation = Operation::AddConstraint {
        schema: schema_name,
        table: table_name,
        constraint,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddPrimaryKeyOutput {
        success: true,
        constraint_name,
        sql_executed: sql,
        message: format!("Added primary key to table '{}'", input.table),
    })
}

/// Implements the add_foreign_key tool.
pub async fn add_foreign_key(
    service: &TernMcpService,
    input: AddForeignKeyInput,
) -> McpResult<AddForeignKeyOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if source table exists
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

    // Check if referenced table exists
    if !namespace
        .tables
        .iter()
        .any(|t| t.name.as_ref() == input.references_table)
    {
        return Err(McpError::TableNotFound {
            name: input.references_table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        });
    }

    // Verify source columns exist
    for col in &input.columns {
        if !table.columns.iter().any(|c| c.name.as_ref() == col) {
            return Err(McpError::ColumnNotFound {
                table: input.table.clone(),
                column: col.clone(),
            });
        }
    }

    // Generate constraint name
    let constraint_name = input
        .constraint_name
        .unwrap_or_else(|| format!("{}_{}_fkey", input.table, input.columns.join("_")));

    // Parse actions
    let on_delete = input
        .on_delete
        .as_ref()
        .map(|s| parse_fk_action(s))
        .transpose()?
        .unwrap_or(ForeignKeyAction::NoAction);
    let on_update = input
        .on_update
        .as_ref()
        .map(|s| parse_fk_action(s))
        .transpose()?
        .unwrap_or(ForeignKeyAction::NoAction);

    // Build SQL
    let source_cols: Vec<String> = input.columns.iter().map(|c| quote_identifier(c)).collect();
    let ref_cols: Vec<String> = input
        .references_columns
        .iter()
        .map(|c| quote_identifier(c))
        .collect();

    let mut sql = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}({})",
        quote_identifier(&input.table),
        quote_identifier(&constraint_name),
        source_cols.join(", "),
        quote_identifier(&input.references_table),
        ref_cols.join(", ")
    );

    if on_delete != ForeignKeyAction::NoAction {
        sql.push_str(&format!(" ON DELETE {}", on_delete.as_sql()));
    }
    if on_update != ForeignKeyAction::NoAction {
        sql.push_str(&format!(" ON UPDATE {}", on_update.as_sql()));
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

    let columns: Vec<ColumnName> = input
        .columns
        .iter()
        .map(|c| {
            ColumnName::try_new(c.clone()).map_err(|e| McpError::InvalidInput {
                message: e.to_string(),
            })
        })
        .collect::<McpResult<Vec<_>>>()?;

    let ref_columns: Vec<ColumnName> = input
        .references_columns
        .iter()
        .map(|c| {
            ColumnName::try_new(c.clone()).map_err(|e| McpError::InvalidInput {
                message: e.to_string(),
            })
        })
        .collect::<McpResult<Vec<_>>>()?;

    let constraint = Constraint {
        name: ConstraintName::try_new(constraint_name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
            columns,
            referenced_table: QualifiedName::new(
                schema_name.clone(),
                TableName::try_new(input.references_table.clone()).map_err(|e| {
                    McpError::InvalidInput {
                        message: e.to_string(),
                    }
                })?,
            ),
            referenced_columns: ref_columns,
            on_delete,
            on_update,
            is_deferrable: false,
            is_initially_deferred: false,
        }),
        comment: None,
    };

    let operation = Operation::AddConstraint {
        schema: schema_name,
        table: table_name,
        constraint,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddForeignKeyOutput {
        success: true,
        constraint_name,
        sql_executed: sql,
        message: format!(
            "Added foreign key from {}.{} to {}.{}",
            input.table,
            input.columns.join(", "),
            input.references_table,
            input.references_columns.join(", ")
        ),
    })
}

/// Implements the add_unique_constraint tool.
pub async fn add_unique_constraint(
    service: &TernMcpService,
    input: AddUniqueConstraintInput,
) -> McpResult<AddUniqueConstraintOutput> {
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

    // Verify all columns exist
    for col in &input.columns {
        if !table.columns.iter().any(|c| c.name.as_ref() == col) {
            return Err(McpError::ColumnNotFound {
                table: input.table.clone(),
                column: col.clone(),
            });
        }
    }

    // Generate constraint name
    let constraint_name = input
        .constraint_name
        .unwrap_or_else(|| format!("{}_{}_key", input.table, input.columns.join("_")));

    // Build SQL
    let cols: Vec<String> = input.columns.iter().map(|c| quote_identifier(c)).collect();
    let sql = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({})",
        quote_identifier(&input.table),
        quote_identifier(&constraint_name),
        cols.join(", ")
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

    let columns: Vec<ColumnName> = input
        .columns
        .iter()
        .map(|c| {
            ColumnName::try_new(c.clone()).map_err(|e| McpError::InvalidInput {
                message: e.to_string(),
            })
        })
        .collect::<McpResult<Vec<_>>>()?;

    let constraint = Constraint {
        name: ConstraintName::try_new(constraint_name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        kind: ConstraintKind::Unique(UniqueConstraint {
            columns,
            index_name: IndexName::try_new(constraint_name.clone()).map_err(|e| {
                McpError::InvalidInput {
                    message: e.to_string(),
                }
            })?,
            nulls_not_distinct: false,
        }),
        comment: None,
    };

    let operation = Operation::AddConstraint {
        schema: schema_name,
        table: table_name,
        constraint,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddUniqueConstraintOutput {
        success: true,
        constraint_name,
        sql_executed: sql,
        message: format!(
            "Added unique constraint on '{}.{}'",
            input.table,
            input.columns.join(", ")
        ),
    })
}

/// Implements the add_check_constraint tool.
pub async fn add_check_constraint(
    service: &TernMcpService,
    input: AddCheckConstraintInput,
) -> McpResult<AddCheckConstraintOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if table exists
    let namespace = session.current_namespace().await?;
    if !namespace
        .tables
        .iter()
        .any(|t| t.name.as_ref() == input.table)
    {
        return Err(McpError::TableNotFound {
            name: input.table.clone(),
            available: namespace
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
        });
    }

    // Generate constraint name
    let constraint_name = input
        .constraint_name
        .unwrap_or_else(|| format!("{}_check", input.table));

    // Build SQL
    let sql = format!(
        "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({})",
        quote_identifier(&input.table),
        quote_identifier(&constraint_name),
        input.expression
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

    let constraint = Constraint {
        name: ConstraintName::try_new(constraint_name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        kind: ConstraintKind::Check(CheckConstraint {
            expression: SqlExpr::new(input.expression.clone()),
            is_no_inherit: false,
        }),
        comment: None,
    };

    let operation = Operation::AddConstraint {
        schema: schema_name,
        table: table_name,
        constraint,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddCheckConstraintOutput {
        success: true,
        constraint_name,
        sql_executed: sql,
        message: format!("Added check constraint on table '{}'", input.table),
    })
}

/// Implements the drop_constraint tool.
pub async fn drop_constraint(
    service: &TernMcpService,
    input: DropConstraintInput,
) -> McpResult<DropConstraintOutput> {
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

    // Check if constraint exists
    if !table
        .constraints
        .iter()
        .any(|c| c.name.as_ref() == input.constraint_name)
    {
        return Err(McpError::ConstraintNotFound {
            table: input.table,
            constraint: input.constraint_name,
        });
    }

    // Build SQL
    let sql = format!(
        "ALTER TABLE {} DROP CONSTRAINT {}",
        quote_identifier(&input.table),
        quote_identifier(&input.constraint_name)
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
    let constraint_name = ConstraintName::try_new(input.constraint_name.clone()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;

    let operation = Operation::DropConstraint {
        schema: schema_name,
        table: table_name,
        name: constraint_name,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(DropConstraintOutput {
        success: true,
        sql_executed: sql,
        message: format!(
            "Dropped constraint '{}' from table '{}'",
            input.constraint_name, input.table
        ),
    })
}
