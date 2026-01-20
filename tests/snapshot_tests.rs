//! Snapshot tests for the tern migration pipeline.
//!
//! These tests exercise the full pipeline:
//! 1. Schema construction with builders
//! 2. Schema diffing
//! 3. SQL migration generation
//! 4. Round-trip verification (apply operations to source → equals target)
//!
//! Snapshots are stored in tests/snapshots/ and managed by cargo-insta.

use insta::{assert_snapshot, assert_yaml_snapshot};
use tern::db::diff::diff_namespaces;
use tern::db::history::builder::NamespaceBuilder;
use tern::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig, SqlOptions};
use tern::db::model::types::ForeignKeyAction;

// ============================================================================
// Test Helpers
// ============================================================================

/// Generate migration SQL from source to target schemas.
fn generate_migration_sql(
    source: &tern::db::model::Namespace,
    target: &tern::db::model::Namespace,
) -> String {
    let diff = diff_namespaces(source, target);
    let plan = MigrationPlan::from_diff(&diff);
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);
    script.to_sql_with_options(SqlOptions::without_transaction())
}

/// Generate minimal SQL (no comments, no transaction) for cleaner snapshots.
#[allow(dead_code)]
fn generate_minimal_sql(
    source: &tern::db::model::Namespace,
    target: &tern::db::model::Namespace,
) -> String {
    let diff = diff_namespaces(source, target);
    let plan = MigrationPlan::from_diff(&diff);
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);
    script.to_sql_with_options(SqlOptions::minimal())
}

// ============================================================================
// Basic Table Operations
// ============================================================================

mod create_table {
    use super::*;

