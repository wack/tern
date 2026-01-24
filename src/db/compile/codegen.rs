//! Source code generation for migration components.
//!
//! This module provides functionality to generate Rust source code for
//! migration components that can be compiled to WebAssembly.
//!
//! # Overview
//!
//! The code generation process:
//! 1. Takes a `Migration` and `MigrationPlan`
//! 2. Renders operations to SQL using `PostgresRenderer`
//! 3. Generates Rust source using the `define_migration!` macro
//! 4. Outputs source code ready for compilation
//!
//! # Example
//!
//! ```ignore
//! use tern::db::compile::{MigrationCompiler, CompilerConfig};
//! use tern::db::state::Migration;
//! use tern::db::migrate::MigrationPlan;
//!
//! let compiler = MigrationCompiler::new();
//! let source = compiler.generate_source(&migration, &plan)?;
//! ```

use jiff::Timestamp;

use crate::db::diff::breaking::{BreakingChange, MitigationStrategy};
use crate::db::migrate::{MigrationPlan, MigrationScript, PostgresRenderer, RenderConfig};
use crate::db::state::{Migration, MigrationId, StateHash};

use super::error::CompileError;

/// Configuration for the migration compiler.
#[derive(Debug, Clone)]
pub struct CompilerConfig {
    /// Configuration for SQL rendering.
    pub render_config: RenderConfig,
    /// Whether to include the full Rust file with imports.
    pub include_full_file: bool,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            render_config: RenderConfig::default(),
            include_full_file: true,
        }
    }
}

impl CompilerConfig {
    /// Create a config that only generates the macro invocation.
    pub fn macro_only() -> Self {
        Self {
            include_full_file: false,
            ..Default::default()
        }
    }
}

/// A compiled statement with its SQL and description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledStatement {
    /// Human-readable description of what this statement does.
    pub description: String,
    /// The SQL statement text.
    pub sql: String,
}

impl CompiledStatement {
    /// Create a new compiled statement.
    pub fn new(description: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            sql: sql.into(),
        }
    }
}

/// A compiled breaking change with metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledBreakingChange {
    /// Human-readable description of the breaking change.
    pub description: String,
    /// Strategy for safely executing this change.
    pub mitigation: MitigationStrategy,
    /// The SQL statement(s) that cause this breaking change.
    pub affected_sql: Vec<String>,
}

impl CompiledBreakingChange {
    /// Create from a `BreakingChange`.
    pub fn from_breaking_change(bc: &BreakingChange, affected_sql: Vec<String>) -> Self {
        Self {
            description: bc.description.clone(),
            mitigation: bc.mitigation,
            affected_sql,
        }
    }
}

/// Result of compiling a migration.
#[derive(Debug, Clone)]
pub struct CompilationResult {
    /// The migration ID.
    pub id: MigrationId,
    /// Human-readable description.
    pub description: String,
    /// Hash of the source schema state.
    pub source_state_hash: StateHash,
    /// Hash of the target schema state.
    pub target_state_hash: StateHash,
    /// When this compilation occurred.
    pub compiled_at: Timestamp,
    /// The compiled SQL statements.
    pub statements: Vec<CompiledStatement>,
    /// Compiled breaking changes.
    pub breaking_changes: Vec<CompiledBreakingChange>,
    /// The generated Rust source code.
    pub source_code: String,
}

impl CompilationResult {
    /// Returns true if this migration has breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        !self.breaking_changes.is_empty()
    }

    /// Returns true if this migration has destructive changes.
    pub fn has_destructive_changes(&self) -> bool {
        self.breaking_changes
            .iter()
            .any(|bc| bc.mitigation == MitigationStrategy::Destructive)
    }

    /// Returns the number of SQL statements.
    pub fn statement_count(&self) -> usize {
        self.statements.len()
    }
}

/// Compiler for generating migration component source code.
///
/// The `MigrationCompiler` takes a migration definition and generates
/// Rust source code that implements the `Migration` trait using the
/// `define_migration!` macro.
#[derive(Debug, Clone)]
pub struct MigrationCompiler {
    config: CompilerConfig,
}

impl Default for MigrationCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl MigrationCompiler {
    /// Create a new migration compiler with default configuration.
    pub fn new() -> Self {
        Self {
            config: CompilerConfig::default(),
        }
    }

