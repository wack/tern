//! MCP resources for schema inspection.
//!
//! This module provides read-only resources that expose schema information
//! to MCP clients. Resources include:
//!
//! - `tern://schema` — Current schema state after all migrations
//! - `tern://migrations` — List of all migrations with metadata
//! - `tern://migration/{id}` — Details of a specific migration

use serde::Serialize;
use serde_json::Value;

use crate::db::model::Namespace;
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::db::state::{Migration, MigrationId, MigrationIndex, StateHash};
use crate::mcp::error::McpError;
use crate::mcp::protocol::{Resource, ResourceContent};

/// URI scheme for tern resources.
pub const TERN_SCHEME: &str = "tern";

/// Resource URIs.
pub mod uris {
    /// Schema resource URI.
    pub const SCHEMA: &str = "tern://schema";
    /// Migrations list resource URI.
    pub const MIGRATIONS: &str = "tern://migrations";
    /// Migration detail resource URI prefix.
    pub const MIGRATION_PREFIX: &str = "tern://migration/";
}

/// Returns the list of available resources.
pub fn list_resources() -> Vec<Resource> {
    vec![
        Resource {
            uri: uris::SCHEMA.to_string(),
            name: "Current Schema".to_string(),
            description: Some(
                "The current database schema after all migrations have been applied.".to_string(),
            ),
            mime_type: Some("application/json".to_string()),
        },
        Resource {
            uri: uris::MIGRATIONS.to_string(),
            name: "Migration History".to_string(),
            description: Some("List of all migrations in order of application.".to_string()),
            mime_type: Some("application/json".to_string()),
        },
    ]
}

/// Reads a resource by URI.
///
/// # Errors
///
/// Returns an error if the resource is not found or cannot be read.
pub async fn read_resource(
    uri: &str,
    backend: &LocalFileBackend,
    current_state: &Namespace,
) -> Result<ResourceContent, McpError> {
    if uri == uris::SCHEMA {
        read_schema_resource(current_state)
    } else if uri == uris::MIGRATIONS {
        read_migrations_resource(backend).await
    } else if let Some(id_str) = uri.strip_prefix(uris::MIGRATION_PREFIX) {
        read_migration_resource(backend, id_str).await
    } else {
        Err(McpError::ResourceNotFound(uri.to_string()))
    }
}

/// Reads the schema resource.
fn read_schema_resource(namespace: &Namespace) -> Result<ResourceContent, McpError> {
    let schema_output = SchemaResource::from(namespace);
    let json = serde_json::to_string_pretty(&schema_output)
        .map_err(|e| McpError::InternalError(format!("failed to serialize schema: {e}")))?;

    Ok(ResourceContent {
        uri: uris::SCHEMA.to_string(),
        mime_type: Some("application/json".to_string()),
        text: Some(json),
        blob: None,
    })
}

/// Reads the migrations list resource.
async fn read_migrations_resource(backend: &LocalFileBackend) -> Result<ResourceContent, McpError> {
    let index = backend
        .get_migration_index()
        .await
        .map_err(|e| McpError::BackendError(e.to_string()))?;

    let migrations = backend
        .get_all_migrations()
        .await
        .map_err(|e| McpError::BackendError(e.to_string()))?;

    let current_state_hash = backend
        .get_current_state_hash()
        .await
        .map_err(|e| McpError::BackendError(e.to_string()))?;

    let output = MigrationsResource::from_migrations(&index, &migrations, current_state_hash);
    let json = serde_json::to_string_pretty(&output)
        .map_err(|e| McpError::InternalError(format!("failed to serialize migrations: {e}")))?;

    Ok(ResourceContent {
        uri: uris::MIGRATIONS.to_string(),
        mime_type: Some("application/json".to_string()),
        text: Some(json),
        blob: None,
    })
}

