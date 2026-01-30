//! Unit tests for Rust SeaORM code generation components.

use tern_ddl::types::QualifiedCollationName;
use tern_ddl::{
    CollationName, Column, ColumnName, Constraint, ConstraintKind, ConstraintName,
    ForeignKeyAction, ForeignKeyConstraint, IdentityKind, IndexName, Oid, PrimaryKeyConstraint,
    QualifiedTableName, SchemaName, Table, TableKind, TableName, TypeInfo, TypeName,
    UniqueConstraint,
};

use crate::Codegen;
use crate::rust::{
    EntityFormat, OutputMode, ReservedWordStrategy, RustSeaOrmCodegen, RustSeaOrmCodegenConfig,
};

/// Test utility: creates a basic column.
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

/// Test utility: creates a column with identity.
fn make_column_with_identity(name: &str, type_name: &str, identity: IdentityKind) -> Column {
    let mut col = make_column(name, type_name, false);
    col.identity = Some(identity);
    col
}

/// Test utility: creates a column with a specific formatted type.
fn make_column_formatted(
    name: &str,
    type_name: &str,
    formatted: &str,
    is_nullable: bool,
) -> Column {
    let mut col = make_column(name, type_name, is_nullable);
    col.type_info.formatted = formatted.to_string();
    col
}

/// Test utility: creates a table with a primary key.
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

/// Test utility: creates a table with FK constraint.
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
            on_delete: ForeignKeyAction::NoAction,
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

// =============================================================================
// Integration Tests
// =============================================================================

#[test]
fn test_simple_table() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
        make_column("email", "text", true),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    assert!(output.contains_key("user.rs"));
    let code = &output["user.rs"];

    // Check model struct
    assert!(code.contains("pub struct Model"));
    assert!(code.contains("table_name = \"users\""));

    // Check fields
    assert!(code.contains("pub id: i32"));
    assert!(code.contains("pub name: String"));
    assert!(code.contains("pub email: Option<String>"));

    // Check primary key
    assert!(code.contains("#[sea_orm(primary_key)]"));
}

#[test]
fn test_composite_primary_key() {
    let columns = vec![
        make_column("order_id", "int4", false),
        make_column("product_id", "int4", false),
        make_column("quantity", "int4", false),
    ];
    let table = make_table_with_pk("order_items", columns, &["order_id", "product_id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["order_item.rs"];

    // Both PK columns should have primary_key and auto_increment = false
    assert!(code.contains("primary_key"));
    assert!(code.contains("auto_increment = false"));
}

#[test]
fn test_reserved_word_column() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("type", "text", false), // "type" is a reserved word
        make_column("match", "text", true), // "match" is also reserved
    ];
    let table = make_table_with_pk("items", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["item.rs"];

    // Should have column_name attributes for reserved words
    assert!(code.contains("column_name = \"type\""));
    assert!(code.contains("type_:")); // Escaped field name
}

#[test]
fn test_raw_identifier_strategy() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("type", "text", false),
    ];
    let table = make_table_with_pk("items", columns, &["id"]);

    let config = RustSeaOrmCodegenConfig {
        reserved_word_strategy: ReservedWordStrategy::RawIdentifier,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![table]);

    let code = &output["item.rs"];
    assert!(code.contains("r#type"));
}

#[test]
fn test_datetime_types() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("created_at", "timestamptz", false),
        make_column("updated_at", "timestamp", true),
        make_column("birth_date", "date", true),
    ];
    let table = make_table_with_pk("events", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["event.rs"];

    assert!(code.contains("chrono::DateTime<chrono::FixedOffset>"));
    assert!(code.contains("Option<chrono::NaiveDateTime>"));
    assert!(code.contains("Option<chrono::NaiveDate>"));
}

#[test]
fn test_json_types() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("config", "jsonb", false),
        make_column("metadata", "json", true),
    ];
    let table = make_table_with_pk("settings", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["setting.rs"];

    assert!(code.contains("serde_json::Value"));
    assert!(code.contains("column_type = \"JsonBinary\"") || code.contains("JsonBinary"));
}

#[test]
fn test_array_types() {
    let mut tags_column = make_column("tags", "text", false);
    tags_column.type_info.is_array = true;

    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        tags_column,
    ];
    let table = make_table_with_pk("posts", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["post.rs"];
    assert!(code.contains("Vec<String>"));
}