    /// Create a new migration compiler with custom configuration.
    pub fn with_config(config: CompilerConfig) -> Self {
        Self { config }
    }

    /// Compile a migration to source code.
    ///
    /// # Arguments
    ///
    /// * `migration` - The migration metadata and breaking changes
    /// * `plan` - The migration plan with operations
    ///
    /// # Returns
    ///
    /// A `CompilationResult` containing the generated source code and metadata.
    pub fn compile(
        &self,
        migration: &Migration,
        plan: &MigrationPlan,
    ) -> Result<CompilationResult, CompileError> {
        // Create renderer with the configured settings
        let renderer = PostgresRenderer::new(self.config.render_config.clone());

        // Render operations to SQL
        let script = plan.render(&renderer);

        // Collect statements from rendered operations
        let statements = self.collect_statements(&script);

        // Map breaking changes to compiled format
        let breaking_changes = self.compile_breaking_changes(migration, &statements);

        // Generate the source code
        let compiled_at = Timestamp::now();
        let source_code = self.generate_source(
            migration,
            &statements,
            &breaking_changes,
            compiled_at,
            self.config.include_full_file,
        )?;

        Ok(CompilationResult {
            id: migration.id,
            description: migration.description.clone(),
            source_state_hash: migration.parent_state_hash,
            target_state_hash: migration.resulting_state_hash,
            compiled_at,
            statements,
            breaking_changes,
            source_code,
        })
    }

    /// Collect statements from a migration script.
    fn collect_statements(&self, script: &MigrationScript) -> Vec<CompiledStatement> {
        script
            .operations
            .iter()
            .flat_map(|op| {
                op.forward
                    .iter()
                    .map(|sql| CompiledStatement::new(op.description.clone(), sql.clone()))
            })
            .collect()
    }

    /// Compile breaking changes, associating each with affected SQL.
    fn compile_breaking_changes(
        &self,
        migration: &Migration,
        statements: &[CompiledStatement],
    ) -> Vec<CompiledBreakingChange> {
        migration
            .breaking_changes
            .iter()
            .map(|bc| {
                // Find SQL statements that match this breaking change
                // For now, we associate all SQL with the breaking change
                // A more sophisticated approach would analyze which operations
                // correspond to which breaking changes
                let affected_sql = self.find_affected_sql(bc, statements);
                CompiledBreakingChange::from_breaking_change(bc, affected_sql)
            })
            .collect()
    }

    /// Find SQL statements affected by a breaking change.
    ///
    /// This uses heuristics to match breaking changes to their SQL statements.
    fn find_affected_sql(
        &self,
        bc: &BreakingChange,
        statements: &[CompiledStatement],
    ) -> Vec<String> {
        // Extract identifiers from the breaking change kind
        // These are the actual object names (tables, columns, etc.) that are affected
        let identifiers = Self::extract_identifiers_from_breaking_change(bc);

        statements
            .iter()
            .filter(|stmt| {
                let sql_lower = stmt.sql.to_lowercase();

                // If we have identifiers, check if any appear in the SQL
                for identifier in &identifiers {
                    let id_lower = identifier.to_lowercase();
                    if sql_lower.contains(&id_lower) {
                        return true;
                    }
                }

                false
            })
            .map(|stmt| stmt.sql.clone())
            .collect()
    }

