//! Snapshot tests for Rust SeaORM code generation.
//!
//! These tests use insta to capture and verify generated code output.

use tern_ddl::types::QualifiedCollationName;
use tern_ddl::{
    CollationName, Column, ColumnName, Constraint, ConstraintKind, ConstraintName,
    ForeignKeyAction, ForeignKeyConstraint, IdentityKind, IndexName, Oid, PrimaryKeyConstraint,
    QualifiedTableName, SchemaName, Table, TableKind, TableName, TypeInfo, TypeName,
};

use crate::Codegen;
use crate::rust::{EntityFormat, RustSeaOrmCodegen, RustSeaOrmCodegenConfig};

fn make_column(name: &str, type_name: &str, is_nullable: bool) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position: 1,
        type_info: TypeInfo {
            name: TypeName::try_new(type_name.to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: type_name.to_string(),
            is_array: false,
        },
        is_nullable,
        default: None,
        generated: None,
        identity: None,
        collation: QualifiedCollationName::new(
            SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            CollationName::try_new("default".to_string()).unwrap(),
        ),
        comment: None,
    }
}

fn make_column_with_identity(name: &str, type_name: &str, identity: IdentityKind) -> Column {
    let mut col = make_column(name, type_name, false);
    col.identity = Some(identity);
    col
}

fn make_table_with_pk(name: &str, columns: Vec<Column>, pk_columns: &[&str]) -> Table {
    let pk = Constraint {
        name: ConstraintName::try_new(format!("{name}_pkey")).unwrap(),
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns: pk_columns
                .iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(format!("{name}_pkey")).unwrap(),
        }),
        comment: None,
    };

    Table {
        oid: Oid::new(1),
        name: TableName::try_new(name.to_string()).unwrap(),
        kind: TableKind::Regular,
        columns,
        constraints: vec![pk],
        indexes: vec![],
        comment: None,
    }
}

fn make_table_with_fk(
    name: &str,
    columns: Vec<Column>,
    pk_columns: &[&str],
    fk_column: &str,
    fk_target_table: &str,
    fk_target_column: &str,
) -> Table {
    let pk = Constraint {
        name: ConstraintName::try_new(format!("{name}_pkey")).unwrap(),
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns: pk_columns
                .iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(format!("{name}_pkey")).unwrap(),
        }),
        comment: None,
    };

    let fk = Constraint {
        name: ConstraintName::try_new(format!("{name}_{fk_column}_fkey")).unwrap(),
        kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
            columns: vec![ColumnName::try_new(fk_column.to_string()).unwrap()],
            referenced_table: QualifiedTableName::new(
                SchemaName::try_new("public".to_string()).unwrap(),
                TableName::try_new(fk_target_table.to_string()).unwrap(),
            ),
            referenced_columns: vec![ColumnName::try_new(fk_target_column.to_string()).unwrap()],
            on_delete: ForeignKeyAction::Cascade,
            on_update: ForeignKeyAction::NoAction,
            is_deferrable: false,
            is_initially_deferred: false,
        }),
        comment: None,
    };

    Table {
        oid: Oid::new(1),
        name: TableName::try_new(name.to_string()).unwrap(),
        kind: TableKind::Regular,
        columns,
        constraints: vec![pk, fk],
        indexes: vec![],
        comment: None,
    }
}

#[test]
fn snapshot_simple_entity_compact() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
        make_column("email", "text", true),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("user.rs").unwrap());
}

#[test]
fn snapshot_simple_entity_expanded() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
        make_column("email", "text", true),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let config = RustSeaOrmCodegenConfig {
        entity_format: EntityFormat::Expanded,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("user.rs").unwrap());
}

#[test]
fn snapshot_with_relations() {
    let user_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let user_table = make_table_with_pk("users", user_columns, &["id"]);

    let post_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("user_id", "int4", false),
        make_column("title", "text", false),
        make_column("body", "text", true),
    ];
    let post_table = make_table_with_fk("posts", post_columns, &["id"], "user_id", "users", "id");

    let config = RustSeaOrmCodegenConfig {
        generate_relations: true,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![user_table, post_table]);

    insta::assert_snapshot!("user_with_relations", output.get("user.rs").unwrap());
    insta::assert_snapshot!("post_with_relations", output.get("post.rs").unwrap());
}

#[test]
fn snapshot_mod_file() {
    let user_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let user_table = make_table_with_pk("users", user_columns, &["id"]);

    let post_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("title", "text", false),
    ];
    let post_table = make_table_with_pk("posts", post_columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![user_table, post_table]);

    insta::assert_snapshot!(output.get("mod.rs").unwrap());
}

#[test]
fn snapshot_prelude_file() {
    let user_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let user_table = make_table_with_pk("users", user_columns, &["id"]);

    let post_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("title", "text", false),
    ];
    let post_table = make_table_with_pk("posts", post_columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![user_table, post_table]);

    insta::assert_snapshot!(output.get("prelude.rs").unwrap());
}

#[test]
fn snapshot_complex_types() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("uuid_col", "uuid", false),
        make_column("created_at", "timestamptz", false),
        make_column("updated_at", "timestamp", true),
        make_column("metadata", "jsonb", true),
        make_column("price", "numeric", false),
        make_column("is_active", "bool", false),
    ];
    let table = make_table_with_pk("complex_table", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("complex_table.rs").unwrap());
}