#[test]
fn test_relations_generation() {
    let user_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let user_table = make_table_with_pk("users", user_columns, &["id"]);

    let post_columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("user_id", "int4", false),
        make_column("title", "text", false),
    ];
    let post_table = make_table_with_fk("posts", post_columns, &["id"], "user_id", "users", "id");

    let config = RustSeaOrmCodegenConfig {
        generate_relations: true,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![user_table, post_table]);

    // User should have has_many Posts
    let user_code = &output["user.rs"];
    assert!(user_code.contains("has_many"));
    assert!(user_code.contains("Posts"));

    // Post should have belongs_to User
    let post_code = &output["post.rs"];
    assert!(user_code.contains("has_many"));
    assert!(post_code.contains("belongs_to"));
    assert!(post_code.contains("User"));
}

#[test]
fn test_expanded_format() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let config = RustSeaOrmCodegenConfig {
        entity_format: EntityFormat::Expanded,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![table]);

    let code = &output["user.rs"];

    // Expanded format has explicit types
    assert!(code.contains("DeriveEntity"));
    assert!(code.contains("pub struct Entity;"));
    assert!(code.contains("impl EntityName for Entity"));
    assert!(code.contains("pub enum Column"));
    assert!(code.contains("pub enum PrimaryKey"));
    assert!(code.contains("impl ColumnTrait for Column"));
    assert!(code.contains("impl PrimaryKeyTrait for PrimaryKey"));
}

#[test]
fn test_single_file_output() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let config = RustSeaOrmCodegenConfig {
        output_mode: OutputMode::SingleFile,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![table]);

    assert!(output.contains_key("entities.rs"));
    assert!(!output.contains_key("user.rs"));
    assert!(!output.contains_key("mod.rs"));

    let code = &output["entities.rs"];
    assert!(code.contains("pub mod user"));
}

#[test]
fn test_multi_file_output() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let config = RustSeaOrmCodegenConfig {
        output_mode: OutputMode::MultiFile,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(vec![table]);

    assert!(output.contains_key("mod.rs"));
    assert!(output.contains_key("prelude.rs"));
    assert!(output.contains_key("user.rs"));
}

#[test]
fn test_uuid_primary_key() {
    let columns = vec![
        make_column("id", "uuid", false),
        make_column("name", "text", false),
    ];
    let table = make_table_with_pk("items", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["item.rs"];

    assert!(code.contains("uuid::Uuid"));
    assert!(code.contains("auto_increment = false"));
}

#[test]
fn test_numeric_types() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column_formatted("price", "numeric", "numeric(10,2)", false),
        make_column("quantity", "int2", false),
        make_column("big_value", "int8", true),
    ];
    let table = make_table_with_pk("products", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["product.rs"];

    assert!(code.contains("rust_decimal::Decimal"));
    assert!(code.contains("i16")); // smallint
    assert!(code.contains("Option<i64>")); // bigint nullable
}

#[test]
fn test_varchar_with_length() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column_formatted("name", "varchar", "character varying(255)", false),
    ];
    let table = make_table_with_pk("users", columns, &["id"]);

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["user.rs"];
    assert!(code.contains("pub name: String"));
}

#[test]
fn test_table_without_primary_key() {
    let columns = vec![make_column("data", "text", false)];
    let table = Table {
        oid: Oid::new(1),
        name: TableName::try_new("log_entries".to_string()).unwrap(),
        kind: TableKind::Regular,
        columns,
        constraints: vec![],
        indexes: vec![],
        comment: None,
    };

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["log_entry.rs"];
    assert!(code.contains("WARNING") || code.contains("no primary key"));
}

#[test]
fn test_unique_constraint() {
    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("email", "text", false),
    ];

    let pk = Constraint {
        name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
        }),
        comment: None,
    };

    let unique = Constraint {
        name: ConstraintName::try_new("users_email_key".to_string()).unwrap(),
        kind: ConstraintKind::Unique(UniqueConstraint {
            columns: vec![ColumnName::try_new("email".to_string()).unwrap()],
            index_name: IndexName::try_new("users_email_key".to_string()).unwrap(),
            nulls_not_distinct: false,
        }),
        comment: None,
    };

    let table = Table {
        oid: Oid::new(1),
        name: TableName::try_new("users".to_string()).unwrap(),
        kind: TableKind::Regular,
        columns,
        constraints: vec![pk, unique],
        indexes: vec![],
        comment: None,
    };

    let codegen = RustSeaOrmCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let code = &output["user.rs"];
    assert!(code.contains("#[sea_orm(unique)]"));
}