    /// Extract identifier names from a breaking change.
    ///
    /// Returns table names, column names, constraint names, etc. that are
    /// referenced by the breaking change.
    fn extract_identifiers_from_breaking_change(bc: &BreakingChange) -> Vec<String> {
        use crate::db::diff::breaking::BreakingChangeKind;

        match &bc.kind {
            BreakingChangeKind::TableDropped { table } => vec![table.as_ref().to_string()],
            BreakingChangeKind::TableRenamed { from, to, .. } => {
                vec![from.as_ref().to_string(), to.as_ref().to_string()]
            }
            BreakingChangeKind::ColumnDropped { table, column } => {
                vec![table.as_ref().to_string(), column.as_ref().to_string()]
            }
            BreakingChangeKind::ColumnRenamed {
                table, from, to, ..
            } => vec![
                table.as_ref().to_string(),
                from.as_ref().to_string(),
                to.as_ref().to_string(),
            ],
            BreakingChangeKind::ColumnTypeChanged { table, column, .. } => {
                vec![table.as_ref().to_string(), column.as_ref().to_string()]
            }
            BreakingChangeKind::ColumnMadeNonNullable { table, column } => {
                vec![table.as_ref().to_string(), column.as_ref().to_string()]
            }
            BreakingChangeKind::ViewDropped { view } => vec![view.as_ref().to_string()],
            BreakingChangeKind::ViewRenamed { from, to, .. } => {
                vec![from.as_ref().to_string(), to.as_ref().to_string()]
            }
            BreakingChangeKind::MaterializationChanged { view, .. } => {
                vec![view.as_ref().to_string()]
            }
            BreakingChangeKind::SequenceDropped { sequence } => {
                vec![sequence.as_ref().to_string()]
            }
            BreakingChangeKind::SequenceRenamed { from, to, .. } => {
                vec![from.as_ref().to_string(), to.as_ref().to_string()]
            }
            BreakingChangeKind::PrimaryKeyAdded {
                table, constraint, ..
            } => vec![table.as_ref().to_string(), constraint.as_ref().to_string()],
            BreakingChangeKind::UniqueConstraintAdded {
                table, constraint, ..
            } => vec![table.as_ref().to_string(), constraint.as_ref().to_string()],
            BreakingChangeKind::CheckConstraintAdded {
                table, constraint, ..
            } => vec![table.as_ref().to_string(), constraint.as_ref().to_string()],
            BreakingChangeKind::ForeignKeyAdded {
                table, constraint, ..
            } => vec![table.as_ref().to_string(), constraint.as_ref().to_string()],
            BreakingChangeKind::ExclusionConstraintAdded {
                table, constraint, ..
            } => vec![table.as_ref().to_string(), constraint.as_ref().to_string()],
            BreakingChangeKind::EnumValueRemoved { enum_type, values } => {
                let mut identifiers = vec![enum_type.as_ref().to_string()];
                identifiers.extend(values.iter().cloned());
                identifiers
            }
            BreakingChangeKind::EnumValuesReordered { enum_type, .. } => {
                vec![enum_type.as_ref().to_string()]
            }
        }
    }

    /// Generate the Rust source code for a migration component.
    fn generate_source(
        &self,
        migration: &Migration,
        statements: &[CompiledStatement],
        breaking_changes: &[CompiledBreakingChange],
        compiled_at: Timestamp,
        include_full_file: bool,
    ) -> Result<String, CompileError> {
        let mut code = String::new();

        if include_full_file {
            code.push_str(&self.generate_file_header());
        }

        code.push_str(&self.generate_macro_invocation(
            migration,
            statements,
            breaking_changes,
            compiled_at,
        )?);

        Ok(code)
    }

    /// Generate the file header with imports.
    fn generate_file_header(&self) -> String {
        r#"//! Generated migration component.
//!
//! This file was automatically generated by the Tern migration compiler.
//! Do not edit manually.

use tern_migration_guest::define_migration;

"#
        .to_string()
    }

    /// Generate the macro invocation.
    fn generate_macro_invocation(
        &self,
        migration: &Migration,
        statements: &[CompiledStatement],
        breaking_changes: &[CompiledBreakingChange],
        compiled_at: Timestamp,
    ) -> Result<String, CompileError> {
        let mut code = String::new();

        code.push_str("define_migration! {\n");

        // ID
        code.push_str(&format!("    id: \"{}\",\n", migration.id.to_hex()));

        // Description (escape quotes and backslashes)
        let escaped_desc = escape_string(&migration.description);
        code.push_str(&format!("    description: \"{}\",\n", escaped_desc));

        // State hashes
        code.push_str(&format!(
            "    source_state_hash: \"{}\",\n",
            migration.parent_state_hash.to_hex()
        ));
        code.push_str(&format!(
            "    target_state_hash: \"{}\",\n",
            migration.resulting_state_hash.to_hex()
        ));

        // Compiled timestamp (RFC 3339 format)
        code.push_str(&format!("    compiled_at: \"{compiled_at}\",\n"));

        // Breaking changes
        code.push_str("    breaking_changes: [");
        if breaking_changes.is_empty() {
            code.push_str("],\n");
        } else {
            code.push('\n');
            for bc in breaking_changes {
                code.push_str(&self.format_breaking_change(bc));
            }
            code.push_str("    ],\n");
        }

        // Statements
        code.push_str("    statements: [");
        if statements.is_empty() {
            code.push_str("]\n");
        } else {
            code.push('\n');
            for (i, stmt) in statements.iter().enumerate() {
                let trailing_comma = if i < statements.len() - 1 { "," } else { "" };
                code.push_str(&self.format_statement(stmt, trailing_comma));
            }
            code.push_str("    ]\n");
        }

        code.push_str("}\n");

        Ok(code)
    }

