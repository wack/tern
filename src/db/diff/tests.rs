//! Tests for schema comparison.

use crate::db::diff::{DiffConfig, diff_namespaces, diff_namespaces_with_config};
use crate::db::model::Column;
use crate::db::model::constraint::{
    CheckConstraint, Constraint, ConstraintKind, ForeignKeyConstraint, PrimaryKeyConstraint,
    UniqueConstraint,
};
use crate::db::model::index::{Index, IndexColumn, NullsOrder, SortOrder};
use crate::db::model::namespace::{EnumType, Namespace, Sequence, View};
use crate::db::model::table::{Table, TableKind};
use crate::db::model::types::{
    ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedTableName, SqlExpr, TypeInfo,
};
use crate::db::schema::{
    CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName, TableName,
    TypeName,
};

// =============================================================================
// Test Fixtures
// =============================================================================

fn default_namespace() -> Namespace {
    Namespace {
        oid: Oid::new(1),
        name: SchemaName::try_new("public".to_string()).unwrap(),
        tables: vec![],
        views: vec![],
        sequences: vec![],
        enums: vec![],
        comment: None,
    }
}

fn default_collation() -> QualifiedCollationName {
    QualifiedCollationName::new(
        SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        CollationName::try_new("default".to_string()).unwrap(),
    )
}

fn make_type_info(type_name: &str) -> TypeInfo {
    TypeInfo {
        name: TypeName::try_new(type_name.to_string()).unwrap(),
        schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        formatted: type_name.to_string(),
        is_array: false,
    }
}

fn make_column(name: &str, type_name: &str) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position: 1,
        type_info: make_type_info(type_name),
        is_nullable: true,
        default: None,
        generated: None,
        identity: None,
        collation: default_collation(),
        comment: None,
    }
}

fn make_column_at_position(name: &str, type_name: &str, position: i16) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position,
        type_info: make_type_info(type_name),
        is_nullable: true,
        default: None,
        generated: None,
        identity: None,
        collation: default_collation(),
        comment: None,
    }
}

fn make_column_nullable(name: &str, type_name: &str, is_nullable: bool) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position: 1,
        type_info: make_type_info(type_name),
        is_nullable,
        default: None,
        generated: None,
        identity: None,
        collation: default_collation(),
        comment: None,
    }
}

fn make_column_with_default(name: &str, type_name: &str, default: &str) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position: 1,
        type_info: make_type_info(type_name),
        is_nullable: true,
        default: Some(SqlExpr::new(default.to_string())),
        generated: None,
        identity: None,
        collation: default_collation(),
        comment: None,
    }
}

fn make_table(name: &str, columns: Vec<Column>) -> Table {
    Table {
        oid: Oid::new(1000),
        name: TableName::try_new(name.to_string()).unwrap(),
        kind: TableKind::Regular,
        columns,
        constraints: vec![],
        indexes: vec![],
        comment: None,
    }
}

fn make_primary_key(name: &str, columns: Vec<&str>) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns: columns
                .into_iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(format!("{name}_idx")).unwrap(),
        }),
        comment: None,
    }
}

fn make_foreign_key(
    name: &str,
    columns: Vec<&str>,
    ref_schema: &str,
    ref_table: &str,
    ref_columns: Vec<&str>,
) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
            columns: columns
                .into_iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            referenced_table: QualifiedTableName::new(
                SchemaName::try_new(ref_schema.to_string()).unwrap(),
                TableName::try_new(ref_table.to_string()).unwrap(),
            ),
            referenced_columns: ref_columns
                .into_iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            on_delete: ForeignKeyAction::NoAction,
            on_update: ForeignKeyAction::NoAction,
            is_deferrable: false,
            is_initially_deferred: false,
        }),
        comment: None,
    }
}

fn make_unique_constraint(name: &str, columns: Vec<&str>) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::Unique(UniqueConstraint {
            columns: columns
                .into_iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(format!("{name}_idx")).unwrap(),
            nulls_not_distinct: false,
        }),
        comment: None,
    }
}

