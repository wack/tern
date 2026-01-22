//! Index operation tool implementations.

use crate::db::migrate::Operation;
use crate::db::model::index::{Index, IndexColumn, NullsOrder, SortOrder};
use crate::db::model::types::IndexMethod;
use crate::db::schema::{ColumnName, IndexName, Oid, SchemaName, TableName};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    CreateIndexInput, CreateIndexOutput, DropIndexInput, DropIndexOutput, IndexColumnSpec,
    TernMcpService,
};

/// Quotes a PostgreSQL identifier if needed.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Parses an index method string.
fn parse_index_method(method: &str) -> Result<IndexMethod, McpError> {
    match method.to_lowercase().as_str() {
        "btree" | "" => Ok(IndexMethod::BTree),
        "hash" => Ok(IndexMethod::Hash),
        "gin" => Ok(IndexMethod::Gin),
        "gist" => Ok(IndexMethod::Gist),
        "brin" => Ok(IndexMethod::Brin),
        _ => Err(McpError::InvalidInput {
            message: format!(
                "Invalid index method: {}. Valid options: btree, hash, gin, gist, brin",
                method
            ),
        }),
    }
}

/// Implements the create_index tool.
pub async fn create_index(
    service: &TernMcpService,
    input: CreateIndexInput,
) -> McpResult<CreateIndexOutput> {
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

    // Parse index method
    let method = input
        .method
        .as_ref()
        .map(|m| parse_index_method(m))
        .transpose()?
        .unwrap_or(IndexMethod::BTree);

    // Generate index name
    let index_name = input.name.clone().unwrap_or_else(|| {
        let col_names: Vec<String> = input
            .columns
            .iter()
            .map(|c| match c {
                IndexColumnSpec::Simple(name) => name.clone(),
                IndexColumnSpec::WithOptions { name, .. } => name.clone(),
            })
            .collect();
        format!("idx_{}_{}", input.table, col_names.join("_"))
    });

    // Check if index already exists
    if table.indexes.iter().any(|i| i.name.as_ref() == index_name) {
        return Err(McpError::DuplicateObject {
            kind: "Index".to_string(),
            name: index_name,
        });
    }

    // Build column specifications for SQL
    let mut col_specs = Vec::new();
    let mut index_columns = Vec::new();

    for col_spec in input.columns.iter() {
        let (name, order, nulls) = match col_spec {
            IndexColumnSpec::Simple(name) => (name.clone(), None, None),
            IndexColumnSpec::WithOptions { name, order, nulls } => {
                (name.clone(), order.clone(), nulls.clone())
            }
        };

        // Verify column exists
        if !table.columns.iter().any(|c| c.name.as_ref() == name) {
            return Err(McpError::ColumnNotFound {
                table: input.table.clone(),
                column: name,
            });
        }

        // Build SQL column spec
        let mut col_sql = quote_identifier(&name);
        if let Some(ref ord) = order {
            col_sql.push(' ');
            col_sql.push_str(ord);
        }
        if let Some(ref n) = nulls {
            col_sql.push_str(" NULLS ");
            col_sql.push_str(n);
        }
        col_specs.push(col_sql);

        // Build index column for operation
        let sort_order = order
            .as_ref()
            .map(|o| match o.to_uppercase().as_str() {
                "DESC" => SortOrder::Descending,
                _ => SortOrder::Ascending,
            })
            .unwrap_or(SortOrder::Ascending);

        let nulls_order = nulls
            .as_ref()
            .map(|n| match n.to_uppercase().as_str() {
                "FIRST" => NullsOrder::First,
                _ => NullsOrder::Last,
            })
            .unwrap_or(NullsOrder::Last);

        index_columns.push(IndexColumn {
            column: Some(
                ColumnName::try_new(name).map_err(|e| McpError::InvalidInput {
                    message: e.to_string(),
                })?,
            ),
            expression: None,
            order: sort_order,
            nulls: nulls_order,
        });
    }

    // Build SQL
    let unique_str = if input.unique { "UNIQUE " } else { "" };
    let method_str = if method != IndexMethod::BTree {
        format!(" USING {}", method.as_str())
    } else {
        String::new()
    };

    let mut sql = format!(
        "CREATE {}INDEX {}{} ON {} ({})",
        unique_str,
        quote_identifier(&index_name),
        method_str,
        quote_identifier(&input.table),
        col_specs.join(", ")
    );

    if let Some(ref where_clause) = input.r#where {
        sql.push_str(&format!(" WHERE {}", where_clause));
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

    let index = Index {
        oid: Oid::default(),
        name: IndexName::try_new(index_name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?,
        method,
        is_unique: input.unique,
        is_constraint_index: false,
        columns: index_columns,
        predicate: input.r#where.map(crate::db::model::types::SqlExpr::new),
        comment: None,
    };

    let operation = Operation::CreateIndex {
        schema: schema_name,
        table: table_name,
        index,
        concurrently: false,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(CreateIndexOutput {
        success: true,
        index_name,
        sql_executed: sql,
    })
}

/// Implements the drop_index tool.
pub async fn drop_index(
    service: &TernMcpService,
    input: DropIndexInput,
) -> McpResult<DropIndexOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if index exists
    let namespace = session.current_namespace().await?;
    let index_exists = namespace
        .tables
        .iter()
        .any(|t| t.indexes.iter().any(|i| i.name.as_ref() == input.name));

    if !index_exists {
        return Err(McpError::IndexNotFound { name: input.name });
    }

    // Build SQL
    let sql = format!("DROP INDEX {}", quote_identifier(&input.name));

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;
    let index_name =
        IndexName::try_new(input.name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?;

    let operation = Operation::DropIndex {
        schema: schema_name,
        name: index_name,
        concurrently: false,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(DropIndexOutput {
        success: true,
        sql_executed: sql,
        message: format!("Dropped index '{}'", input.name),
    })
}
