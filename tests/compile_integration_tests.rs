//! Integration tests for the migration compilation pipeline.
//!
//! These tests exercise the full compilation pipeline:
//! 1. Schema construction with builders
//! 2. Schema diffing
//! 3. Breaking change analysis
//! 4. Migration compilation to source code
//! 5. Verification of generated artifacts
//!
//! Snapshots are stored in tests/snapshots/ and managed by cargo-insta.

use insta::{assert_snapshot, assert_yaml_snapshot};
use tern::db::compile::{CompileOptions, CompilerConfig, MigrationCompiler, compile_migration};
use tern::db::diff::breaking::analyze_breaking_changes;
use tern::db::diff::diff_namespaces;
use tern::db::history::builder::NamespaceBuilder;
use tern::db::migrate::MigrationPlan;
use tern::db::model::types::ForeignKeyAction;
use tern::db::state::{Migration, StateHash};

// ============================================================================
// Test Helpers
// ============================================================================

/// Extract SQL statements from a compilation result for snapshot testing.
fn extract_sql_statements(
    source: &tern::db::model::Namespace,
    target: &tern::db::model::Namespace,
) -> Vec<String> {
    let result = compile_migration(source, target, CompileOptions::new("Test migration")).unwrap();
    result.statements().map(|s| s.sql.clone()).collect()
}

/// Redact the compiled_at timestamp from generated source code for deterministic snapshots.
fn redact_compiled_at(source_code: &str) -> String {
    // Find the compiled_at line and replace the timestamp
    source_code
        .lines()
        .map(|line| {
            if line.trim().starts_with("compiled_at:") {
                // Replace the timestamp with a placeholder
                let indent = line.len() - line.trim_start().len();
                format!("{}compiled_at: \"[TIMESTAMP]\",", " ".repeat(indent))
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Create a summary of compilation results for YAML snapshot.
#[derive(Debug, serde::Serialize)]
struct CompilationSummary {
    migration_id: String,
    description: String,
    statement_count: usize,
    has_breaking_changes: bool,
    has_destructive_changes: bool,
    source_hash: String,
    target_hash: String,
    breaking_changes: Vec<BreakingChangeSummary>,
    sql_statements: Vec<StatementSummary>,
}

#[derive(Debug, serde::Serialize)]
struct BreakingChangeSummary {
    description: String,
    mitigation: String,
}

#[derive(Debug, serde::Serialize)]
struct StatementSummary {
    description: String,
    sql: String,
}

fn create_compilation_summary(
    source: &tern::db::model::Namespace,
    target: &tern::db::model::Namespace,
    description: &str,
) -> CompilationSummary {
    let result = compile_migration(source, target, CompileOptions::new(description)).unwrap();

    CompilationSummary {
        migration_id: result.migration_id(),
        description: description.to_string(),
        statement_count: result.statement_count(),
        has_breaking_changes: result.has_breaking_changes(),
        has_destructive_changes: result.has_destructive_changes(),
        source_hash: result.source_hash.to_short_hex(),
        target_hash: result.target_hash.to_short_hex(),
        breaking_changes: result
            .compilation
            .breaking_changes
            .iter()
            .map(|bc| BreakingChangeSummary {
                description: bc.description.clone(),
                mitigation: format!("{:?}", bc.mitigation),
            })
            .collect(),
        sql_statements: result
            .statements()
            .map(|s| StatementSummary {
                description: s.description.clone(),
                sql: s.sql.clone(),
            })
            .collect(),
    }
}

// ============================================================================
// Basic Compilation Tests
// ============================================================================

mod simple_migrations {
    use super::*;

    #[test]
    fn compile_create_simple_table() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column("email", "text")
            })
            .build();

        let summary = create_compilation_summary(&source, &target, "Create users table");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn compile_create_table_with_primary_key() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .primary_key(["id"])
            })
            .build();

        let summary = create_compilation_summary(&source, &target, "Create users table with PK");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn compile_add_nullable_column() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column("email", "text")
            })
            .build();

        let summary = create_compilation_summary(&source, &target, "Add email column");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn compile_add_column_with_default() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("status", "text", |c| c.not_null().default("'active'"))
            })
            .build();

        let summary =
            create_compilation_summary(&source, &target, "Add status column with default");
        assert_yaml_snapshot!(summary);
    }
}

