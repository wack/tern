//! Migration compilation to WebAssembly components and standalone executables.
//!
//! This module provides the infrastructure for compiling database migrations
//! into standalone WebAssembly components and platform-native executables.
//! These can be run without requiring Tern to be installed.
//!
//! # Overview
//!
//! The compilation pipeline:
//!
//! ```text
//! Source Schema + Target Schema
//!         │
//!         ▼
//! ┌───────────────────────┐
//! │ compile_migration()   │  ─── High-level API
//! └───────────────────────┘
//!         │
//!         ▼
//! ┌───────────────────┐
//! │ Schema Diff       │  ─── Compare source to target
//! └───────────────────┘
//!         │
//!         ▼
//! ┌───────────────────┐
//! │ MigrationPlan     │  ─── Ordered operations
//! └───────────────────┘
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
//!         │
//!         ▼ (optional)
//! ┌───────────────────┐
//! │ ExecutableBuilder │  ─── Builds platform-native executable
//! └───────────────────┘
//! ```
//!
//! # Usage
//!
//! ## High-Level API
//!
//! The simplest way to compile a migration is using the `compile_migration` function:
//!
//! ```ignore
//! use tern::db::compile::{compile_migration, CompileOptions, Target};
//! use tern::db::model::Namespace;
//!
//! let result = compile_migration(
//!     &source_namespace,
//!     &target_namespace,
//!     CompileOptions::new("Add email column to users"),
//! )?;
//!
//! println!("Generated {} statements", result.compilation.statement_count());
//! ```
//!
//! ## Low-Level API
//!
//! For more control, you can use the compiler directly:
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
//!
//! # Standalone Executables
//!
//! Use the `ExecutableBuilder` to create platform-native executables:
//!
//! ```ignore
//! use tern::db::compile::{ExecutableBuilder, Target};
//! use std::path::Path;
//!
//! let builder = ExecutableBuilder::new();
//! let result = builder.build(
//!     &wasm_component_bytes,
//!     Path::new("./migrations/add_email"),
//!     Target::Native,
//! )?;
//! ```

mod aot;
mod codegen;
mod composer;
mod data_component;
mod embedded;
mod error;
mod executable;
pub mod oci;

pub use aot::{AotCompiler, AotOutput, AotResult, AotTarget};
pub use codegen::{
    CompilationResult, CompiledBreakingChange, CompiledStatement, CompilerConfig, MigrationCompiler,
};
pub use composer::{ComponentComposer, CompositionConfig};
pub use data_component::{
    BreakingChangeData, DataComponentGenerator, MigrationData, MitigationStrategy, StatementData,
};
pub use embedded::{
    GUEST_COMPONENT, RUNNER_COMPONENT, components_available, guest_component, guest_component_size,
    runner_component, runner_component_size, validate_wasm_bytes,
};
pub use error::CompileError;
pub use executable::{BuildResult, ExecutableBuilder, PackagedBuildResult, Target};
pub use oci::{OciBuildResult, OciConfig, OciImageBuilder};

// =============================================================================
// Package Format
// =============================================================================

/// Output format for packaged migration executables.
///
/// Specifies how the compiled migration should be packaged for distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PackageFormat {
    /// Standalone native binary executable.
    ///
    /// The executable can be run directly on the target platform.
    #[default]
    Binary,

    /// OCI (Open Container Initiative) image.
    ///
    /// The executable is packaged in an OCI-compliant container image
    /// that can be used with Docker, Podman, or Kubernetes.
    /// Output is a tar archive in OCI image layout format.
    Oci,
}

impl PackageFormat {
    /// Returns the default file extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Binary => "",
            Self::Oci => ".tar",
        }
    }

    /// Parse a format from a string representation.
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "binary" | "bin" | "executable" | "exe" => Some(Self::Binary),
            "oci" | "oci-image" | "container" | "docker" => Some(Self::Oci),
            _ => None,
        }
    }
}