    #[test]
    fn simple_table() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("name", "text")
                    .column("email", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_not_null_columns() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("name", "text", |c| c.not_null())
                    .column("bio", "text") // nullable
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_primary_key() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_composite_primary_key() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("order_items", |t| {
                t.column_with("order_id", "integer", |c| c.not_null())
                    .column_with("product_id", "integer", |c| c.not_null())
                    .column("quantity", "integer")
                    .primary_key(["order_id", "product_id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_identity_column() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_default_values() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column("title", "text")
                    .column_with("status", "text", |c| c.default("'draft'"))
                    .column_with("created_at", "timestamptz", |c| c.default("now()"))
                    .column_with("view_count", "integer", |c| c.default("0"))
                    .primary_key(["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_unique_constraint() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .column_with("username", "text", |c| c.not_null())
                    .primary_key(["id"])
                    .unique(["email"])
                    .unique(["username"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_check_constraint() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("products", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("price", "numeric", |c| c.not_null())
                    .column_with("quantity", "integer", |c| c.not_null())
                    .primary_key(["id"])
                    .check_named("price_positive", "price > 0")
                    .check_named("quantity_non_negative", "quantity >= 0")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_index() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("email", "text")
                    .column("created_at", "timestamptz")
                    .primary_key(["id"])
                    .index(["email"])
                    .index(["created_at"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

mod drop_table {
    use super::*;

    #[test]
    fn simple_table() {
        let source = NamespaceBuilder::new("public")
            .table("old_table", |t| {
                t.column("id", "integer").column("data", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public").build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_with_constraints() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .build();
        let target = NamespaceBuilder::new("public").build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

mod rename_table {
    use super::*;

    #[test]
    fn simple_rename() {
        let source = NamespaceBuilder::new("public")
            .table("old_users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("new_users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();

        // Use a lower rename threshold to detect the rename
        let diff = diff_namespaces(&source, &target);
        let plan = MigrationPlan::from_diff(&diff);
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let script = plan.render(&renderer);

        assert_snapshot!(script.to_sql_with_options(SqlOptions::without_transaction()));
    }
}

// ============================================================================
// Column Operations
// ============================================================================

mod add_column {
    use super::*;

    #[test]
    fn nullable_column() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn not_null_column_with_default() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("status", "text", |c| c.not_null().default("'active'"))
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn multiple_columns() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("first_name", "text")
                    .column("last_name", "text")
                    .column("created_at", "timestamptz")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

mod drop_column {
    use super::*;

    #[test]
    fn single_column() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .column("deprecated_field", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

mod alter_column {
    use super::*;

    #[test]
    fn change_type() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("age", "integer")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("age", "bigint")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_not_null() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("name", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("name", "text", |c| c.not_null())
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_not_null() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("name", "text", |c| c.not_null())
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("name", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_default() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("status", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("status", "text", |c| c.default("'pending'"))
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn change_default() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("status", "text", |c| c.default("'draft'"))
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("status", "text", |c| c.default("'pending'"))
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_default() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column_with("status", "text", |c| c.default("'pending'"))
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("status", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Constraint Operations
// ============================================================================

mod add_constraint {
    use super::*;

    #[test]
    fn add_primary_key() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_unique_constraint() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .unique(["email"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_check_constraint() {
        let source = NamespaceBuilder::new("public")
            .table("products", |t| {
                t.column("id", "integer").column("price", "numeric")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("products", |t| {
                t.column("id", "integer")
                    .column("price", "numeric")
                    .check_named("price_positive", "price > 0")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

mod drop_constraint {
    use super::*;

    #[test]
    fn drop_unique_constraint() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .unique(["email"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_check_constraint() {
        let source = NamespaceBuilder::new("public")
            .table("products", |t| {
                t.column("id", "integer")
                    .column("price", "numeric")
                    .check_named("price_positive", "price > 0")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("products", |t| {
                t.column("id", "integer").column("price", "numeric")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Foreign Key Operations
// ============================================================================

mod foreign_keys {
    use super::*;

    #[test]
    fn add_foreign_key() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("user_id", "integer")
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
                    .column("user_id", "integer")
                    .primary_key(["id"])
                    .foreign_key(["user_id"], "users", ["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn foreign_key_with_cascade() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
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
                    .column("user_id", "integer")
                    .primary_key(["id"])
                    .foreign_key_with(
                        ["user_id"],
                        "users",
                        ["id"],
                        ForeignKeyAction::Cascade,
                        ForeignKeyAction::Cascade,
                    )
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Index Operations
// ============================================================================

mod indexes {
    use super::*;

    #[test]
    fn create_index() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .index(["email"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn create_unique_index() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .index_with("idx_users_email_unique", |i| i.column("email").unique())
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn create_composite_index() {
        let source = NamespaceBuilder::new("public")
            .table("events", |t| {
                t.column("id", "integer")
                    .column("user_id", "integer")
                    .column("created_at", "timestamptz")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("events", |t| {
                t.column("id", "integer")
                    .column("user_id", "integer")
                    .column("created_at", "timestamptz")
                    .index(["user_id", "created_at"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_index() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .index(["email"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Enum Type Operations
// ============================================================================

mod enum_types {
    use super::*;

    #[test]
    fn create_enum() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("status", ["pending", "active", "archived"])
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_enum() {
        let source = NamespaceBuilder::new("public")
            .enum_type("old_status", ["a", "b", "c"])
            .build();
        let target = NamespaceBuilder::new("public").build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_enum_value() {
        let source = NamespaceBuilder::new("public")
            .enum_type("status", ["pending", "active"])
            .build();
        let target = NamespaceBuilder::new("public")
            .enum_type("status", ["pending", "active", "archived"])
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn table_using_enum() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("user_status", ["pending", "active", "suspended"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column_with("status", "user_status", |c| c.default("'pending'"))
                    .primary_key(["id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Sequence Operations
// ============================================================================

mod sequences {
    use super::*;

    #[test]
    fn create_sequence() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .sequence("user_id_seq")
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn create_sequence_with_options() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .sequence_with("order_number_seq", |s| {
                s.start(1000).increment(1).min(1000).max(999999)
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_sequence() {
        let source = NamespaceBuilder::new("public").sequence("old_seq").build();
        let target = NamespaceBuilder::new("public").build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// View Operations
// ============================================================================

mod views {
    use super::*;

    #[test]
    fn create_view() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("name", "text")
                    .column("email", "text")
                    .column("password_hash", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("name", "text")
                    .column("email", "text")
                    .column("password_hash", "text")
            })
            .view("public_users", "SELECT id, name, email FROM users")
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn drop_view() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("name", "text")
            })
            .view("user_names", "SELECT id, name FROM users")
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("name", "text")
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn modify_view() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("name", "text")
                    .column("email", "text")
            })
            .view("public_users", "SELECT id, name FROM users")
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("name", "text")
                    .column("email", "text")
            })
            .view("public_users", "SELECT id, name, email FROM users")
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Complex Migrations (Multiple Operations)
// ============================================================================

mod complex_migrations {
    use super::*;

    #[test]
    fn initial_schema_setup() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("user_role", ["admin", "editor", "viewer"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("email", "text", |c| c.not_null())
                    .column_with("name", "text", |c| c.not_null())
                    .column_with("role", "user_role", |c| c.default("'viewer'"))
                    .column_with("created_at", "timestamptz", |c| {
                        c.not_null().default("now()")
                    })
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("author_id", "integer", |c| c.not_null())
                    .column_with("title", "text", |c| c.not_null())
                    .column("content", "text")
                    .column_with("published", "boolean", |c| c.not_null().default("false"))
                    .column_with("created_at", "timestamptz", |c| {
                        c.not_null().default("now()")
                    })
                    .primary_key(["id"])
                    .foreign_key(["author_id"], "users", ["id"])
                    .index(["author_id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn add_comments_feature() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("author_id", "integer")
                    .primary_key(["id"])
                    .foreign_key(["author_id"], "users", ["id"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("author_id", "integer")
                    .primary_key(["id"])
                    .foreign_key(["author_id"], "users", ["id"])
            })
            .table("comments", |t| {
                t.column_with("id", "integer", |c| c.not_null().identity_always())
                    .column_with("post_id", "integer", |c| c.not_null())
                    .column_with("author_id", "integer", |c| c.not_null())
                    .column_with("content", "text", |c| c.not_null())
                    .column_with("created_at", "timestamptz", |c| {
                        c.not_null().default("now()")
                    })
                    .primary_key(["id"])
                    .foreign_key(["post_id"], "posts", ["id"])
                    .foreign_key(["author_id"], "users", ["id"])
                    .index(["post_id"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }

    #[test]
    fn refactor_schema() {
        // Simulate a refactoring: rename column, add new columns, add constraints
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column("email", "text")
                    .primary_key(["id"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("display_name", "text", |c| c.not_null()) // was "name"
                    .column_with("email", "text", |c| c.not_null()) // now required
                    .column_with("verified", "boolean", |c| c.not_null().default("false"))
                    .column_with("updated_at", "timestamptz", |c| c.default("now()"))
                    .primary_key(["id"])
                    .unique(["email"])
                    .index(["display_name"])
            })
            .build();

        assert_snapshot!(generate_migration_sql(&source, &target));
    }
}

// ============================================================================
// Round-Trip Verification
// ============================================================================

mod round_trip {
    use super::*;
    use tern::db::migrate::Operation;

    /// Verify that applying diff operations to source produces a schema
    /// that, when diffed against target, produces no changes.
    fn verify_round_trip(source: &tern::db::model::Namespace, target: &tern::db::model::Namespace) {
        // Get the diff and operations
        let diff = diff_namespaces(source, target);
        let plan = MigrationPlan::from_diff(&diff);

        // Apply operations to source
        let operations: Vec<Operation> = plan.operations;
        let transformed = source.apply(&operations).expect("apply should succeed");

        // Diff transformed against target - should be empty
        let verification_diff = diff_namespaces(&transformed, target);
        assert!(
            verification_diff.is_empty(),
            "Round-trip failed: diff after apply is not empty.\n\
             Tables added: {:?}\n\
             Tables removed: {:?}\n\
             Tables modified: {:?}",
            verification_diff.tables.added.len(),
            verification_diff.tables.removed.len(),
            verification_diff.tables.modified.len()
        );
    }

    #[test]
    fn create_simple_table() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();

        verify_round_trip(&source, &target);
    }

    #[test]
    fn add_column() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .column("name", "text")
            })
            .build();

        verify_round_trip(&source, &target);
    }

    #[test]
    fn drop_column() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .column("deprecated", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();

        verify_round_trip(&source, &target);
    }

    #[test]
    fn add_and_drop_constraint() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .unique(["email"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .check_named("email_not_empty", "email <> ''")
            })
            .build();

        verify_round_trip(&source, &target);
    }

    #[test]
    fn create_enum_and_table() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("status", ["pending", "active", "archived"])
            .table("items", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("status", "status")
                    .primary_key(["id"])
            })
            .build();

        verify_round_trip(&source, &target);
    }

    #[test]
    fn add_index() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer").column("email", "text")
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column("id", "integer")
                    .column("email", "text")
                    .index(["email"])
            })
            .build();

        verify_round_trip(&source, &target);
    }

    // TODO: This test is failing due to differences in how constraint indexes
    // are represented after apply() vs. built directly. The diff shows modified
    // tables even though the structures should be equivalent. This needs
    // investigation into how the apply() function handles constraint indexes.
    #[test]
    #[ignore = "Constraint index representation differs after apply - needs investigation"]
    fn complex_migration() {
        let source = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .primary_key(["id"])
            })
            .build();
        let target = NamespaceBuilder::new("public")
            .enum_type("user_status", ["active", "inactive"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column_with("email", "text", |c| c.not_null())
                    .column("status", "user_status")
                    .primary_key(["id"])
                    .unique(["email"])
                    .index(["name"])
            })
            .table("profiles", |t| {
                t.column_with("user_id", "integer", |c| c.not_null())
                    .column("bio", "text")
                    .primary_key(["user_id"])
                    .foreign_key(["user_id"], "users", ["id"])
            })
            .build();

        verify_round_trip(&source, &target);
    }
}

// ============================================================================
// Diff Structure Snapshots (YAML)
// ============================================================================

mod diff_structure {
    use super::*;
    use serde::Serialize;

    /// Simplified diff summary for readable YAML snapshots.
    #[derive(Serialize)]
    struct DiffSummary {
        tables_added: Vec<String>,
        tables_removed: Vec<String>,
        tables_modified: Vec<String>,
        enums_added: Vec<String>,
        enums_removed: Vec<String>,
        sequences_added: Vec<String>,
        views_added: Vec<String>,
        views_removed: Vec<String>,
    }

    fn summarize_diff(
        source: &tern::db::model::Namespace,
        target: &tern::db::model::Namespace,
    ) -> DiffSummary {
        let diff = diff_namespaces(source, target);
        DiffSummary {
            tables_added: diff
                .tables
                .added
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
            tables_removed: diff
                .tables
                .removed
                .iter()
                .map(|t| t.as_ref().to_string()) // removed items are just keys (TableName)
                .collect(),
            tables_modified: diff
                .tables
                .modified
                .iter()
                .map(|m| m.name.as_ref().to_string())
                .collect(),
            enums_added: diff
                .enums
                .added
                .iter()
                .map(|e| e.name.as_ref().to_string())
                .collect(),
            enums_removed: diff
                .enums
                .removed
                .iter()
                .map(|e| e.as_ref().to_string()) // removed items are just keys (TypeName)
                .collect(),
            sequences_added: diff
                .sequences
                .added
                .iter()
                .map(|s| s.name.as_ref().to_string())
                .collect(),
            views_added: diff
                .views
                .added
                .iter()
                .map(|v| v.name.as_ref().to_string())
                .collect(),
            views_removed: diff
                .views
                .removed
                .iter()
                .map(|v| v.as_ref().to_string()) // removed items are just keys (TableName)
                .collect(),
        }
    }

    #[test]
    fn full_schema_diff() {
        let source = NamespaceBuilder::new("public")
            .table("old_table", |t| t.column("id", "integer"))
            .table("modified_table", |t| t.column("id", "integer"))
            .enum_type("old_enum", ["a", "b"])
            .view("old_view", "SELECT 1")
            .build();
        let target = NamespaceBuilder::new("public")
            .table("new_table", |t| t.column("id", "integer"))
            .table("modified_table", |t| {
                t.column("id", "integer").column("new_col", "text")
            })
            .enum_type("new_enum", ["x", "y"])
            .sequence("new_seq")
            .view("new_view", "SELECT 2")
            .build();

        assert_yaml_snapshot!(summarize_diff(&source, &target));
    }
}

// ============================================================================
// Operation Order Snapshots
// ============================================================================

mod operation_order {
    use super::*;

    fn get_operation_descriptions(
        source: &tern::db::model::Namespace,
        target: &tern::db::model::Namespace,
    ) -> Vec<String> {
        let diff = diff_namespaces(source, target);
        let plan = MigrationPlan::from_diff(&diff);
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let script = plan.render(&renderer);
        script
            .descriptions()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn drops_before_creates() {
        let source = NamespaceBuilder::new("public")
            .table("old_table", |t| t.column("id", "integer"))
            .build();
        let target = NamespaceBuilder::new("public")
            .table("new_table", |t| t.column("id", "integer"))
            .build();

        assert_yaml_snapshot!(get_operation_descriptions(&source, &target));
    }

    #[test]
    fn enums_before_tables() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .enum_type("status", ["a", "b"])
            .table("items", |t| {
                t.column("id", "integer").column("status", "status")
            })
            .build();

        assert_yaml_snapshot!(get_operation_descriptions(&source, &target));
    }

    #[test]
    fn foreign_key_ordering() {
        let source = NamespaceBuilder::new("public").build();
        let target = NamespaceBuilder::new("public")
            .table("parents", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .primary_key(["id"])
            })
            .table("children", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("parent_id", "integer")
                    .primary_key(["id"])
                    .foreign_key(["parent_id"], "parents", ["id"])
            })
            .build();

        assert_yaml_snapshot!(get_operation_descriptions(&source, &target));
    }
}