    /// Format a single statement for the macro.
    fn format_statement(&self, stmt: &CompiledStatement, trailing_comma: &str) -> String {
        let escaped_desc = escape_string(&stmt.description);
        let escaped_sql = escape_string(&stmt.sql);
        format!(
            "        (\"{}\", \"{}\"){}\n",
            escaped_desc, escaped_sql, trailing_comma
        )
    }

    /// Format a breaking change for the macro.
    fn format_breaking_change(&self, bc: &CompiledBreakingChange) -> String {
        let mut code = String::new();

        code.push_str("        {\n");
        code.push_str(&format!(
            "            description: \"{}\",\n",
            escape_string(&bc.description)
        ));
        code.push_str(&format!(
            "            mitigation: {},\n",
            format_mitigation_strategy(&bc.mitigation)
        ));

        code.push_str("            affected_sql: [");
        if bc.affected_sql.is_empty() {
            code.push_str("]\n");
        } else {
            code.push('\n');
            for (i, sql) in bc.affected_sql.iter().enumerate() {
                let trailing_comma = if i < bc.affected_sql.len() - 1 {
                    ","
                } else {
                    ""
                };
                code.push_str(&format!(
                    "                \"{}\"{}\n",
                    escape_string(sql),
                    trailing_comma
                ));
            }
            code.push_str("            ]\n");
        }

        code.push_str("        },\n");

        code
    }
}