impl std::fmt::Display for PackageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Binary => write!(f, "binary"),
            Self::Oci => write!(f, "oci"),
        }
    }
}

use crate::db::diff::breaking::analyze_breaking_changes;
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{MigrationPlan, compute_inverse_operations};
use crate::db::model::Namespace;
use crate::db::state::{Migration, StateHash};

// =============================================================================
// Compile Options
// =============================================================================

/// Options for compiling a migration.
///
/// Controls how the migration is compiled, including the description,
/// target platform, and compiler configuration.
#[derive(Debug, Clone)]
pub struct CompileOptions {
    /// Human-readable description of the migration.
    pub description: String,
    /// Target platform for executable generation (if building executable).
    pub target: Target,
    /// Configuration for the source code compiler.
    pub compiler_config: CompilerConfig,
}

impl CompileOptions {
    /// Creates new compile options with the given description.
    ///
    /// Uses default values for target (native) and compiler config.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            target: Target::Native,
            compiler_config: CompilerConfig::default(),
        }
    }

    /// Sets the target platform for executable generation.
    pub fn with_target(mut self, target: Target) -> Self {
        self.target = target;
        self
    }

    /// Sets the compiler configuration.
    pub fn with_compiler_config(mut self, config: CompilerConfig) -> Self {
        self.compiler_config = config;
        self
    }

    /// Creates options configured for macro-only output.
    pub fn macro_only(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            target: Target::Native,
            compiler_config: CompilerConfig::macro_only(),
        }
    }
}

// =============================================================================
// Compilation Pipeline Result
// =============================================================================

/// Result of the full migration compilation pipeline.
///
/// Contains all artifacts from compiling a migration, including the
/// migration record, the compilation result, and the plan.
#[derive(Debug, Clone)]
pub struct MigrationCompilationResult {
    /// The migration record.
    pub migration: Migration,
    /// The compiled source code and metadata.
    pub compilation: CompilationResult,
    /// The migration plan with ordered operations.
    pub plan: MigrationPlan,
    /// Source state hash.
    pub source_hash: StateHash,
    /// Target state hash.
    pub target_hash: StateHash,
}

impl MigrationCompilationResult {
    /// Returns the migration ID as a hex string.
    pub fn migration_id(&self) -> String {
        self.migration.id.to_hex()
    }

    /// Returns the number of SQL statements.
    pub fn statement_count(&self) -> usize {
        self.compilation.statement_count()
    }

    /// Returns true if there are breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        self.compilation.has_breaking_changes()
    }

    /// Returns true if there are destructive changes.
    pub fn has_destructive_changes(&self) -> bool {
        self.compilation.has_destructive_changes()
    }

    /// Returns the generated source code.
    pub fn source_code(&self) -> &str {
        &self.compilation.source_code
    }

    /// Returns an iterator over the SQL statements.
    pub fn statements(&self) -> impl Iterator<Item = &CompiledStatement> {
        self.compilation.statements.iter()
    }
}

// =============================================================================
// High-Level Compilation API
// =============================================================================