// ============================================================================
// Generated Source Code Tests
// ============================================================================

mod generated_source_code {
    use super::*;

    #[test]
    fn generated_source_contains_macro() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Create users table")).unwrap();

        let source_code = result.source_code();

        // Verify the source code structure
        assert!(source_code.contains("define_migration!"));
        assert!(source_code.contains("use tern_migration_guest::define_migration"));
        assert!(source_code.contains(&format!("id: \"{}\"", result.migration_id())));
        assert!(source_code.contains("description: \"Create users table\""));
        assert!(source_code.contains("source_state_hash:"));
        assert!(source_code.contains("target_state_hash:"));
        assert!(source_code.contains("compiled_at:"));
        assert!(source_code.contains("statements:"));
    }

    #[test]
    fn generated_source_macro_only() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::macro_only("Create users table"),
        )
        .unwrap();

        let source_code = result.source_code();

        // Should start directly with macro, no header
        assert!(source_code.starts_with("define_migration!"));
        assert!(!source_code.contains("use tern_migration_guest"));
    }

    #[test]
    fn generated_source_escapes_special_characters() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::new("Add \"quoted\" field with 'apostrophes' and \\backslash"),
        )
        .unwrap();

        let source_code = result.source_code();

        // Double quotes should be escaped
        assert!(source_code.contains("\\\"quoted\\\""));
        // Backslashes should be escaped
        assert!(source_code.contains("\\\\backslash"));
        // Single quotes don't need escaping in Rust string literals
        assert!(source_code.contains("'apostrophes'"));
    }

    #[test]
    fn snapshot_generated_source_for_complex_migration() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("user_status", ["active", "inactive", "pending"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .column_with("status", "user_status", |c| c.default("'active'"))
                    .column_with("created_at", "timestamptz", |c| {
                        c.not_null().default("now()")
                    })
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::macro_only("Create users table with enum"),
        )
        .unwrap();

        // Snapshot the generated source code (macro-only for cleaner snapshot)
        // Redact the compiled_at timestamp since it's non-deterministic
        let source_code = result.source_code();
        let redacted = redact_compiled_at(&source_code);
        assert_snapshot!(redacted);
    }
}

// ============================================================================
// Migration ID Determinism Tests
// ============================================================================

mod migration_id_determinism {
    use super::*;

    #[test]
    fn same_schemas_produce_same_migration_id() {
        let source1 = NamespaceBuilder::new("public").build();
        let target1 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let source2 = NamespaceBuilder::new("public").build();
        let target2 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result1 =
            compile_migration(&source1, &target1, CompileOptions::new("Create users")).unwrap();
        let result2 =
            compile_migration(&source2, &target2, CompileOptions::new("Create users")).unwrap();

        // Same schemas and same description should produce same migration ID
        assert_eq!(result1.migration_id(), result2.migration_id());
    }

    #[test]
    fn different_descriptions_produce_different_migration_ids() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result1 =
            compile_migration(&source, &target, CompileOptions::new("Description A")).unwrap();
        let result2 =
            compile_migration(&source, &target, CompileOptions::new("Description B")).unwrap();

        // Different descriptions should produce different migration IDs
        assert_ne!(result1.migration_id(), result2.migration_id());
    }

    #[test]
    fn different_source_schemas_produce_different_migration_ids() {
        let source1 = NamespaceBuilder::new("public").build();
        let source2 = NamespaceBuilder::new("public")
            .table("existing", |t| t.column("id", "integer"))
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result1 =
            compile_migration(&source1, &target, CompileOptions::new("Add users")).unwrap();
        let result2 =
            compile_migration(&source2, &target, CompileOptions::new("Add users")).unwrap();

        // Different source schemas (different parent state hash) should produce different IDs
        assert_ne!(result1.migration_id(), result2.migration_id());
    }

    #[test]
    fn migration_id_is_64_hex_characters() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Create users")).unwrap();
        let id = result.migration_id();

        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

// ============================================================================
// State Hash Tests
// ============================================================================

mod state_hash_tests {
    use super::*;

    #[test]
    fn source_and_target_hashes_differ() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Create users")).unwrap();

