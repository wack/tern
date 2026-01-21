//! Migration compilation to WebAssembly components.
//!
//! This module provides the infrastructure for compiling database migrations
//! into standalone WebAssembly components. These components can be embedded
//! in platform-native executables or run directly in a Wasm runtime.
//!
//! # Overview
//!
//! The compilation pipeline:
//!
//! ```text
//! Migration + MigrationPlan
//!         │
//!         ▼
//! ┌───────────────────┐
//! │ MigrationCompiler │  ─── Renders operations to SQL
//! └───────────────────┘      Generates Rust source code
//!         │
//!         ▼
//! CompilationResult
//! ├── source_code (Rust)
//! ├── statements
//! └── breaking_changes
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use tern::db::compile::{MigrationCompiler, CompilerConfig};
//! use tern::db::state::Migration;
//! use tern::db::migrate::MigrationPlan;
//! use tern::db::diff::diff_namespaces;
//!
//! // Create a migration from schema diff
//! let diff = diff_namespaces(&source, &target);
//! let plan = MigrationPlan::from_diff(&diff);
//! let migration = Migration::new(
//!     "Add email column",
//!     plan.operations.clone(),
//!     source_hash,
//!     target_hash,
//!     breaking_changes,
//! );
//!
//! // Compile to source code
//! let compiler = MigrationCompiler::new();
//! let result = compiler.compile(&migration, &plan)?;
//!
//! // The result contains:
//! // - result.source_code: Rust source using define_migration! macro
//! // - result.statements: The compiled SQL statements
//! // - result.breaking_changes: Breaking changes with affected SQL
//! println!("Generated {} statements", result.statement_count());
//! println!("{}", result.source_code);
//! ```
//!
//! # Generated Code
//!
//! The compiler generates Rust source code that uses the `define_migration!`
//! macro from the `tern-migration-guest` crate:
//!
//! ```ignore
//! use tern_migration_guest::define_migration;
//!
//! define_migration! {
//!     id: "abc123...",
//!     description: "Add email column",
//!     source_state_hash: "...",
//!     target_state_hash: "...",
//!     compiled_at: "2024-01-15T10:00:00Z",
//!     breaking_changes: [],
//!     statements: [
//!         ("Add column", "ALTER TABLE users ADD COLUMN email TEXT"),
//!     ]
//! }
//! ```
//!
//! # Breaking Changes
//!
//! The compiler tracks breaking changes and associates them with the
//! SQL statements that cause them. This allows the runtime to:
//!
//! - Warn users about potentially dangerous operations
//! - Require explicit confirmation for destructive changes
//! - Provide guidance on safe migration patterns

mod codegen;
mod error;