/// Compiles a migration from source schema to target schema.
///
/// This is the high-level API that handles the full compilation pipeline:
/// 1. Computes the diff between source and target schemas
/// 2. Analyzes breaking changes
/// 3. Creates a migration plan with ordered operations
/// 4. Generates the migration record
/// 5. Compiles to Rust source code
///
/// # Arguments
///
/// * `source` - The source schema (current state)
/// * `target` - The target schema (desired state)
/// * `options` - Compilation options including description
///
/// # Returns
///
/// Returns a `MigrationCompilationResult` containing the migration record,
/// compiled source code, and plan.
///
/// # Errors
///
/// Returns an error if:
/// - The schemas are identical (no changes to migrate)
/// - The compilation fails
///
/// # Example
///
/// ```ignore
/// use tern::db::compile::{compile_migration, CompileOptions};
/// use tern::db::model::Namespace;
///
/// let source = load_current_schema().await?;
/// let target = load_target_schema()?;
///
/// let result = compile_migration(&source, &target, CompileOptions::new("Add users table"))?;
///
/// println!("Migration ID: {}", result.migration_id());
/// println!("Statements: {}", result.statement_count());
/// if result.has_breaking_changes() {
///     println!("Warning: This migration has breaking changes!");
/// }
/// ```
pub fn compile_migration(
    source: &Namespace,
    target: &Namespace,
    options: CompileOptions,
) -> Result<MigrationCompilationResult, CompileError> {
    // Step 1: Compute the diff
    let diff = diff_namespaces(source, target);

    // Step 2: Create the migration plan
    let plan = MigrationPlan::from_diff(&diff);

    // Check if there are any changes
    if plan.is_empty() {
        return Err(CompileError::no_changes());
    }

    // Step 3: Analyze breaking changes
    let breaking_changes = analyze_breaking_changes(&diff);

    // Step 4: Compute state hashes
    let source_hash = StateHash::from_namespace(source);
    let target_hash = StateHash::from_namespace(target);

    // Step 4.5: Compute inverse operations for down migration
    let inverse_result = compute_inverse_operations(&plan.operations);
    let down_operations = inverse_result.operations;

    // Step 5: Create the migration record
    let migration = Migration::new(
        &options.description,
        plan.operations.clone(),
        down_operations,
        source_hash,
        target_hash,
        breaking_changes.into_changes(),
    );

    // Step 6: Compile to source code
    let compiler = MigrationCompiler::with_config(options.compiler_config);
    let compilation = compiler.compile(&migration, &plan)?;

    Ok(MigrationCompilationResult {
        migration,
        compilation,
        plan,
        source_hash,
        target_hash,
    })
}