/// Reads a specific migration resource.
async fn read_migration_resource(
    backend: &LocalFileBackend,
    id_str: &str,
) -> Result<ResourceContent, McpError> {
    // Parse the migration ID
    let migration_id = MigrationId::from_hex(id_str)
        .or_else(|| MigrationId::from_hex_prefix(id_str))
        .ok_or_else(|| McpError::InvalidResourceUri(format!("invalid migration ID: {id_str}")))?;

    // Get the migration index to find the sequence number
    let index = backend
        .get_migration_index()
        .await
        .map_err(|e| McpError::BackendError(e.to_string()))?;

    let sequence_number = index
        .position(&migration_id)
        .map(|pos| pos + 1)
        .ok_or_else(|| McpError::MigrationNotFound(id_str.to_string()))?;

    // Get the migration
    let migration = backend
        .get_migration(&migration_id)
        .await
        .map_err(|e| McpError::MigrationNotFound(e.to_string()))?;

    let output = MigrationDetailResource::from_migration(&migration, sequence_number);
    let json = serde_json::to_string_pretty(&output)
        .map_err(|e| McpError::InternalError(format!("failed to serialize migration: {e}")))?;

    let uri = format!("{}{}", uris::MIGRATION_PREFIX, id_str);
    Ok(ResourceContent {
        uri,
        mime_type: Some("application/json".to_string()),
        text: Some(json),
        blob: None,
    })
}

// =============================================================================
// Resource Output Types
// =============================================================================

/// Output format for the schema resource.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaResource {
    /// Schema name.
    pub name: String,
    /// Tables in the schema.
    pub tables: Vec<TableOutput>,
    /// Views in the schema.
    pub views: Vec<ViewOutput>,
    /// Sequences in the schema.
    pub sequences: Vec<SequenceOutput>,
    /// Enum types in the schema.
    pub enums: Vec<EnumOutput>,
}

impl From<&Namespace> for SchemaResource {
    fn from(ns: &Namespace) -> Self {
        Self {
            name: ns.name.as_ref().to_string(),
            tables: ns.tables.iter().map(TableOutput::from).collect(),
            views: ns.views.iter().map(ViewOutput::from).collect(),
            sequences: ns.sequences.iter().map(SequenceOutput::from).collect(),
            enums: ns.enums.iter().map(EnumOutput::from).collect(),
        }
    }
}

/// Output format for a table.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOutput {
    /// Table name.
    pub name: String,
    /// Columns in the table.
    pub columns: Vec<ColumnOutput>,
    /// Primary key constraint, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_key: Option<PrimaryKeyOutput>,
    /// Foreign key constraints.
    pub foreign_keys: Vec<ForeignKeyOutput>,
    /// Unique constraints.
    pub unique_constraints: Vec<UniqueConstraintOutput>,
    /// Check constraints.
    pub check_constraints: Vec<CheckConstraintOutput>,
    /// Indexes on the table.
    pub indexes: Vec<IndexOutput>,
}