        // Different schemas should have different state hashes
        assert_ne!(result.source_hash, result.target_hash);
    }

    #[test]
    fn identical_schemas_have_same_hash() {
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let hash1 = StateHash::from_namespace(&ns1);
        let hash2 = StateHash::from_namespace(&ns2);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn state_hash_is_64_hex_characters() {
        let ns = NamespaceBuilder::new("public").build();
        let hash = StateHash::from_namespace(&ns);

        assert_eq!(hash.to_hex().len(), 64);
        assert!(hash.to_hex().chars().all(|c| c.is_ascii_hexdigit()));
    }
}

// ============================================================================
// Breaking Change Detection Tests
// ============================================================================

mod breaking_change_detection {
    use super::*;

    #[test]
    fn dropping_table_is_destructive() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let target = NamespaceBuilder::new("public").build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Drop users table")).unwrap();

        assert!(result.has_breaking_changes());
        assert!(result.has_destructive_changes());

        let summary = create_compilation_summary(&source, &target, "Drop users table");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn dropping_column_is_destructive() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column("email", "text")
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Drop email column")).unwrap();

        assert!(result.has_breaking_changes());
        assert!(result.has_destructive_changes());

        let summary = create_compilation_summary(&source, &target, "Drop email column");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn adding_not_null_constraint_is_backfill() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text") // nullable
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("name", "text", |c| c.not_null()) // now not null
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::new("Make name column not null"),
        )
        .unwrap();

        assert!(result.has_breaking_changes());
        // Making a column NOT NULL requires backfill (not destructive)
        // The mitigation strategy depends on the implementation

        let summary = create_compilation_summary(&source, &target, "Make name column not null");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn adding_nullable_column_is_not_breaking() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("email", "text") // nullable
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Add email column")).unwrap();

        // Adding a nullable column should not be a breaking change
        assert!(!result.has_breaking_changes());
        assert!(!result.has_destructive_changes());
    }

    #[test]
    fn adding_column_with_default_is_not_breaking() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("status", "text", |c| c.not_null().default("'active'"))
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::new("Add status column with default"),
        )
        .unwrap();

        // Adding a NOT NULL column with a default value should not be breaking
        assert!(!result.has_breaking_changes());
    }

    #[test]
    fn adding_unique_constraint_is_ratchet() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .primary_key(["id"])
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .build();

        let result = compile_migration(
            &source,
            &target,
            CompileOptions::new("Add unique constraint on email"),
        )
        .unwrap();

        // Adding a unique constraint is a breaking change (ratchet)
        assert!(result.has_breaking_changes());

        let summary =
            create_compilation_summary(&source, &target, "Add unique constraint on email");
        assert_yaml_snapshot!(summary);
    }
}

// ============================================================================
// Complex Migration Tests
// ============================================================================

mod complex_migrations {
    use super::*;

    #[test]
    fn compile_multi_table_schema() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type(
                "order_status",
                ["pending", "processing", "shipped", "delivered"],
            )
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .column_with("created_at", "timestamptz", |c| {
                        c.not_null().default("now()")
                    })
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .table("orders", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("user_id", "integer", |c| c.not_null())
                    .column_with("status", "order_status", |c| c.default("'pending'"))
                    .column_with("total", "numeric", |c| c.not_null())
                    .column_with("created_at", "timestamptz", |c| c.default("now()"))
                    .primary_key(["id"])
                    .foreign_key_with(
                        ["user_id"],
                        "users",
                        ["id"],
                        ForeignKeyAction::Cascade,
                        ForeignKeyAction::NoAction,
                    )
                    .check_named("orders_total_positive", "total >= 0")
                    .index(["user_id"])
            })
            .build();

        let summary = create_compilation_summary(&source, &target, "Create e-commerce schema");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn compile_adding_foreign_key() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("user_id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("user_id", "integer", |c| c.not_null())
                    .primary_key(["id"])
                    .foreign_key(["user_id"], "users", ["id"])
            })
            .build();

        let summary = create_compilation_summary(&source, &target, "Add FK from posts to users");
        assert_yaml_snapshot!(summary);
    }

    #[test]
    fn compile_schema_with_views_and_sequences() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .sequence("invoice_number_seq")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .column_with("active", "boolean", |c| c.default("true"))
                    .primary_key(["id"])
            })
            .view("active_users", "SELECT * FROM users WHERE active = true")
            .build();

        let summary = create_compilation_summary(&source, &target, "Create users with view");
        assert_yaml_snapshot!(summary);
    }
}