fn make_check_constraint(name: &str, expression: &str) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::Check(CheckConstraint {
            expression: SqlExpr::new(expression.to_string()),
            is_no_inherit: false,
        }),
        comment: None,
    }
}

fn make_index(name: &str, columns: Vec<&str>, is_unique: bool) -> Index {
    Index {
        oid: Oid::new(2000),
        name: IndexName::try_new(name.to_string()).unwrap(),
        method: IndexMethod::BTree,
        is_unique,
        is_constraint_index: false,
        columns: columns
            .into_iter()
            .map(|c| IndexColumn {
                column: Some(ColumnName::try_new(c.to_string()).unwrap()),
                expression: None,
                order: SortOrder::Ascending,
                nulls: NullsOrder::Last,
            })
            .collect(),
        predicate: None,
        comment: None,
    }
}

fn make_view(name: &str, definition: &str) -> View {
    View {
        oid: Oid::new(3000),
        name: TableName::try_new(name.to_string()).unwrap(),
        definition: SqlExpr::new(definition.to_string()),
        is_materialized: false,
        comment: None,
    }
}

fn make_sequence(name: &str, start: i64, increment: i64) -> Sequence {
    Sequence {
        oid: Oid::new(4000),
        name: SequenceName::try_new(name.to_string()).unwrap(),
        data_type: make_type_info("int8"),
        start_value: start,
        increment,
        min_value: 1,
        max_value: i64::MAX,
        cache_size: 1,
        is_cyclic: false,
        comment: None,
    }
}

fn make_enum(name: &str, values: Vec<&str>) -> EnumType {
    EnumType {
        oid: Oid::new(5000),
        name: TypeName::try_new(name.to_string()).unwrap(),
        values: values.into_iter().map(String::from).collect(),
        comment: None,
    }
}

// =============================================================================
// Basic Diff Tests
// =============================================================================

mod basic_diff_tests {
    use super::*;

    #[test]
    fn empty_schemas_produce_empty_diff() {
        let source = default_namespace();
        let target = default_namespace();

        let diff = diff_namespaces(&source, &target);

        assert!(diff.tables.added.is_empty());
        assert!(diff.tables.removed.is_empty());
        assert!(diff.tables.modified.is_empty());
        assert!(diff.tables.potential_renames.is_empty());
        assert!(diff.is_empty());
    }

    #[test]
    fn identical_schemas_produce_empty_diff() {
        let users_table = make_table(
            "users",
            vec![make_column("id", "integer"), make_column("name", "text")],
        );

        let source = Namespace {
            tables: vec![users_table.clone()],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![users_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert!(diff.tables.added.is_empty());
        assert!(diff.tables.removed.is_empty());
        assert!(diff.tables.modified.is_empty());
        assert!(diff.is_empty());
    }

    #[test]
    fn detects_added_table() {
        let source = default_namespace();
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.tables.added.len(), 1);
        assert_eq!(diff.tables.added[0].name.as_ref(), "users");
        assert!(diff.tables.removed.is_empty());
    }

    #[test]
    fn detects_removed_table() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = default_namespace();

        let diff = diff_namespaces(&source, &target);

        assert!(diff.tables.added.is_empty());
        assert_eq!(diff.tables.removed.len(), 1);
        assert_eq!(diff.tables.removed[0].as_ref(), "users");
    }

    #[test]
    fn detects_multiple_changes() {
        let source = Namespace {
            tables: vec![
                make_table("users", vec![make_column("id", "integer")]),
                make_table("orders", vec![make_column("id", "integer")]),
            ],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![
                make_table("users", vec![make_column("id", "integer")]),
                make_table("products", vec![make_column("id", "integer")]),
            ],
            ..default_namespace()
        };

        // Use high threshold to disable rename detection for this test
        let config = DiffConfig::no_rename_detection();
        let diff = diff_namespaces_with_config(&source, &target, &config);

        assert_eq!(diff.tables.added.len(), 1);
        assert_eq!(diff.tables.added[0].name.as_ref(), "products");
        assert_eq!(diff.tables.removed.len(), 1);
        assert_eq!(diff.tables.removed[0].as_ref(), "orders");
        assert!(diff.tables.modified.is_empty());
    }
}