/// Compiles a migration and verifies it produces the expected operations.
///
/// This is useful for testing and validation. It compiles the migration
/// and returns the result along with the SQL statements for inspection.
///
/// # Arguments
///
/// * `source` - The source schema (current state)
/// * `target` - The target schema (desired state)
/// * `description` - Human-readable description of the migration
///
/// # Returns
///
/// Returns a tuple of (MigrationCompilationResult, Vec<String>) where
/// the vector contains all SQL statements that would be executed.
pub fn compile_and_extract_sql(
    source: &Namespace,
    target: &Namespace,
    description: impl Into<String>,
) -> Result<(MigrationCompilationResult, Vec<String>), CompileError> {
    let result = compile_migration(source, target, CompileOptions::new(description))?;

    let sql_statements: Vec<String> = result
        .compilation
        .statements
        .iter()
        .map(|s| s.sql.clone())
        .collect();

    Ok((result, sql_statements))
}

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
            vec![],
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
            vec![],
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
            vec![],
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
            vec![],
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
            vec![],
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

    // =========================================================================
    // High-Level API Tests
    // =========================================================================

    mod compile_options_tests {
        use super::*;

        #[test]
        fn new_creates_with_description() {
            let options = CompileOptions::new("Test migration");
            assert_eq!(options.description, "Test migration");
            assert_eq!(options.target, Target::Native);
        }

        #[test]
        fn with_target_sets_target() {
            let options = CompileOptions::new("Test").with_target(Target::X86_64LinuxMusl);
            assert_eq!(options.target, Target::X86_64LinuxMusl);
        }

        #[test]
        fn with_compiler_config_sets_config() {
            let config = CompilerConfig::macro_only();
            let options = CompileOptions::new("Test").with_compiler_config(config.clone());
            assert!(!options.compiler_config.include_full_file);
        }

        #[test]
        fn macro_only_creates_correct_options() {
            let options = CompileOptions::macro_only("Test");
            assert_eq!(options.description, "Test");
            assert!(!options.compiler_config.include_full_file);
        }
    }

    mod compile_migration_tests {
        use super::*;

        #[test]
        fn compile_migration_add_column() {
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

            let result =
                compile_migration(&source, &target, CompileOptions::new("Add email column"))
                    .unwrap();

            assert!(result.statement_count() > 0);
            assert!(!result.has_breaking_changes());
            assert!(result.source_code().contains("ALTER TABLE"));
            assert!(result.source_code().contains("email"));
        }

        #[test]
        fn compile_migration_returns_error_for_no_changes() {
            let source = Namespace::empty("public");
            let target = Namespace::empty("public");

            let result = compile_migration(&source, &target, CompileOptions::new("No changes"));

            assert!(matches!(result, Err(CompileError::NoChanges)));
        }

        #[test]
        fn compile_migration_detects_breaking_changes() {
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

            let target = Namespace {
                oid: Oid::new(1),
                name: test_schema(),
                tables: vec![],
                views: vec![],
                sequences: vec![],
                enums: vec![],
                comment: None,
            };

            let result =
                compile_migration(&source, &target, CompileOptions::new("Drop users table"))
                    .unwrap();

            assert!(result.has_breaking_changes());
            assert!(result.has_destructive_changes());
        }

        #[test]
        fn compile_migration_generates_migration_id() {
            let source = Namespace::empty("public");
            let target = Namespace {
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

            let result =
                compile_migration(&source, &target, CompileOptions::new("Create users table"))
                    .unwrap();

            // Migration ID should be a 64-character hex string (256 bits)
            let id = result.migration_id();
            assert_eq!(id.len(), 64);
            assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        }

        #[test]
        fn compile_migration_computes_state_hashes() {
            let source = Namespace::empty("public");
            let target = Namespace {
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

            let result =
                compile_migration(&source, &target, CompileOptions::new("Create users table"))
                    .unwrap();

            // Hashes should be different since schemas are different
            assert_ne!(result.source_hash, result.target_hash);
        }

        #[test]
        fn compile_migration_includes_plan() {
            let source = Namespace::empty("public");
            let target = Namespace {
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

            let result =
                compile_migration(&source, &target, CompileOptions::new("Create users table"))
                    .unwrap();

            assert!(!result.plan.is_empty());
            assert!(result.plan.len() > 0);
        }
    }

    mod compile_and_extract_sql_tests {
        use super::*;

        #[test]
        fn extracts_sql_statements() {
            let source = Namespace::empty("public");
            let target = Namespace {
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

            let (result, sql) =
                compile_and_extract_sql(&source, &target, "Create users table").unwrap();

            assert!(!sql.is_empty());
            assert_eq!(sql.len(), result.statement_count());

            // At least one statement should contain CREATE TABLE
            assert!(sql.iter().any(|s| s.contains("CREATE TABLE")));
        }

        #[test]
        fn returns_error_for_no_changes() {
            let source = Namespace::empty("public");
            let target = Namespace::empty("public");

            let result = compile_and_extract_sql(&source, &target, "No changes");

            assert!(matches!(result, Err(CompileError::NoChanges)));
        }
    }

    mod migration_compilation_result_tests {
        use super::*;

        #[test]
        fn statements_iterator_works() {
            let source = Namespace::empty("public");
            let target = Namespace {
                oid: Oid::new(1),
                name: test_schema(),
                tables: vec![simple_table_with_columns(
                    "users",
                    vec![
                        simple_column("id", "integer"),
                        simple_column("name", "text"),
                    ],
                )],
                views: vec![],
                sequences: vec![],
                enums: vec![],
                comment: None,
            };

            let result =
                compile_migration(&source, &target, CompileOptions::new("Create users table"))
                    .unwrap();

            let statements: Vec<_> = result.statements().collect();
            assert!(!statements.is_empty());

            // All statements should have SQL content
            for stmt in statements {
                assert!(!stmt.sql.is_empty());
            }
        }
    }
}