// ============================================================================
// No Changes Error Test
// ============================================================================

mod no_changes_error {
    use super::*;
    use tern::db::compile::CompileError;

    #[test]
    fn compile_fails_with_no_changes() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let result = compile_migration(&source, &target, CompileOptions::new("No changes"));

        assert!(matches!(result, Err(CompileError::NoChanges)));
    }

    #[test]
    fn compile_fails_with_empty_schemas() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public").build();

        let result = compile_migration(&source, &target, CompileOptions::new("Empty migration"));

        assert!(matches!(result, Err(CompileError::NoChanges)));
    }
}

// ============================================================================
// Low-Level API Tests
// ============================================================================

mod low_level_api {
    use super::*;

    #[test]
    fn manual_compilation_with_migration_compiler() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        // Create the diff and plan manually
        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);
        let breaking_changes = analyze_breaking_changes(&diff);

        // Compute state hashes
        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);

        // Create migration record
        let migration = Migration::new(
            "Create users table",
            plan.operations.clone(),
            vec![],
            source_hash,
            target_hash,
            breaking_changes.into_changes(),
        );

        // Compile using the low-level API
        let compiler = MigrationCompiler::new();
        let result = compiler.compile(&migration, &plan).unwrap();

        // Verify the result
        assert_eq!(result.description, "Create users table");
        assert!(result.statement_count() > 0);
        assert!(result.source_code.contains("define_migration!"));
        assert!(result.source_code.contains("CREATE TABLE"));
    }

    #[test]
    fn compiler_config_affects_output() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);
        let source_hash = StateHash::from_namespace(&source);
        let target_hash = StateHash::from_namespace(&target);

        let migration = Migration::new(
            "Create users",
            plan.operations.clone(),
            vec![],
            source_hash,
            target_hash,
            vec![],
        );

        // Full file output
        let full_compiler = MigrationCompiler::new();
        let full_result = full_compiler.compile(&migration, &plan).unwrap();

        // Macro-only output
        let macro_compiler = MigrationCompiler::with_config(CompilerConfig::macro_only());
        let macro_result = macro_compiler.compile(&migration, &plan).unwrap();

        // Full file should have imports
        assert!(full_result.source_code.contains("use tern_migration_guest"));

        // Macro-only should not have imports
        assert!(
            !macro_result
                .source_code
                .contains("use tern_migration_guest")
        );
        assert!(macro_result.source_code.starts_with("define_migration!"));
    }
}

// ============================================================================
// Migration Plan Consistency Tests
// ============================================================================

mod plan_consistency {
    use super::*;

    #[test]
    fn compilation_result_includes_plan() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Create users")).unwrap();

        // Plan should be included
        assert!(!result.plan.is_empty());
        assert_eq!(result.plan.len(), result.statement_count());
    }

    #[test]
    fn statement_count_matches_sql_statements() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("status", ["active", "inactive"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("status", "status")
            })
            .build();

        let result =
            compile_migration(&source, &target, CompileOptions::new("Create schema")).unwrap();

        let statements: Vec<_> = result.statements().collect();
        assert_eq!(statements.len(), result.statement_count());

        // All statements should have SQL content
        for stmt in statements {
            assert!(!stmt.sql.is_empty());
            assert!(!stmt.description.is_empty());
        }
    }
}

// ============================================================================
// Snapshot Test for SQL Extraction
// ============================================================================

mod sql_extraction {
    use super::*;
    use tern::db::compile::compile_and_extract_sql;

    #[test]
    fn extract_sql_returns_all_statements() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .build();

        let (result, sql) =
            compile_and_extract_sql(&source, &target, "Create users table").unwrap();

        assert_eq!(sql.len(), result.statement_count());
        assert!(sql.iter().any(|s| s.contains("CREATE TABLE")));
    }

    #[test]
    fn snapshot_extracted_sql() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .primary_key(["id"])
                    .unique(["email"])
                    .index(["email"])
            })
            .build();

        let sql = extract_sql_statements(&source, &target);
        assert_yaml_snapshot!(sql);
    }
}