// =============================================================================
// Column Diff Tests
// =============================================================================

mod column_diff_tests {
    use super::*;

    #[test]
    fn detects_added_column() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("email", "text")],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert!(diff.tables.added.is_empty());
        assert!(diff.tables.removed.is_empty());
        assert_eq!(diff.tables.modified.len(), 1);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.name.as_ref(), "users");
        assert_eq!(table_diff.columns.added.len(), 1);
        assert_eq!(table_diff.columns.added[0].name.as_ref(), "email");
    }

    #[test]
    fn detects_removed_column() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column("id", "integer"),
                    make_column("deprecated_field", "text"),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.columns.removed.len(), 1);
        assert_eq!(table_diff.columns.removed[0].as_ref(), "deprecated_field");
    }

    #[test]
    fn detects_column_type_change() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "bigint")])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.columns.modified.len(), 1);

        let col_diff = &table_diff.columns.modified[0];
        assert_eq!(col_diff.name.as_ref(), "id");
        assert!(col_diff.type_info.is_some());

        let type_change = col_diff.type_info.as_ref().unwrap();
        assert_eq!(type_change.source.formatted, "integer");
        assert_eq!(type_change.target.formatted, "bigint");
    }

    #[test]
    fn detects_nullability_change() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", true)],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", false)],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let col_diff = &diff.tables.modified[0].columns.modified[0];
        assert!(col_diff.is_nullable.is_some());

        let nullable_change = col_diff.is_nullable.as_ref().unwrap();
        assert!(nullable_change.source);
        assert!(!nullable_change.target);
    }

    #[test]
    fn detects_default_added() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("status", "text")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_with_default("status", "text", "'active'")],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let col_diff = &diff.tables.modified[0].columns.modified[0];
        assert!(col_diff.default.is_some());

        let default_change = col_diff.default.as_ref().unwrap();
        assert!(default_change.source.is_none());
        assert_eq!(default_change.target.as_ref().unwrap().as_ref(), "'active'");
    }
}

// =============================================================================
// Constraint Diff Tests
// =============================================================================

mod constraint_diff_tests {
    use super::*;

    #[test]
    fn detects_added_constraint() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };

        let mut target_table = make_table("users", vec![make_column("id", "integer")]);
        target_table
            .constraints
            .push(make_primary_key("users_pkey", vec!["id"]));

        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.constraints.added.len(), 1);
        assert_eq!(table_diff.constraints.added[0].name.as_ref(), "users_pkey");
    }

    #[test]
    fn detects_removed_foreign_key() {
        let mut source_table = make_table(
            "orders",
            vec![
                make_column("id", "integer"),
                make_column("user_id", "integer"),
            ],
        );
        source_table.constraints.push(make_foreign_key(
            "orders_user_id_fkey",
            vec!["user_id"],
            "public",
            "users",
            vec!["id"],
        ));

        let target_table = make_table(
            "orders",
            vec![
                make_column("id", "integer"),
                make_column("user_id", "integer"),
            ],
        );

        let source = Namespace {
            tables: vec![source_table],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.constraints.removed.len(), 1);
        assert_eq!(
            table_diff.constraints.removed[0].as_ref(),
            "orders_user_id_fkey"
        );
    }

    #[test]
    fn detects_modified_check_constraint() {
        let mut source_table = make_table("users", vec![make_column("age", "integer")]);
        source_table
            .constraints
            .push(make_check_constraint("users_age_check", "age >= 0"));

        let mut target_table = make_table("users", vec![make_column("age", "integer")]);
        target_table
            .constraints
            .push(make_check_constraint("users_age_check", "age >= 18"));

        let source = Namespace {
            tables: vec![source_table],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.constraints.modified.len(), 1);
        assert_eq!(
            table_diff.constraints.modified[0].name.as_ref(),
            "users_age_check"
        );
    }
}

// =============================================================================
// Index Diff Tests
// =============================================================================

mod index_diff_tests {
    use super::*;

