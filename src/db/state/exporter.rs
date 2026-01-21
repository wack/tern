//! Schema export functionality.
//!
//! This module provides the ability to export a `Namespace` (Tern's in-memory
//! schema representation) to SQL DDL. This is a core component of the model-first
//! migration workflow, allowing users to see the current schema as plain SQL.
//!
//! # How It Works
//!
//! Exporting a schema to DDL is equivalent to asking "what SQL would I need to
//! run to create this schema from scratch?" This is exactly what a migration from
//! an empty database to the current state would look like.
//!
//! The exporter leverages existing infrastructure:
//! 1. `diff_namespaces(&Namespace::empty(), &current)` - produces a diff where
//!    everything is "added"
//! 2. `MigrationPlan::from_diff()` - converts the diff to semantic operations
//! 3. `PostgresRenderer` - renders operations as SQL DDL
//!
//! # Output Ordering
//!
//! The exporter produces DDL in dependency order:
//! 1. Enum types and domains (no dependencies)
//! 2. Sequences (no dependencies)
//! 3. Tables (may reference enums, sequences)
//! 4. Foreign key constraints (reference other tables)
//! 5. Indexes (reference tables)
//! 6. Views (may reference tables, other views)
//! 7. Comments (reference any object)
//!
//! This ordering ensures the schema.sql can be executed top-to-bottom without
//! dependency errors.
//!
//! # Example
//!
//! ```ignore
//! use tern::db::state::SchemaExporter;
//! use tern::db::model::Namespace;
//!
//! let namespace = load_namespace_from_somewhere();
//! let sql = SchemaExporter::export(&namespace);
//!
//! println!("{}", sql);
//! ```

use crate::db::diff::diff_namespaces;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig, SqlOptions};
use crate::db::model::Namespace;

/// Configuration options for schema export.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Whether to wrap the output in a transaction (BEGIN/COMMIT).
    ///
    /// Default: `false` - schema files are typically not wrapped in transactions
    /// since they represent a declarative schema definition, not a migration.
    pub include_transaction: bool,

    /// Whether to include comment descriptions before each statement.
    ///
    /// Default: `true` - comments help users understand what each statement does.
    pub include_comments: bool,

    /// Render configuration for SQL generation.
    ///
    /// This controls identifier quoting, IF EXISTS/IF NOT EXISTS clauses, etc.
    pub render_config: RenderConfig,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            include_transaction: false,
            include_comments: true,
            render_config: RenderConfig::default(),
        }
    }
}

impl ExportConfig {
    /// Creates a minimal configuration without comments or transactions.
    ///
    /// Useful for generating compact SQL output.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            include_transaction: false,
            include_comments: false,
            render_config: RenderConfig::default(),
        }
    }
}

/// Exports a `Namespace` to SQL DDL.
///
/// The `SchemaExporter` converts Tern's in-memory schema representation to
/// SQL DDL statements that would recreate the schema from scratch. This is
/// the inverse operation of loading a schema from a database.
///
/// # Implementation Notes
///
/// Rather than implementing custom DDL generation, the exporter reuses the
/// existing diff and migration infrastructure. By diffing an empty namespace
/// against the current namespace, we get a diff where all objects are "added".
/// This diff is then converted to a migration plan and rendered to SQL.
///
/// This approach has several advantages:
/// - No duplication of SQL generation logic
/// - Consistent output with migration generation
/// - Automatically handles all object types and their dependencies
/// - Topological sorting ensures correct execution order
pub struct SchemaExporter;

impl SchemaExporter {
    /// Exports a namespace to SQL DDL using default configuration.
    ///
    /// The output is suitable for creating the schema from scratch in an
    /// empty database. Statements are ordered to respect dependencies.
    ///
    /// # Arguments
    ///
    /// * `namespace` - The namespace to export
    ///
    /// # Returns
    ///
    /// A string containing SQL DDL statements that would recreate the schema.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let sql = SchemaExporter::export(&namespace);
    /// std::fs::write(".tern/schema.sql", sql)?;
    /// ```
    #[must_use]
    pub fn export(namespace: &Namespace) -> String {
        Self::export_with_config(namespace, &ExportConfig::default())
    }

