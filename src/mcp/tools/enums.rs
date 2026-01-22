//! Enum operation tool implementations.

use crate::db::migrate::{EnumValuePosition, Operation};
use crate::db::model::EnumType;
use crate::db::schema::{Oid, SchemaName, TypeName};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    AddEnumValueInput, AddEnumValueOutput, CreateEnumInput, CreateEnumOutput, DropEnumInput,
    DropEnumOutput, TernMcpService,
};

/// Quotes a PostgreSQL identifier if needed.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Implements the create_enum tool.
pub async fn create_enum(
    service: &TernMcpService,
    input: CreateEnumInput,
) -> McpResult<CreateEnumOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if enum already exists
    let namespace = session.current_namespace().await?;
    if namespace
        .enums
        .iter()
        .any(|e| e.name.as_ref() == input.name)
    {
        return Err(McpError::DuplicateObject {
            kind: "Enum".to_string(),
            name: input.name,
        });
    }

    // Validate values
    if input.values.is_empty() {
        return Err(McpError::InvalidInput {
            message: "Enum must have at least one value".to_string(),
        });
    }

    // Build SQL
    let values: Vec<String> = input
        .values
        .iter()
        .map(|v| format!("'{}'", v.replace('\'', "''")))
        .collect();
    let sql = format!(
        "CREATE TYPE {} AS ENUM ({})",
        quote_identifier(&input.name),
        values.join(", ")
    );

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;

    let enum_type = EnumType {
        oid: Oid::default(),
        name: TypeName::try_new(input.name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?,
        values: input.values.clone(),
        comment: None,
    };

    let operation = Operation::CreateEnum {
        schema: schema_name,
        enum_type,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(CreateEnumOutput {
        success: true,
        sql_executed: sql,
    })
}

/// Implements the add_enum_value tool.
pub async fn add_enum_value(
    service: &TernMcpService,
    input: AddEnumValueInput,
) -> McpResult<AddEnumValueOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if enum exists
    let namespace = session.current_namespace().await?;
    let enum_type = namespace
        .enums
        .iter()
        .find(|e| e.name.as_ref() == input.enum_name)
        .ok_or_else(|| McpError::EnumNotFound {
            name: input.enum_name.clone(),
        })?;

    // Check if value already exists
    if enum_type.values.contains(&input.value) {
        return Err(McpError::DuplicateObject {
            kind: "Enum value".to_string(),
            name: format!("{}.{}", input.enum_name, input.value),
        });
    }

    // Determine position
    let position = if let Some(ref before) = input.before {
        if !enum_type.values.contains(before) {
            return Err(McpError::InvalidInput {
                message: format!("Value '{}' not found in enum '{}'", before, input.enum_name),
            });
        }
        EnumValuePosition::Before(before.clone())
    } else if let Some(ref after) = input.after {
        if !enum_type.values.contains(after) {
            return Err(McpError::InvalidInput {
                message: format!("Value '{}' not found in enum '{}'", after, input.enum_name),
            });
        }
        EnumValuePosition::After(after.clone())
    } else {
        EnumValuePosition::End
    };

    // Build SQL
    let value_str = format!("'{}'", input.value.replace('\'', "''"));
    let mut sql = format!(
        "ALTER TYPE {} ADD VALUE {}",
        quote_identifier(&input.enum_name),
        value_str
    );

    match &position {
        EnumValuePosition::Before(v) => {
            sql.push_str(&format!(" BEFORE '{}'", v.replace('\'', "''")));
        }
        EnumValuePosition::After(v) => {
            sql.push_str(&format!(" AFTER '{}'", v.replace('\'', "''")));
        }
        EnumValuePosition::End => {}
    }

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;

    let operation = Operation::AddEnumValue {
        schema: schema_name,
        enum_name: TypeName::try_new(input.enum_name.clone()).map_err(|e| {
            McpError::InvalidInput {
                message: e.to_string(),
            }
        })?,
        value: input.value,
        position,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(AddEnumValueOutput {
        success: true,
        sql_executed: sql,
    })
}

/// Implements the drop_enum tool.
pub async fn drop_enum(
    service: &TernMcpService,
    input: DropEnumInput,
) -> McpResult<DropEnumOutput> {
    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    // Check if enum exists
    let namespace = session.current_namespace().await?;
    if !namespace
        .enums
        .iter()
        .any(|e| e.name.as_ref() == input.name)
    {
        return Err(McpError::EnumNotFound { name: input.name });
    }

    // Build SQL
    let sql = format!("DROP TYPE {}", quote_identifier(&input.name));

    // Execute SQL
    session.execute_sql(&sql).await?;

    // Build the operation for tracking
    let schema_name = SchemaName::try_new(session.schema_name().to_string()).map_err(|e| {
        McpError::InvalidInput {
            message: e.to_string(),
        }
    })?;

    let operation = Operation::DropEnum {
        schema: schema_name,
        name: TypeName::try_new(input.name.clone()).map_err(|e| McpError::InvalidInput {
            message: e.to_string(),
        })?,
    };

    session.apply_operation(&sql, operation).await?;

    Ok(DropEnumOutput {
        success: true,
        sql_executed: sql,
    })
}