    #[test]
    fn detects_added_index() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("email", "text")])],
            ..default_namespace()
        };

        let mut target_table = make_table("users", vec![make_column("email", "text")]);
        target_table
            .indexes
            .push(make_index("users_email_idx", vec!["email"], false));

        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.indexes.added.len(), 1);
        assert_eq!(table_diff.indexes.added[0].name.as_ref(), "users_email_idx");
    }

    #[test]
    fn detects_removed_index() {
        let mut source_table = make_table("users", vec![make_column("email", "text")]);
        source_table
            .indexes
            .push(make_index("users_email_idx", vec!["email"], false));

        let target_table = make_table("users", vec![make_column("email", "text")]);

        let source = Namespace {
            tables: vec![source_table],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert_eq!(table_diff.indexes.removed.len(), 1);
        assert_eq!(table_diff.indexes.removed[0].as_ref(), "users_email_idx");
    }
}

// =============================================================================
// View Diff Tests
// =============================================================================

mod view_diff_tests {
    use super::*;

    #[test]
    fn detects_view_definition_change() {
        let source = Namespace {
            views: vec![make_view(
                "active_users",
                "SELECT * FROM users WHERE active",
            )],
            ..default_namespace()
        };
        let target = Namespace {
            views: vec![make_view(
                "active_users",
                "SELECT * FROM users WHERE active = true",
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.views.modified.len(), 1);
        assert!(diff.views.modified[0].definition.is_some());
    }

    #[test]
    fn detects_added_view() {
        let source = default_namespace();
        let target = Namespace {
            views: vec![make_view(
                "active_users",
                "SELECT * FROM users WHERE active",
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.views.added.len(), 1);
        assert_eq!(diff.views.added[0].name.as_ref(), "active_users");
    }
}

// =============================================================================
// Enum Diff Tests
// =============================================================================

mod enum_diff_tests {
    use super::*;

    #[test]
    fn detects_enum_value_added() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active", "archived"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.enums.modified.len(), 1);
        let enum_diff = &diff.enums.modified[0];
        assert_eq!(enum_diff.values_added, vec!["archived"]);
        assert!(enum_diff.values_removed.is_empty());
    }

    #[test]
    fn detects_enum_value_removed() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active", "deprecated"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let enum_diff = &diff.enums.modified[0];
        assert_eq!(enum_diff.values_removed, vec!["deprecated"]);
    }

    #[test]
    fn detects_enum_value_reorder() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["a", "b", "c"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["a", "c", "b"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let enum_diff = &diff.enums.modified[0];
        assert!(enum_diff.values_reordered);
    }
}

// =============================================================================
// Sequence Diff Tests
// =============================================================================

mod sequence_diff_tests {
    use super::*;

    #[test]
    fn detects_sequence_increment_change() {
        let source = Namespace {
            sequences: vec![make_sequence("users_id_seq", 1, 1)],
            ..default_namespace()
        };
        let target = Namespace {
            sequences: vec![make_sequence("users_id_seq", 1, 10)],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.sequences.modified.len(), 1);
        let seq_diff = &diff.sequences.modified[0];
        assert!(seq_diff.increment.is_some());
        assert_eq!(seq_diff.increment.as_ref().unwrap().source, 1);
        assert_eq!(seq_diff.increment.as_ref().unwrap().target, 10);
    }

    #[test]
    fn detects_added_sequence() {
        let source = default_namespace();
        let target = Namespace {
            sequences: vec![make_sequence("users_id_seq", 1, 1)],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.sequences.added.len(), 1);
        assert_eq!(diff.sequences.added[0].name.as_ref(), "users_id_seq");
    }
}

// =============================================================================
// Rename Detection Tests
// =============================================================================

mod rename_detection_tests {
    use super::*;

    #[test]
    fn detects_table_rename_identical_structure() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column("id", "integer"),
                    make_column("email", "text"),
                    make_column("created_at", "timestamp"),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "accounts",
                vec![
                    make_column("id", "integer"),
                    make_column("email", "text"),
                    make_column("created_at", "timestamp"),
                ],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert!(diff.tables.added.is_empty());
        assert!(diff.tables.removed.is_empty());
        assert_eq!(diff.tables.potential_renames.len(), 1);

        let rename = &diff.tables.potential_renames[0];
        assert_eq!(rename.source_key.as_ref(), "users");
        assert_eq!(rename.target.name.as_ref(), "accounts");
        assert!(rename.similarity >= 0.9);
    }

    #[test]
    fn detects_table_rename_with_minor_changes() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("email", "text")],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "accounts",
                vec![
                    make_column("id", "integer"),
                    make_column("email", "text"),
                    make_column("phone", "text"),
                ],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.tables.potential_renames.len(), 1);
        let rename = &diff.tables.potential_renames[0];
        assert!(rename.similarity >= 0.5);
        assert!(rename.similarity < 1.0);
    }

    #[test]
    fn no_rename_for_completely_different_tables() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("email", "text")],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "products",
                vec![
                    make_column("sku", "text"),
                    make_column("price", "numeric"),
                    make_column("inventory", "integer"),
                ],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.tables.added.len(), 1);
        assert_eq!(diff.tables.removed.len(), 1);
        assert!(diff.tables.potential_renames.is_empty());
    }

    #[test]
    fn rename_detection_respects_threshold() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column("id", "integer"),
                    make_column("a", "text"),
                    make_column("b", "text"),
                    make_column("c", "text"),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "accounts",
                vec![
                    make_column("id", "integer"),
                    make_column("x", "text"),
                    make_column("y", "text"),
                    make_column("z", "text"),
                ],
            )],
            ..default_namespace()
        };

        // With high threshold: should be add/remove
        let config_high = DiffConfig::with_rename_threshold(0.8);
        let diff_high = diff_namespaces_with_config(&source, &target, &config_high);
        assert!(diff_high.tables.potential_renames.is_empty());
        assert_eq!(diff_high.tables.added.len(), 1);
        assert_eq!(diff_high.tables.removed.len(), 1);

        // With low threshold: detected as potential rename
        let config_low = DiffConfig::with_rename_threshold(0.2);
        let diff_low = diff_namespaces_with_config(&source, &target, &config_low);
        assert_eq!(diff_low.tables.potential_renames.len(), 1);
    }

    #[test]
    fn detects_column_rename_same_type() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column_at_position("id", "integer", 1),
                    make_column_at_position("email", "text", 2),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column_at_position("id", "integer", 1),
                    make_column_at_position("email_address", "text", 2),
                ],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        assert!(table_diff.columns.added.is_empty());
        assert!(table_diff.columns.removed.is_empty());
        assert_eq!(table_diff.columns.potential_renames.len(), 1);

        let rename = &table_diff.columns.potential_renames[0];
        assert_eq!(rename.source_key.as_ref(), "email");
        assert_eq!(rename.target.name.as_ref(), "email_address");
    }

    #[test]
    fn no_column_rename_different_types() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("code", "integer")],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column("id", "integer"),
                    make_column("code_name", "text"),
                ],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        let table_diff = &diff.tables.modified[0];
        // Different types = low similarity, should be add/remove
        assert_eq!(table_diff.columns.added.len(), 1);
        assert_eq!(table_diff.columns.removed.len(), 1);
        assert!(table_diff.columns.potential_renames.is_empty());
    }

    #[test]
    fn empty_table_not_renamed_to_populated_table() {
        let source = Namespace {
            tables: vec![make_table("empty", vec![])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "full",
                vec![make_column("id", "integer"), make_column("data", "text")],
            )],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.tables.added.len(), 1);
        assert_eq!(diff.tables.removed.len(), 1);
        assert!(diff.tables.potential_renames.is_empty());
    }

    #[test]
    fn constraint_similarity_contributes_to_table_match() {
        let mut source_table = make_table("users", vec![make_column("id", "integer")]);
        source_table
            .constraints
            .push(make_primary_key("users_pkey", vec!["id"]));
        source_table
            .constraints
            .push(make_unique_constraint("users_id_unique", vec!["id"]));

        let mut target_table = make_table("accounts", vec![make_column("id", "integer")]);
        target_table
            .constraints
            .push(make_primary_key("accounts_pkey", vec!["id"]));
        target_table
            .constraints
            .push(make_unique_constraint("accounts_id_unique", vec!["id"]));

        let source = Namespace {
            tables: vec![source_table],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);

        assert_eq!(diff.tables.potential_renames.len(), 1);
        assert!(diff.tables.potential_renames[0].similarity >= 0.7);
    }
}