pub use codegen::{
    CompilationResult, CompiledBreakingChange, CompiledStatement, CompilerConfig, MigrationCompiler,
};
pub use error::CompileError;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::diff::diff_namespaces;
    use crate::db::migrate::MigrationPlan;
    use crate::db::model::types::{QualifiedCollationName, TypeInfo};
    use crate::db::model::{Column, Namespace, Table, TableKind};
    use crate::db::schema::{CollationName, ColumnName, Oid, SchemaName, TableName, TypeName};
    use crate::db::state::{Migration, StateHash};

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn default_collation() -> QualifiedCollationName {
        QualifiedCollationName::new(
            SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            CollationName::try_new("default".to_string()).unwrap(),
        )
    }

    fn make_type_info(name: &str) -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new(name.to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: name.to_string(),
            is_array: false,
        }
    }

    fn simple_column(name: &str, type_name: &str) -> Column {
        Column {
            position: 1,
            name: ColumnName::try_new(name.to_string()).unwrap(),
            type_info: make_type_info(type_name),
            is_nullable: true,
            default: None,
            identity: None,
            generated: None,
            collation: default_collation(),
            comment: None,
        }
    }

    fn simple_table_with_columns(name: &str, columns: Vec<Column>) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns,
            constraints: vec![],
            indexes: vec![],
            comment: None,
        }
    }

    /// Integration test: compile a migration that adds a column.
    #[test]
    fn integration_compile_add_column_migration() {
        // Source: table with one column
        let source = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![simple_table_with_columns(
                "users",
                vec![simple_column("id", "integer")],
            )],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        // Target: table with two columns
        let target = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![simple_table_with_columns(
                "users",
                vec![
                    simple_column("id", "integer"),
                    simple_column("email", "text"),
                ],
            )],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        // Compute diff and plan
        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);

        // Create migration
        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);
        let migration = Migration::new(
            "Add email column to users table",
            plan.operations.clone(),
            source_hash,
            target_hash,
            vec![], // No breaking changes for adding nullable column
        );

        // Compile
        let compiler = MigrationCompiler::new();
        let result = compiler.compile(&migration, &plan).unwrap();

        // Verify results
        assert_eq!(result.description, "Add email column to users table");
        assert!(result.statement_count() > 0);
        assert!(!result.has_breaking_changes());

        // Check source code
        assert!(result.source_code.contains("define_migration!"));
        assert!(result.source_code.contains("Add email column"));
        assert!(result.source_code.contains("ALTER TABLE"));
        assert!(result.source_code.contains("email"));
    }

    /// Integration test: compile a migration that drops a table.
    #[test]
    fn integration_compile_drop_table_migration() {
        use crate::db::diff::breaking::analyze_breaking_changes;

        // Source: has users table
        let source = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![simple_table_with_columns(
                "users",
                vec![simple_column("id", "integer")],
            )],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        // Target: no tables
        let target = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        // Compute diff and plan
        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);

        // Analyze breaking changes
        let breaking_changes = analyze_breaking_changes(&diff);

        // Create migration
        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);
        let migration = Migration::new(
            "Drop users table",
            plan.operations.clone(),
            source_hash,
            target_hash,
            breaking_changes.into_changes(),
        );

        // Compile
        let compiler = MigrationCompiler::new();
        let result = compiler.compile(&migration, &plan).unwrap();

        // Verify results
        assert!(result.has_breaking_changes());
        assert!(result.has_destructive_changes());
        assert!(result.source_code.contains("Destructive"));
        assert!(result.source_code.contains("DROP TABLE"));
    }

    /// Integration test: compile a migration with multiple operations.
    #[test]
    fn integration_compile_complex_migration() {
        use crate::db::model::EnumType;

        // Source: empty
        let source = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        // Target: table and enum
        let target = Namespace {
            oid: Oid::new(1),
            name: test_schema(),
            tables: vec![simple_table_with_columns(
                "users",
                vec![
                    simple_column("id", "integer"),
                    simple_column("name", "text"),
                    simple_column("status", "user_status"),
                ],
            )],
            views: vec![],
            sequences: vec![],
            enums: vec![EnumType {
                oid: Oid::new(2),
                name: TypeName::try_new("user_status".to_string()).unwrap(),
                values: vec!["active".to_string(), "inactive".to_string()],
                comment: None,
            }],
            comment: None,
        };

        // Compute diff and plan
        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);

        // Create migration
        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);
        let migration = Migration::new(
            "Create users table with status enum",
            plan.operations.clone(),
            source_hash,
            target_hash,
            vec![],
        );

        // Compile
        let compiler = MigrationCompiler::new();
        let result = compiler.compile(&migration, &plan).unwrap();

        // Verify results
        assert!(result.statement_count() >= 2); // At least enum and table
        assert!(result.source_code.contains("CREATE TYPE"));
        assert!(result.source_code.contains("CREATE TABLE"));
        assert!(result.source_code.contains("user_status"));
    }

    /// Test that the generated code escapes strings properly.
    #[test]
    fn integration_special_characters_escaped() {
        let source = Namespace::empty("public");
        let target = Namespace::empty("public");

        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);

        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);

        // Description with special characters
        let migration = Migration::new(
            r#"Add "quoted" column with 'single quotes' and \backslashes\"#,
            plan.operations.clone(),
            source_hash,
            target_hash,
            vec![],
        );

        let compiler = MigrationCompiler::new();
        let result = compiler.compile(&migration, &plan).unwrap();

        // Verify escaping
        // Double quotes must be escaped in Rust strings
        assert!(result.source_code.contains("\\\"quoted\\\""));
        // Single quotes do NOT need escaping in Rust string literals
        assert!(result.source_code.contains("'single quotes'"));
        // Backslashes must be escaped
        assert!(result.source_code.contains("\\\\backslashes\\\\"));
    }

    /// Test compilation with macro-only config.
    #[test]
    fn integration_macro_only_output() {
        let source = Namespace::empty("public");
        let target = Namespace::empty("public");

        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);

        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);

        let migration = Migration::new(
            "Empty migration",
            plan.operations.clone(),
            source_hash,
            target_hash,
            vec![],
        );

        let compiler = MigrationCompiler::with_config(CompilerConfig::macro_only());
        let result = compiler.compile(&migration, &plan).unwrap();

        // Should start directly with macro, no header
        assert!(result.source_code.starts_with("define_migration!"));
        assert!(!result.source_code.contains("use tern_migration_guest"));
    }
}