impl From<&crate::db::model::Table> for TableOutput {
    fn from(table: &crate::db::model::Table) -> Self {
        use crate::db::model::constraint::ConstraintKind;

        let primary_key = table.constraints.iter().find_map(|c| {
            if let ConstraintKind::PrimaryKey(pk) = &c.kind {
                Some(PrimaryKeyOutput {
                    name: c.name.as_ref().to_string(),
                    columns: pk
                        .columns
                        .iter()
                        .map(|col| col.as_ref().to_string())
                        .collect(),
                })
            } else {
                None
            }
        });

        let foreign_keys = table
            .constraints
            .iter()
            .filter_map(|c| {
                if let ConstraintKind::ForeignKey(fk) = &c.kind {
                    Some(ForeignKeyOutput {
                        name: c.name.as_ref().to_string(),
                        columns: fk
                            .columns
                            .iter()
                            .map(|col| col.as_ref().to_string())
                            .collect(),
                        references_table: fk.referenced_table.name.as_ref().to_string(),
                        references_columns: fk
                            .referenced_columns
                            .iter()
                            .map(|col| col.as_ref().to_string())
                            .collect(),
                        on_delete: fk.on_delete.as_sql().to_string(),
                        on_update: fk.on_update.as_sql().to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        let unique_constraints = table
            .constraints
            .iter()
            .filter_map(|c| {
                if let ConstraintKind::Unique(uq) = &c.kind {
                    Some(UniqueConstraintOutput {
                        name: c.name.as_ref().to_string(),
                        columns: uq
                            .columns
                            .iter()
                            .map(|col| col.as_ref().to_string())
                            .collect(),
                    })
                } else {
                    None
                }
            })
            .collect();

        let check_constraints = table
            .constraints
            .iter()
            .filter_map(|c| {
                if let ConstraintKind::Check(chk) = &c.kind {
                    Some(CheckConstraintOutput {
                        name: c.name.as_ref().to_string(),
                        expression: chk.expression.as_ref().to_string(),
                    })
                } else {
                    None
                }
            })
            .collect();

        Self {
            name: table.name.as_ref().to_string(),
            columns: table.columns.iter().map(ColumnOutput::from).collect(),
            primary_key,
            foreign_keys,
            unique_constraints,
            check_constraints,
            indexes: table.indexes.iter().map(IndexOutput::from).collect(),
        }
    }
}

/// Output format for a column.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnOutput {
    /// Column name.
    pub name: String,
    /// PostgreSQL data type.
    #[serde(rename = "type")]
    pub data_type: String,
    /// Whether the column allows NULL values.
    pub nullable: bool,
    /// Default value expression, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Identity column information, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<IdentityOutput>,
}

impl From<&crate::db::model::Column> for ColumnOutput {
    fn from(col: &crate::db::model::Column) -> Self {
        Self {
            name: col.name.as_ref().to_string(),
            data_type: col.type_info.formatted.clone(),
            nullable: col.is_nullable,
            default: col.default.as_ref().map(|d| d.as_ref().to_string()),
            identity: col.identity.map(|id| IdentityOutput {
                kind: format!("{:?}", id),
            }),
        }
    }
}

/// Output format for identity column info.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityOutput {
    /// Identity kind (ALWAYS or BY DEFAULT).
    #[serde(rename = "type")]
    pub kind: String,
}

/// Output format for a primary key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryKeyOutput {
    /// Constraint name.
    pub name: String,
    /// Column names.
    pub columns: Vec<String>,
}

/// Output format for a foreign key.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyOutput {
    /// Constraint name.
    pub name: String,
    /// Columns in the referencing table.
    pub columns: Vec<String>,
    /// Referenced table name.
    pub references_table: String,
    /// Referenced column names.
    pub references_columns: Vec<String>,
    /// ON DELETE action.
    pub on_delete: String,
    /// ON UPDATE action.
    pub on_update: String,
}

/// Output format for a unique constraint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniqueConstraintOutput {
    /// Constraint name.
    pub name: String,
    /// Column names.
    pub columns: Vec<String>,
}

/// Output format for a check constraint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckConstraintOutput {
    /// Constraint name.
    pub name: String,
    /// Check expression.
    pub expression: String,
}

/// Output format for an index.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexOutput {
    /// Index name.
    pub name: String,
    /// Column names (with sort order if specified).
    pub columns: Vec<String>,
    /// Whether the index is unique.
    pub unique: bool,
    /// Index method (btree, hash, gin, etc.).
    pub method: String,
    /// Partial index predicate, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
}

impl From<&crate::db::model::Index> for IndexOutput {
    fn from(idx: &crate::db::model::Index) -> Self {
        use crate::db::model::index::SortOrder;

        let columns = idx
            .columns
            .iter()
            .map(|col| {
                // Use column name if available, otherwise use expression
                let base_name = col
                    .column
                    .as_ref()
                    .map(|c| c.as_ref().to_string())
                    .or_else(|| col.expression.as_ref().map(|e| e.as_ref().to_string()))
                    .unwrap_or_default();

                // Append sort order if not ascending (the default)
                match col.order {
                    SortOrder::Descending => format!("{} DESC", base_name),
                    SortOrder::Ascending => base_name,
                }
            })
            .collect();

        Self {
            name: idx.name.as_ref().to_string(),
            columns,
            unique: idx.is_unique,
            method: idx.method.as_str().to_string(),
            predicate: idx.predicate.as_ref().map(|p| p.as_ref().to_string()),
        }
    }
}

/// Output format for a view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewOutput {
    /// View name.
    pub name: String,
    /// View definition (SELECT statement).
    pub definition: String,
    /// Whether this is a materialized view.
    pub is_materialized: bool,
}