    /// Exports a namespace to SQL DDL with custom configuration.
    ///
    /// This method provides control over the output format, including whether
    /// to include transactions, comments, and identifier quoting style.
    ///
    /// # Arguments
    ///
    /// * `namespace` - The namespace to export
    /// * `config` - Export configuration options
    ///
    /// # Returns
    ///
    /// A string containing SQL DDL statements.
    #[must_use]
    pub fn export_with_config(namespace: &Namespace, config: &ExportConfig) -> String {
        // Create an empty namespace with the same name.
        // Diffing empty -> current produces a diff where everything is "added".
        let empty = Namespace::empty(namespace.name.as_ref());

        // Generate diff: everything in the namespace will appear as "added"
        let diff = diff_namespaces(&empty, namespace);

        // Convert diff to migration plan (handles dependency ordering)
        let plan = MigrationPlan::from_diff(&diff);

        // If the plan is empty, return an empty schema file with a header comment
        if plan.is_empty() {
            return Self::empty_schema_header(namespace);
        }

        // Render to SQL using the PostgreSQL renderer
        let renderer = PostgresRenderer::new(config.render_config.clone());
        let script = plan.render(&renderer);

        // Build the output with optional header
        let mut output = Self::schema_header(namespace);

        // Convert to SQL with the configured options
        let sql_options = SqlOptions {
            include_transaction: config.include_transaction,
            include_comments: config.include_comments,
        };

        output.push_str(&script.to_sql_with_options(sql_options));

        output
    }

    /// Generates a header comment for the schema file.
    fn schema_header(namespace: &Namespace) -> String {
        format!(
            "-- Schema: {}\n-- Generated by Tern\n--\n-- This file represents the complete database schema.\n-- It can be executed against an empty database to recreate the schema.\n\n",
            namespace.name.as_ref()
        )
    }

    /// Generates output for an empty schema.
    fn empty_schema_header(namespace: &Namespace) -> String {
        format!(
            "-- Schema: {}\n-- Generated by Tern\n--\n-- This schema is empty.\n",
            namespace.name.as_ref()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::column::Column;
    use crate::db::model::constraint::{Constraint, ConstraintKind, PrimaryKeyConstraint};
    use crate::db::model::types::{QualifiedCollationName, TypeInfo};
    use crate::db::model::{EnumType, Sequence, Table, TableKind, View};
    use crate::db::schema::{
        CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName,
        TableName, TypeName,
    };

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn integer_type() -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new("int4".to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: "integer".to_string(),
            is_array: false,
        }
    }

    fn varchar_type(len: usize) -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new("varchar".to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: format!("character varying({})", len),
            is_array: false,
        }
    }

    fn default_collation() -> QualifiedCollationName {
        QualifiedCollationName::new(
            SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            CollationName::try_new("default".to_string()).unwrap(),
        )
    }

    fn simple_column(name: &str, position: i16, type_info: TypeInfo, nullable: bool) -> Column {
        Column {
            name: ColumnName::try_new(name.to_string()).unwrap(),
            position,
            type_info,
            is_nullable: nullable,
            default: None,
            generated: None,
            identity: None,
            collation: default_collation(),
            comment: None,
        }
    }