/// Escape a string for use in Rust source code.
fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Format a mitigation strategy as a Rust identifier for the macro.
fn format_mitigation_strategy(strategy: &MitigationStrategy) -> &'static str {
    match strategy {
        MitigationStrategy::DualWrite => "DualWrite",
        MitigationStrategy::Backfill => "Backfill",
        MitigationStrategy::Ratchet => "Ratchet",
        MitigationStrategy::Destructive => "Destructive",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::diff::breaking::BreakingChangeKind;
    use crate::db::migrate::Operation;
    use crate::db::model::{Table, TableKind};
    use crate::db::schema::{Oid, SchemaName, TableName};

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn simple_table(name: &str) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        }
    }

    mod escape_string_tests {
        use super::*;

        #[test]
        fn escapes_backslashes() {
            assert_eq!(escape_string("foo\\bar"), "foo\\\\bar");
        }

        #[test]
        fn escapes_quotes() {
            assert_eq!(escape_string("foo\"bar"), "foo\\\"bar");
        }

        #[test]
        fn escapes_newlines() {
            assert_eq!(escape_string("foo\nbar"), "foo\\nbar");
        }

        #[test]
        fn escapes_tabs() {
            assert_eq!(escape_string("foo\tbar"), "foo\\tbar");
        }

        #[test]
        fn escapes_carriage_returns() {
            assert_eq!(escape_string("foo\rbar"), "foo\\rbar");
        }

        #[test]
        fn handles_multiple_escapes() {
            assert_eq!(escape_string("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        }

        #[test]
        fn handles_empty_string() {
            assert_eq!(escape_string(""), "");
        }

        #[test]
        fn handles_no_escapes_needed() {
            assert_eq!(escape_string("hello world"), "hello world");
        }
    }

    mod compiled_statement_tests {
        use super::*;

        #[test]
        fn new_creates_statement() {
            let stmt = CompiledStatement::new("Create table", "CREATE TABLE foo (id INT)");
            assert_eq!(stmt.description, "Create table");
            assert_eq!(stmt.sql, "CREATE TABLE foo (id INT)");
        }
    }

    mod compiled_breaking_change_tests {
        use super::*;

        #[test]
        fn from_breaking_change() {
            let bc = BreakingChange::new(BreakingChangeKind::TableDropped {
                table: TableName::try_new("users".to_string()).unwrap(),
            });

            let affected_sql = vec!["DROP TABLE users".to_string()];
            let compiled = CompiledBreakingChange::from_breaking_change(&bc, affected_sql);

            assert_eq!(compiled.description, "Table 'users' was dropped");
            assert_eq!(compiled.mitigation, MitigationStrategy::Destructive);
            assert_eq!(compiled.affected_sql.len(), 1);
        }
    }

    mod compiler_config_tests {
        use super::*;

        #[test]
        fn default_includes_full_file() {
            let config = CompilerConfig::default();
            assert!(config.include_full_file);
        }

        #[test]
        fn macro_only_disables_full_file() {
            let config = CompilerConfig::macro_only();
            assert!(!config.include_full_file);
        }
    }

    mod migration_compiler_tests {
        use super::*;

        fn create_test_migration() -> Migration {
            Migration::new(
                "Test migration",
                vec![],
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                vec![],
            )
        }

        fn create_migration_with_operations() -> (Migration, MigrationPlan) {
            let table = simple_table("users");
            let ops = vec![Operation::CreateTable {
                schema: test_schema(),
                table,
            }];

            let migration = Migration::new(
                "Add users table",
                ops.clone(),
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                vec![],
            );

            let plan = MigrationPlan::from_operations(ops);

            (migration, plan)
        }

        #[test]
        fn new_creates_default_compiler() {
            let compiler = MigrationCompiler::new();
            assert!(compiler.config.include_full_file);
        }

        #[test]
        fn with_config_uses_custom_config() {
            let config = CompilerConfig::macro_only();
            let compiler = MigrationCompiler::with_config(config);
            assert!(!compiler.config.include_full_file);
        }

        #[test]
        fn compile_empty_migration() {
            let compiler = MigrationCompiler::new();
            let migration = create_test_migration();
            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            assert_eq!(result.id, migration.id);
            assert_eq!(result.description, "Test migration");
            assert!(result.statements.is_empty());
            assert!(!result.has_breaking_changes());
        }

        #[test]
        fn compile_with_operations() {
            let compiler = MigrationCompiler::new();
            let (migration, plan) = create_migration_with_operations();

            let result = compiler.compile(&migration, &plan).unwrap();

            assert!(!result.statements.is_empty());
            // Should have CREATE TABLE statement
            assert!(
                result
                    .statements
                    .iter()
                    .any(|s| s.sql.to_uppercase().contains("CREATE TABLE"))
            );
        }

        #[test]
        fn compile_generates_valid_source() {
            let compiler = MigrationCompiler::new();
            let (migration, plan) = create_migration_with_operations();

            let result = compiler.compile(&migration, &plan).unwrap();

            // Check source code contains expected elements
            assert!(result.source_code.contains("define_migration!"));
            assert!(result.source_code.contains(&migration.id.to_hex()));
            assert!(result.source_code.contains("Add users table"));
            assert!(result.source_code.contains("source_state_hash:"));
            assert!(result.source_code.contains("target_state_hash:"));
            assert!(result.source_code.contains("compiled_at:"));
            assert!(result.source_code.contains("statements:"));
        }

        #[test]
        fn compile_macro_only_omits_header() {
            let compiler = MigrationCompiler::with_config(CompilerConfig::macro_only());
            let migration = create_test_migration();
            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            assert!(!result.source_code.contains("use tern_migration_guest"));
            assert!(result.source_code.starts_with("define_migration!"));
        }

        #[test]
        fn compile_with_full_file_includes_header() {
            let compiler = MigrationCompiler::new();
            let migration = create_test_migration();
            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            assert!(
                result
                    .source_code
                    .contains("//! Generated migration component")
            );
            assert!(
                result
                    .source_code
                    .contains("use tern_migration_guest::define_migration")
            );
        }
    }

    mod breaking_change_tests {
        use super::*;

        #[test]
        fn compile_with_breaking_changes() {
            let compiler = MigrationCompiler::new();
            let table = simple_table("users");

            let ops = vec![Operation::DropTable {
                schema: test_schema(),
                name: table.name.clone(),
            }];

            let breaking_changes = vec![BreakingChange::new(BreakingChangeKind::TableDropped {
                table: table.name.clone(),
            })];

            let migration = Migration::new(
                "Drop users table",
                ops.clone(),
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                breaking_changes,
            );

            let plan = MigrationPlan::from_operations(ops);

            let result = compiler.compile(&migration, &plan).unwrap();

            assert!(result.has_breaking_changes());
            assert!(result.has_destructive_changes());
            assert!(result.source_code.contains("breaking_changes:"));
            assert!(result.source_code.contains("Destructive"));
            assert!(result.source_code.contains("dropped"));
        }

        #[test]
        fn compile_with_multiple_breaking_changes() {
            let compiler = MigrationCompiler::new();

            let breaking_changes = vec![
                BreakingChange::new(BreakingChangeKind::TableDropped {
                    table: TableName::try_new("users".to_string()).unwrap(),
                }),
                BreakingChange::new(BreakingChangeKind::ColumnDropped {
                    table: TableName::try_new("posts".to_string()).unwrap(),
                    column: crate::db::schema::ColumnName::try_new("content".to_string()).unwrap(),
                }),
            ];

            let migration = Migration::new(
                "Destructive changes",
                vec![],
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                breaking_changes,
            );

            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            assert_eq!(result.breaking_changes.len(), 2);
            assert!(
                result
                    .breaking_changes
                    .iter()
                    .all(|bc| bc.mitigation == MitigationStrategy::Destructive)
            );
        }
    }

    mod format_mitigation_strategy_tests {
        use super::*;

        #[test]
        fn formats_all_strategies() {
            assert_eq!(
                format_mitigation_strategy(&MitigationStrategy::DualWrite),
                "DualWrite"
            );
            assert_eq!(
                format_mitigation_strategy(&MitigationStrategy::Backfill),
                "Backfill"
            );
            assert_eq!(
                format_mitigation_strategy(&MitigationStrategy::Ratchet),
                "Ratchet"
            );
            assert_eq!(
                format_mitigation_strategy(&MitigationStrategy::Destructive),
                "Destructive"
            );
        }
    }

    mod compilation_result_tests {
        use super::*;

        fn create_result(breaking_changes: Vec<CompiledBreakingChange>) -> CompilationResult {
            CompilationResult {
                id: MigrationId::zero(),
                description: "Test".to_string(),
                source_state_hash: StateHash::zero(),
                target_state_hash: StateHash::zero(),
                compiled_at: Timestamp::now(),
                statements: vec![CompiledStatement::new(
                    "Create table",
                    "CREATE TABLE t (id INT)",
                )],
                breaking_changes,
                source_code: String::new(),
            }
        }

        #[test]
        fn has_breaking_changes_false_when_empty() {
            let result = create_result(vec![]);
            assert!(!result.has_breaking_changes());
        }

        #[test]
        fn has_breaking_changes_true_when_present() {
            let bc = CompiledBreakingChange {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::DualWrite,
                affected_sql: vec![],
            };
            let result = create_result(vec![bc]);
            assert!(result.has_breaking_changes());
        }

        #[test]
        fn has_destructive_changes_false_for_dual_write() {
            let bc = CompiledBreakingChange {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::DualWrite,
                affected_sql: vec![],
            };
            let result = create_result(vec![bc]);
            assert!(!result.has_destructive_changes());
        }

        #[test]
        fn has_destructive_changes_true_for_destructive() {
            let bc = CompiledBreakingChange {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::Destructive,
                affected_sql: vec![],
            };
            let result = create_result(vec![bc]);
            assert!(result.has_destructive_changes());
        }

        #[test]
        fn statement_count_returns_correct_count() {
            let result = create_result(vec![]);
            assert_eq!(result.statement_count(), 1);
        }
    }

    mod source_code_format_tests {
        use super::*;

        #[test]
        fn generated_source_is_valid_rust_syntax() {
            let compiler = MigrationCompiler::new();
            let migration = Migration::new(
                "Test with 'quotes' and \"double quotes\"",
                vec![],
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                vec![],
            );
            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            // Ensure double quotes are properly escaped (single quotes don't need escaping in Rust strings)
            assert!(result.source_code.contains("\\\""));
            // Should not have unescaped double quotes inside strings
            let macro_start = result.source_code.find("define_migration!").unwrap();
            let macro_content = &result.source_code[macro_start..];
            // Check that the description is properly escaped
            // Note: single quotes don't need escaping in Rust string literals
            assert!(macro_content.contains("Test with 'quotes' and \\\"double quotes\\\""));
        }

        #[test]
        fn handles_special_characters_in_sql() {
            let compiler = MigrationCompiler::with_config(CompilerConfig::macro_only());

            // Create a migration with SQL containing special characters
            let _migration = Migration::new(
                "Test SQL",
                vec![],
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                vec![],
            );

            // We'll test the statement formatting directly
            let stmt = CompiledStatement::new(
                "Add check",
                "ALTER TABLE t ADD CONSTRAINT chk CHECK (name LIKE '%test%')",
            );

            let formatted = compiler.format_statement(&stmt, ",");

            // Should escape backslashes properly if any
            assert!(formatted.contains("LIKE"));
            assert!(formatted.contains("%test%"));
        }

        #[test]
        fn handles_multiline_descriptions() {
            let compiler = MigrationCompiler::with_config(CompilerConfig::macro_only());

            let migration = Migration::new(
                "Line 1\nLine 2\nLine 3",
                vec![],
                vec![],
                StateHash::zero(),
                StateHash::from_bytes([1u8; 32]),
                vec![],
            );
            let plan = MigrationPlan::empty();

            let result = compiler.compile(&migration, &plan).unwrap();

            // Newlines should be escaped
            assert!(result.source_code.contains("Line 1\\nLine 2\\nLine 3"));
            // Should not have actual newlines in the string literals
            let in_quotes = result
                .source_code
                .split("description:")
                .nth(1)
                .unwrap()
                .split(',')
                .next()
                .unwrap();
            assert!(!in_quotes.contains('\n') || in_quotes.contains("\\n"));
        }
    }

    mod find_affected_sql_tests {
        use super::*;

        #[test]
        fn finds_drop_statements() {
            let compiler = MigrationCompiler::new();
            let bc = BreakingChange::new(BreakingChangeKind::TableDropped {
                table: TableName::try_new("users".to_string()).unwrap(),
            });

            let statements = vec![
                CompiledStatement::new("Create table", "CREATE TABLE posts (id INT)"),
                CompiledStatement::new("Drop users", "DROP TABLE users"),
                CompiledStatement::new("Add column", "ALTER TABLE posts ADD COLUMN name TEXT"),
            ];

            let affected = compiler.find_affected_sql(&bc, &statements);

            assert_eq!(affected.len(), 1);
            assert!(affected[0].contains("DROP TABLE users"));
        }

        #[test]
        fn finds_rename_statements() {
            let compiler = MigrationCompiler::new();
            let bc = BreakingChange::new(BreakingChangeKind::TableRenamed {
                from: TableName::try_new("old_name".to_string()).unwrap(),
                to: TableName::try_new("new_name".to_string()).unwrap(),
                similarity: 0.9,
            });

            let statements = vec![CompiledStatement::new(
                "Rename table",
                "ALTER TABLE old_name RENAME TO new_name",
            )];

            let affected = compiler.find_affected_sql(&bc, &statements);

            assert_eq!(affected.len(), 1);
            assert!(affected[0].contains("RENAME"));
        }

        #[test]
        fn finds_constraint_statements() {
            let compiler = MigrationCompiler::new();
            let bc = BreakingChange::new(BreakingChangeKind::UniqueConstraintAdded {
                table: TableName::try_new("users".to_string()).unwrap(),
                constraint: crate::db::schema::ConstraintName::try_new(
                    "users_email_key".to_string(),
                )
                .unwrap(),
                columns: vec![crate::db::schema::ColumnName::try_new("email".to_string()).unwrap()],
            });

            let statements = vec![CompiledStatement::new(
                "Add unique constraint",
                "ALTER TABLE users ADD CONSTRAINT users_email_key UNIQUE (email)",
            )];

            let affected = compiler.find_affected_sql(&bc, &statements);

            assert!(!affected.is_empty());
            assert!(affected[0].contains("CONSTRAINT"));
        }

        #[test]
        fn returns_empty_when_no_match() {
            let compiler = MigrationCompiler::new();
            let bc = BreakingChange::new(BreakingChangeKind::TableDropped {
                table: TableName::try_new("users".to_string()).unwrap(),
            });

            let statements = vec![CompiledStatement::new(
                "Create posts",
                "CREATE TABLE posts (id INT)",
            )];

            let affected = compiler.find_affected_sql(&bc, &statements);

            // May find partial matches based on heuristics, but shouldn't match "posts"
            // when looking for "users dropped"
            for sql in &affected {
                assert!(!sql.contains("posts"));
            }
        }
    }
}