impl From<&crate::db::model::View> for ViewOutput {
    fn from(view: &crate::db::model::View) -> Self {
        Self {
            name: view.name.as_ref().to_string(),
            definition: view.definition.as_ref().to_string(),
            is_materialized: view.is_materialized,
        }
    }
}

/// Output format for a sequence.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceOutput {
    /// Sequence name.
    pub name: String,
    /// Data type.
    pub data_type: String,
    /// Start value.
    pub start: i64,
    /// Increment.
    pub increment: i64,
    /// Minimum value.
    pub min_value: i64,
    /// Maximum value.
    pub max_value: i64,
    /// Cache size.
    pub cache: i64,
    /// Whether the sequence cycles.
    pub cycle: bool,
}

impl From<&crate::db::model::Sequence> for SequenceOutput {
    fn from(seq: &crate::db::model::Sequence) -> Self {
        Self {
            name: seq.name.as_ref().to_string(),
            data_type: seq.data_type.formatted.clone(),
            start: seq.start_value,
            increment: seq.increment,
            min_value: seq.min_value,
            max_value: seq.max_value,
            cache: seq.cache_size,
            cycle: seq.is_cyclic,
        }
    }
}

/// Output format for an enum type.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumOutput {
    /// Enum type name.
    pub name: String,
    /// Enum values in order.
    pub values: Vec<String>,
}

impl From<&crate::db::model::EnumType> for EnumOutput {
    fn from(enum_type: &crate::db::model::EnumType) -> Self {
        Self {
            name: enum_type.name.as_ref().to_string(),
            values: enum_type.values.clone(),
        }
    }
}

// =============================================================================
// Migrations Resource Output
// =============================================================================

/// Output format for the migrations list resource.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationsResource {
    /// List of migrations.
    pub migrations: Vec<MigrationSummary>,
    /// Total number of migrations.
    pub total_count: usize,
    /// Current state hash.
    pub current_state_hash: String,
}

impl MigrationsResource {
    /// Creates a migrations resource from the index and migrations.
    pub fn from_migrations(
        index: &MigrationIndex,
        migrations: &[Migration],
        current_state_hash: StateHash,
    ) -> Self {
        let summaries = migrations
            .iter()
            .enumerate()
            .map(|(i, m)| MigrationSummary {
                id: m.id.to_hex(),
                sequence_number: i + 1,
                description: m.description.clone(),
                created_at: m.created_at.to_string(),
                operation_count: m.up_operations.len(),
                has_breaking_changes: m.has_breaking_changes(),
                parent_state_hash: m.parent_state_hash.to_hex(),
                resulting_state_hash: m.resulting_state_hash.to_hex(),
            })
            .collect();

        Self {
            migrations: summaries,
            total_count: index.len(),
            current_state_hash: current_state_hash.to_hex(),
        }
    }
}

/// Summary of a single migration for listing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSummary {
    /// Migration ID.
    pub id: String,
    /// Sequence number (1-indexed).
    pub sequence_number: usize,
    /// Description.
    pub description: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Number of up operations.
    pub operation_count: usize,
    /// Whether this migration has breaking changes.
    pub has_breaking_changes: bool,
    /// Parent state hash.
    pub parent_state_hash: String,
    /// Resulting state hash.
    pub resulting_state_hash: String,
}

// =============================================================================
// Migration Detail Resource Output
// =============================================================================

/// Output format for a specific migration resource.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDetailResource {
    /// Migration ID.
    pub id: String,
    /// Sequence number (1-indexed).
    pub sequence_number: usize,
    /// Description.
    pub description: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Parent state hash.
    pub parent_state_hash: String,
    /// Resulting state hash.
    pub resulting_state_hash: String,
    /// Up operations.
    pub up_operations: Vec<OperationOutput>,
    /// Down operations.
    pub down_operations: Vec<OperationOutput>,
    /// Breaking changes.
    pub breaking_changes: Vec<String>,
    /// Whether this migration is reversible.
    pub is_reversible: bool,
}