    fn simple_table(name: &str, columns: Vec<Column>, constraints: Vec<Constraint>) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns,
            constraints,
            indexes: vec![],
            comment: None,
        }
    }

    #[test]
    fn export_empty_namespace() {
        let ns = Namespace::empty("public");
        let sql = SchemaExporter::export(&ns);

        assert!(sql.contains("Schema: public"));
        assert!(sql.contains("Generated by Tern"));
        assert!(sql.contains("This schema is empty"));
    }

    #[test]
    fn export_namespace_with_table() {
        let columns = vec![
            simple_column("id", 1, integer_type(), false),
            simple_column("name", 2, varchar_type(100), true),
        ];

        let pk_constraint = Constraint {
            name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
            }),
            comment: None,
        };

        let table = simple_table("users", columns, vec![pk_constraint]);

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![table],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        let sql = SchemaExporter::export(&ns);

        // Check header
        assert!(sql.contains("Schema: public"));
        assert!(sql.contains("Generated by Tern"));

        // Check table creation
        assert!(sql.contains("CREATE TABLE"));
        assert!(sql.contains("public.users"));
        assert!(sql.contains("id integer NOT NULL"));
        assert!(sql.contains("name character varying(100)"));
        assert!(sql.contains("PRIMARY KEY"));
    }

    #[test]
    fn export_namespace_with_enum() {
        let enum_type = EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new("status".to_string()).unwrap(),
            values: vec![
                "pending".to_string(),
                "active".to_string(),
                "completed".to_string(),
            ],
            comment: None,
        };

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![],
            views: vec![],
            sequences: vec![],
            enums: vec![enum_type],
            comment: None,
        };

        let sql = SchemaExporter::export(&ns);

        assert!(sql.contains("CREATE TYPE"));
        assert!(sql.contains("public.status"));
        assert!(sql.contains("AS ENUM"));
        assert!(sql.contains("'pending'"));
        assert!(sql.contains("'active'"));
        assert!(sql.contains("'completed'"));
    }

    #[test]
    fn export_namespace_with_sequence() {
        let sequence = Sequence {
            oid: Oid::new(1),
            name: SequenceName::try_new("users_id_seq".to_string()).unwrap(),
            data_type: TypeInfo {
                name: TypeName::try_new("int8".to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: "bigint".to_string(),
                is_array: false,
            },
            start_value: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cache_size: 1,
            is_cyclic: false,
            comment: None,
        };

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![],
            views: vec![],
            sequences: vec![sequence],
            enums: vec![],
            comment: None,
        };

        let sql = SchemaExporter::export(&ns);

        assert!(sql.contains("CREATE SEQUENCE"));
        assert!(sql.contains("public.users_id_seq"));
        assert!(sql.contains("AS bigint"));
        assert!(sql.contains("START WITH 1"));
        assert!(sql.contains("INCREMENT BY 1"));
    }

    #[test]
    fn export_namespace_with_view() {
        let view = View {
            oid: Oid::new(1),
            name: TableName::try_new("active_users".to_string()).unwrap(),
            definition: crate::db::model::types::SqlExpr::new(
                "SELECT * FROM users WHERE active = true".to_string(),
            ),
            is_materialized: false,
            comment: None,
        };

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![],
            views: vec![view],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        let sql = SchemaExporter::export(&ns);

        assert!(sql.contains("CREATE VIEW"));
        assert!(sql.contains("public.active_users"));
        assert!(sql.contains("SELECT * FROM users WHERE active = true"));
    }

    #[test]
    fn export_minimal_config_no_comments() {
        let columns = vec![simple_column("id", 1, integer_type(), false)];
        let table = simple_table("users", columns, vec![]);

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![table],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        let sql = SchemaExporter::export_with_config(&ns, &ExportConfig::minimal());

        // Should still have header
        assert!(sql.contains("Schema: public"));
        // Should have CREATE TABLE but no operation comments
        assert!(sql.contains("CREATE TABLE"));
        // Minimal config doesn't include the "-- Create table" comments from renderer
        assert!(!sql.contains("-- Create table public.users"));
    }

    #[test]
    fn export_orders_enums_before_tables() {
        // When a table uses an enum type, the enum must be created first.
        // Verify that the export produces them in the correct order.
        let enum_type = EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new("status".to_string()).unwrap(),
            values: vec!["active".to_string(), "inactive".to_string()],
            comment: None,
        };

        let columns = vec![simple_column("id", 1, integer_type(), false)];
        let table = simple_table("users", columns, vec![]);

        let ns = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![table],
            views: vec![],
            sequences: vec![],
            enums: vec![enum_type],
            comment: None,
        };

        let sql = SchemaExporter::export(&ns);

        // Enum should appear before table
        let enum_pos = sql.find("CREATE TYPE").expect("should contain CREATE TYPE");
        let table_pos = sql
            .find("CREATE TABLE")
            .expect("should contain CREATE TABLE");
        assert!(enum_pos < table_pos, "Enum should be created before table");
    }
}