// =============================================================================
// Breaking Change Detection Tests
// =============================================================================

mod breaking_change_tests {
    use super::*;
    use crate::db::diff::breaking::{
        BreakingChangeKind, MitigationStrategy, analyze_breaking_changes,
    };

    #[test]
    fn empty_diff_is_safe() {
        let source = default_namespace();
        let target = default_namespace();
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(analysis.is_safe());
        assert_eq!(analysis.len(), 0);
    }

    #[test]
    fn adding_table_is_safe() {
        let source = default_namespace();
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(analysis.is_safe());
    }

    #[test]
    fn adding_nullable_column_is_safe() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("email", "text")],
            )],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(analysis.is_safe());
    }

    #[test]
    fn dropping_table_is_breaking() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = default_namespace();
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        assert_eq!(analysis.len(), 1);

        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::TableDropped { table } if table.as_ref() == "users")
        );
    }

    #[test]
    fn renaming_table_is_breaking() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column("id", "integer"),
                    make_column("email", "text"),
                    make_column("name", "text"),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "accounts",
                vec![
                    make_column("id", "integer"),
                    make_column("email", "text"),
                    make_column("name", "text"),
                ],
            )],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::DualWrite);
        assert!(
            matches!(&change.kind, BreakingChangeKind::TableRenamed { from, to, .. }
            if from.as_ref() == "users" && to.as_ref() == "accounts")
        );
    }

    #[test]
    fn dropping_column_is_breaking() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column("id", "integer"), make_column("email", "text")],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::ColumnDropped { table, column }
            if table.as_ref() == "users" && column.as_ref() == "email")
        );
    }

    #[test]
    fn renaming_column_is_breaking() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column_at_position("id", "integer", 1),
                    make_column_at_position("email", "text", 2),
                ],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![
                    make_column_at_position("id", "integer", 1),
                    make_column_at_position("email_address", "text", 2),
                ],
            )],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::DualWrite);
        assert!(
            matches!(&change.kind, BreakingChangeKind::ColumnRenamed { table, from, to, .. }
            if table.as_ref() == "users" && from.as_ref() == "email" && to.as_ref() == "email_address")
        );
    }

    #[test]
    fn making_column_non_nullable_is_breaking() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", true)],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", false)],
            )],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Backfill);
        assert!(
            matches!(&change.kind, BreakingChangeKind::ColumnMadeNonNullable { table, column }
            if table.as_ref() == "users" && column.as_ref() == "email")
        );
    }

    #[test]
    fn making_column_nullable_is_safe() {
        let source = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", false)],
            )],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table(
                "users",
                vec![make_column_nullable("email", "text", true)],
            )],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        // Making nullable is safe (no breaking changes)
        assert!(analysis.is_safe());
    }

    #[test]
    fn narrowing_column_type_is_breaking() {
        // bigint -> integer is narrowing
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "bigint")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::DualWrite);
        assert!(matches!(
            &change.kind,
            BreakingChangeKind::ColumnTypeChanged { .. }
        ));
    }

    #[test]
    fn widening_column_type_is_safe() {
        // integer -> bigint is widening
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "integer")])],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![make_column("id", "bigint")])],
            ..default_namespace()
        };
        let diff = diff_namespaces(&source, &target);

        let analysis = analyze_breaking_changes(&diff);

        // Widening is safe
        assert!(analysis.is_safe());
    }

    #[test]
    fn adding_constraint_is_breaking() {
        let source = Namespace {
            tables: vec![make_table("users", vec![make_column("email", "text")])],
            ..default_namespace()
        };

        let mut target_table = make_table("users", vec![make_column("email", "text")]);
        target_table
            .constraints
            .push(make_unique_constraint("users_email_unique", vec!["email"]));

        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        // Adding a constraint is breaking - requires Ratchet mitigation (NOT VALID pattern)
        assert!(!analysis.is_safe());
        assert_eq!(analysis.len(), 1);

        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Ratchet);
        assert!(matches!(
            &change.kind,
            BreakingChangeKind::UniqueConstraintAdded { .. }
        ));
    }

    #[test]
    fn dropping_constraint_is_safe() {
        let mut source_table = make_table("users", vec![make_column("email", "text")]);
        source_table
            .constraints
            .push(make_unique_constraint("users_email_unique", vec!["email"]));

        let target_table = make_table("users", vec![make_column("email", "text")]);

        let source = Namespace {
            tables: vec![source_table],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![target_table],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        // Dropping constraints is safe
        assert!(analysis.is_safe());
    }

    #[test]
    fn removing_enum_value_is_breaking() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active", "archived"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::EnumValueRemoved { enum_type, values }
            if enum_type.as_ref() == "status" && values == &vec!["archived".to_string()])
        );
    }

    #[test]
    fn adding_enum_value_is_safe() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["pending", "active", "archived"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        // Adding enum values is safe
        assert!(analysis.is_safe());
    }

    #[test]
    fn reordering_enum_values_is_breaking() {
        let source = Namespace {
            enums: vec![make_enum("status", vec!["a", "b", "c"])],
            ..default_namespace()
        };
        let target = Namespace {
            enums: vec![make_enum("status", vec!["a", "c", "b"])],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::EnumValuesReordered { enum_type }
            if enum_type.as_ref() == "status")
        );
    }

    #[test]
    fn dropping_view_is_breaking() {
        let source = Namespace {
            views: vec![make_view(
                "active_users",
                "SELECT * FROM users WHERE active",
            )],
            ..default_namespace()
        };
        let target = default_namespace();

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::ViewDropped { view }
            if view.as_ref() == "active_users")
        );
    }

    #[test]
    fn dropping_sequence_is_breaking() {
        let source = Namespace {
            sequences: vec![make_sequence("users_id_seq", 1, 1)],
            ..default_namespace()
        };
        let target = default_namespace();

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::Destructive);
        assert!(
            matches!(&change.kind, BreakingChangeKind::SequenceDropped { sequence }
            if sequence.as_ref() == "users_id_seq")
        );
    }

    #[test]
    fn multiple_breaking_changes_all_detected() {
        let source = Namespace {
            tables: vec![
                make_table("users", vec![make_column("id", "integer")]),
                make_table("orders", vec![make_column("id", "integer")]),
            ],
            views: vec![make_view("user_view", "SELECT * FROM users")],
            ..default_namespace()
        };
        let target = Namespace {
            tables: vec![make_table("users", vec![])], // dropped column
            // dropped orders table
            // dropped view
            ..default_namespace()
        };

        let config = DiffConfig::no_rename_detection();
        let diff = diff_namespaces_with_config(&source, &target, &config);
        let analysis = analyze_breaking_changes(&diff);

        // Should detect: dropped table, dropped column, dropped view
        assert!(!analysis.is_safe());
        assert_eq!(analysis.len(), 3);
        // All drops are Destructive
        assert_eq!(
            analysis.count_by_mitigation(MitigationStrategy::Destructive),
            3
        );
    }

    #[test]
    fn view_materialization_change_is_breaking() {
        let mut source_view = make_view("cached_data", "SELECT * FROM data");
        source_view.is_materialized = false;

        let mut target_view = make_view("cached_data", "SELECT * FROM data");
        target_view.is_materialized = true;

        let source = Namespace {
            views: vec![source_view],
            ..default_namespace()
        };
        let target = Namespace {
            views: vec![target_view],
            ..default_namespace()
        };

        let diff = diff_namespaces(&source, &target);
        let analysis = analyze_breaking_changes(&diff);

        assert!(!analysis.is_safe());
        let change = analysis.iter().next().unwrap();
        assert_eq!(change.mitigation, MitigationStrategy::DualWrite);
        assert!(
            matches!(&change.kind, BreakingChangeKind::MaterializationChanged { view, became_materialized }
            if view.as_ref() == "cached_data" && *became_materialized)
        );
    }
}