impl MigrationDetailResource {
    /// Creates a migration detail from a migration.
    pub fn from_migration(migration: &Migration, sequence_number: usize) -> Self {
        Self {
            id: migration.id.to_hex(),
            sequence_number,
            description: migration.description.clone(),
            created_at: migration.created_at.to_string(),
            parent_state_hash: migration.parent_state_hash.to_hex(),
            resulting_state_hash: migration.resulting_state_hash.to_hex(),
            up_operations: migration
                .up_operations
                .iter()
                .map(OperationOutput::from)
                .collect(),
            down_operations: migration
                .down_operations
                .iter()
                .map(OperationOutput::from)
                .collect(),
            breaking_changes: migration
                .breaking_changes
                .iter()
                .map(|bc| bc.description.clone())
                .collect(),
            is_reversible: migration.is_reversible(),
        }
    }
}

/// Output format for an operation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationOutput {
    /// Operation type.
    #[serde(rename = "type")]
    pub op_type: String,
    /// Operation description.
    pub description: String,
    /// Full operation data (JSON).
    pub data: Value,
}

impl From<&crate::db::migrate::Operation> for OperationOutput {
    fn from(op: &crate::db::migrate::Operation) -> Self {
        let op_type = match op {
            crate::db::migrate::Operation::CreateTable { .. } => "CreateTable",
            crate::db::migrate::Operation::DropTable { .. } => "DropTable",
            crate::db::migrate::Operation::RenameTable { .. } => "RenameTable",
            crate::db::migrate::Operation::AddColumn { .. } => "AddColumn",
            crate::db::migrate::Operation::DropColumn { .. } => "DropColumn",
            crate::db::migrate::Operation::RenameColumn { .. } => "RenameColumn",
            crate::db::migrate::Operation::AlterColumn { .. } => "AlterColumn",
            crate::db::migrate::Operation::AddConstraint { .. } => "AddConstraint",
            crate::db::migrate::Operation::DropConstraint { .. } => "DropConstraint",
            crate::db::migrate::Operation::RenameConstraint { .. } => "RenameConstraint",
            crate::db::migrate::Operation::CreateIndex { .. } => "CreateIndex",
            crate::db::migrate::Operation::DropIndex { .. } => "DropIndex",
            crate::db::migrate::Operation::RenameIndex { .. } => "RenameIndex",
            crate::db::migrate::Operation::CreateEnum { .. } => "CreateEnum",
            crate::db::migrate::Operation::DropEnum { .. } => "DropEnum",
            crate::db::migrate::Operation::RenameEnum { .. } => "RenameEnum",
            crate::db::migrate::Operation::AddEnumValue { .. } => "AddEnumValue",
            crate::db::migrate::Operation::CreateSequence { .. } => "CreateSequence",
            crate::db::migrate::Operation::DropSequence { .. } => "DropSequence",
            crate::db::migrate::Operation::RenameSequence { .. } => "RenameSequence",
            crate::db::migrate::Operation::AlterSequence { .. } => "AlterSequence",
            crate::db::migrate::Operation::CreateView { .. } => "CreateView",
            crate::db::migrate::Operation::DropView { .. } => "DropView",
            crate::db::migrate::Operation::RenameView { .. } => "RenameView",
            crate::db::migrate::Operation::ReplaceView { .. } => "ReplaceView",
            crate::db::migrate::Operation::RefreshMaterializedView { .. } => {
                "RefreshMaterializedView"
            }
            crate::db::migrate::Operation::SetComment { .. } => "SetComment",
        };

        Self {
            op_type: op_type.to_string(),
            description: op.description(),
            data: serde_json::to_value(op).unwrap_or(Value::Null),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_resources_returns_expected() {
        let resources = list_resources();

        assert_eq!(resources.len(), 2);
        assert!(resources.iter().any(|r| r.uri == uris::SCHEMA));
        assert!(resources.iter().any(|r| r.uri == uris::MIGRATIONS));
    }

    #[test]
    fn schema_resource_from_namespace() {
        let namespace = Namespace::empty("public");
        let output = SchemaResource::from(&namespace);

        assert_eq!(output.name, "public");
        assert!(output.tables.is_empty());
        assert!(output.views.is_empty());
        assert!(output.sequences.is_empty());
        assert!(output.enums.is_empty());
    }
}
