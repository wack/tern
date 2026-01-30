//! Unit tests for Python code generation components.

use crate::Codegen;
use crate::python::naming::{
    is_python_keyword, is_reserved_word, sanitize_identifier, to_class_name, to_module_name,
};
use crate::python::type_mapping::{
    extract_numeric_precision, extract_string_length, is_fixed_length_char, map_pg_type,
};
use crate::python::{OutputMode, PythonCodegenConfig, ReservedWordStrategy};

use tern_ddl::types::{ForeignKeyAction, QualifiedCollationName, QualifiedName, SqlExpr};
use tern_ddl::{
    CheckConstraint, CollationName, Column, ColumnName, Constraint, ConstraintKind, ConstraintName,
    ForeignKeyConstraint, IdentityKind, IndexName, Oid, PrimaryKeyConstraint, SchemaName, Table,
    TableKind, TableName, TypeInfo, TypeName, UniqueConstraint,
};

// =============================================================================
// Test Helpers
// =============================================================================

fn make_type_info(name: &str, formatted: &str, is_array: bool) -> TypeInfo {
    TypeInfo {
        name: TypeName::try_new(name.to_string()).unwrap(),
        schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        formatted: formatted.to_string(),
        is_array,
    }
}

fn make_column(name: &str, type_name: &str, is_nullable: bool) -> Column {
    Column {
        name: ColumnName::try_new(name.to_string()).unwrap(),
        position: 1,
        type_info: make_type_info(type_name, type_name, false),
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

fn make_table(name: &str, columns: Vec<Column>, constraints: Vec<Constraint>) -> Table {
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

fn make_pk_constraint(table_name: &str, columns: &[&str]) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(format!("{}_pkey", table_name)).unwrap(),
        kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
            columns: columns
                .iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(format!("{}_pkey", table_name)).unwrap(),
        }),
        comment: None,
    }
}

fn make_fk_constraint(
    name: &str,
    columns: &[&str],
    ref_table: &str,
    ref_columns: &[&str],
) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
            columns: columns
                .iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            referenced_table: QualifiedName::new(
                SchemaName::try_new("public".to_string()).unwrap(),
                TableName::try_new(ref_table.to_string()).unwrap(),
            ),
            referenced_columns: ref_columns
                .iter()
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

fn make_unique_constraint(name: &str, columns: &[&str]) -> Constraint {
    Constraint {
        name: ConstraintName::try_new(name.to_string()).unwrap(),
        kind: ConstraintKind::Unique(UniqueConstraint {
            columns: columns
                .iter()
                .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                .collect(),
            index_name: IndexName::try_new(name.to_string()).unwrap(),
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

// =============================================================================
// Type Mapping Tests
// =============================================================================

#[test]
fn test_all_integer_types() {
    for type_name in ["int2", "int4", "int8", "smallint", "integer", "bigint"] {
        let ty = make_type_info(type_name, type_name, false);
        let py = map_pg_type(&ty);
        assert_eq!(py.annotation, "int", "Failed for type: {}", type_name);
    }
}

#[test]
fn test_serial_types() {
    for type_name in [
        "serial",
        "serial2",
        "serial4",
        "serial8",
        "smallserial",
        "bigserial",
    ] {
        let ty = make_type_info(type_name, type_name, false);
        let py = map_pg_type(&ty);
        assert_eq!(py.annotation, "int", "Failed for type: {}", type_name);
    }
}

#[test]
fn test_all_float_types() {
    for type_name in ["float4", "float8", "real", "double precision"] {
        let ty = make_type_info(type_name, type_name, false);
        let py = map_pg_type(&ty);
        assert_eq!(py.annotation, "float", "Failed for type: {}", type_name);
    }
}

#[test]
fn test_all_text_types() {
    for type_name in ["text", "varchar", "char", "character", "bpchar", "name"] {
        let ty = make_type_info(type_name, type_name, false);
        let py = map_pg_type(&ty);
        assert_eq!(py.annotation, "str", "Failed for type: {}", type_name);
    }
}

#[test]
fn test_all_datetime_types() {
    let cases = [
        ("date", "date"),
        ("time", "time"),
        ("timetz", "time"),
        ("timestamp", "datetime"),
        ("timestamptz", "datetime"),
        ("interval", "timedelta"),
    ];

    for (pg_type, expected) in cases {
        let ty = make_type_info(pg_type, pg_type, false);
        let py = map_pg_type(&ty);
        assert_eq!(
            py.annotation, expected,
            "Failed for type: {} -> expected {}",
            pg_type, expected
        );
    }
}

#[test]
fn test_json_has_sa_type() {
    for type_name in ["json", "jsonb"] {
        let ty = make_type_info(type_name, type_name, false);
        let py = map_pg_type(&ty);
        assert_eq!(py.sa_type, Some("JSON".to_string()));
        assert!(py.sa_imports.iter().any(|i| i.name == "JSON"));
    }
}

#[test]
fn test_array_types() {
    let cases = [
        ("int4", "integer[]", "list[int]"),
        ("text", "text[]", "list[str]"),
        ("uuid", "uuid[]", "list[UUID]"),
    ];

    for (type_name, formatted, expected) in cases {
        let ty = make_type_info(type_name, formatted, true);
        let py = map_pg_type(&ty);
        assert_eq!(py.annotation, expected);
        assert!(py.sa_type.is_some());
        assert!(py.sa_type.as_ref().unwrap().contains("ARRAY"));
    }
}

// =============================================================================
// Naming Tests
// =============================================================================

#[test]
fn test_all_python_keywords() {
    let keywords = [
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ];

    for kw in keywords {
        assert!(is_python_keyword(kw), "Expected '{}' to be a keyword", kw);
        assert!(is_reserved_word(kw), "Expected '{}' to be reserved", kw);
    }
}

#[test]
fn test_class_name_pluralization() {
    let cases = [
        ("users", "User"),
        ("categories", "Category"),
        ("companies", "Company"),
        ("addresses", "Addresse"), // 'addresses' -> singular is 'addresse' with simple rule
        ("status", "Status"),      // ends in 's' but 'us' ending preserved
        ("analyses", "Analyse"),   // ends in 's' but 'is' ending requires special handling
        ("classes", "Classe"),     // double 's' preserved
    ];

    for (input, expected) in cases {
        let result = to_class_name(input);
        assert_eq!(
            result, expected,
            "to_class_name('{}') = '{}', expected '{}'",
            input, result, expected
        );
    }
}

#[test]
fn test_class_name_snake_to_pascal() {
    let cases = [
        ("user_accounts", "UserAccount"),
        ("order_items", "OrderItem"),
        ("api_keys", "ApiKey"),
        ("http_requests", "HttpRequest"),
    ];

    for (input, expected) in cases {
        let result = to_class_name(input);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_module_name_conversion() {
    let cases = [
        ("users", "user"),
        ("UserAccounts", "user_account"),
        ("order_items", "order_item"),
        ("HTTPRequests", "httprequest"), // Consecutive caps become lowercase
    ];

    for (input, expected) in cases {
        let result = to_module_name(input);
        assert_eq!(result, expected);
    }
}

#[test]
fn test_sanitize_special_characters() {
    let strategy = ReservedWordStrategy::AppendUnderscore;

    let cases = [
        ("column-name", "column_name", true),
        ("column name", "column_name", true),
        ("column.name", "column_name", true),
        ("column@name", "column_name", true),
        ("column#1", "column_1", true),
    ];

    for (input, expected, needs_alias) in cases {
        let (result, alias) = sanitize_identifier(input, &strategy);
        assert_eq!(result, expected, "sanitize('{}') = '{}'", input, result);
        assert_eq!(alias, needs_alias);
    }
}

#[test]
fn test_sanitize_leading_digit() {
    let strategy = ReservedWordStrategy::AppendUnderscore;
    let (result, needs_alias) = sanitize_identifier("1column", &strategy);
    assert_eq!(result, "_1column");
    assert!(needs_alias);
}

#[test]
fn test_reserved_word_strategies() {
    let input = "class";

    let (result, _) = sanitize_identifier(input, &ReservedWordStrategy::AppendUnderscore);
    assert_eq!(result, "class_");

    let (result, _) = sanitize_identifier(
        input,
        &ReservedWordStrategy::PrependPrefix("col_".to_string()),
    );
    assert_eq!(result, "col_class");
}

// =============================================================================
// Full Generation Tests
// =============================================================================

#[test]
fn test_generate_simple_users_table() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("name", "text", false),
        make_column("email", "text", false),
        make_column("bio", "text", true),
    ];
    let constraints = vec![
        make_pk_constraint("users", &["id"]),
        make_unique_constraint("users_email_key", &["email"]),
    ];
    let table = make_table("users", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    // Check class definition
    assert!(content.contains("class User(SQLModel, table=True):"));
    assert!(content.contains("__tablename__ = \"users\""));

    // Check imports
    assert!(content.contains("from sqlmodel import"));
    assert!(content.contains("SQLModel"));
    assert!(content.contains("Field"));

    // Check fields
    assert!(content.contains("id:"));
    assert!(content.contains("name: str"));
    assert!(content.contains("email: str"));
    assert!(content.contains("bio: str | None"));

    // Check constraints
    assert!(content.contains("primary_key=True"));
    assert!(content.contains("unique=True"));
}

#[test]
fn test_generate_table_with_foreign_key() {
    use crate::python::PythonCodegen;

    let user_columns = vec![
        make_column("id", "int4", false),
        make_column("name", "text", false),
    ];
    let user_constraints = vec![make_pk_constraint("users", &["id"])];
    let users = make_table("users", user_columns, user_constraints);

    let post_columns = vec![
        make_column("id", "int4", false),
        make_column("title", "text", false),
        make_column("user_id", "int4", true),
    ];
    let post_constraints = vec![
        make_pk_constraint("posts", &["id"]),
        make_fk_constraint("posts_user_id_fkey", &["user_id"], "users", &["id"]),
    ];
    let posts = make_table("posts", post_columns, post_constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![users, posts]);

    let content = &output["models.py"];

    assert!(content.contains("class User(SQLModel, table=True):"));
    assert!(content.contains("class Post(SQLModel, table=True):"));
    assert!(content.contains("foreign_key="));
}

#[test]
fn test_generate_table_with_check_constraint() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("price", "numeric", false),
        make_column("quantity", "int4", false),
    ];
    let constraints = vec![
        make_pk_constraint("products", &["id"]),
        make_check_constraint("products_price_positive", "price > 0"),
        make_check_constraint("products_quantity_positive", "quantity >= 0"),
    ];
    let table = make_table("products", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("CheckConstraint"));
    assert!(content.contains("price > 0"));
    assert!(content.contains("quantity >= 0"));
}

#[test]
fn test_generate_table_with_composite_unique() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("email", "text", false),
        make_column("tenant_id", "int4", false),
    ];
    let constraints = vec![
        make_pk_constraint("users", &["id"]),
        make_unique_constraint("users_email_tenant_key", &["email", "tenant_id"]),
    ];
    let table = make_table("users", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("__table_args__"));
    assert!(content.contains("UniqueConstraint"));
    assert!(content.contains("\"email\""));
    assert!(content.contains("\"tenant_id\""));
}

#[test]
fn test_generate_table_with_identity_column() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column_with_identity("id", "int4", IdentityKind::Always),
        make_column("name", "text", false),
    ];
    let constraints = vec![make_pk_constraint("users", &["id"])];
    let table = make_table("users", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    // Identity column should be Optional with default=None
    assert!(content.contains("id: int | None"));
    assert!(content.contains("default=None"));
    assert!(content.contains("primary_key=True"));
}

#[test]
fn test_generate_reserved_word_column() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("class", "text", false),
        make_column("from", "text", true),
    ];
    let constraints = vec![make_pk_constraint("items", &["id"])];
    let table = make_table("items", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    // Reserved words should be aliased
    assert!(content.contains("class_:"));
    assert!(content.contains("alias=\"class\""));
    assert!(content.contains("from_:"));
    assert!(content.contains("alias=\"from\""));
}

#[test]
fn test_generate_multi_file_output() {
    use crate::python::PythonCodegen;

    let user_columns = vec![
        make_column("id", "int4", false),
        make_column("name", "text", false),
    ];
    let users = make_table(
        "users",
        user_columns,
        vec![make_pk_constraint("users", &["id"])],
    );

    let post_columns = vec![
        make_column("id", "int4", false),
        make_column("title", "text", false),
    ];
    let posts = make_table(
        "posts",
        post_columns,
        vec![make_pk_constraint("posts", &["id"])],
    );

    let config = PythonCodegenConfig {
        output_mode: OutputMode::MultiFile,
        ..Default::default()
    };
    let codegen = PythonCodegen::new(config);
    let output = codegen.generate(vec![users, posts]);

    // Check files exist
    assert!(output.contains_key("__init__.py"));
    assert!(output.contains_key("user.py"));
    assert!(output.contains_key("post.py"));

    // Check __init__.py
    let init = &output["__init__.py"];
    assert!(init.contains("from .user import User"));
    assert!(init.contains("from .post import Post"));
    assert!(init.contains("__all__"));

    // Check individual files
    assert!(output["user.py"].contains("class User(SQLModel, table=True):"));
    assert!(output["post.py"].contains("class Post(SQLModel, table=True):"));
}

#[test]
fn test_generate_with_datetime_types() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("created_at", "timestamptz", false),
        make_column("updated_at", "timestamptz", true),
        make_column("birth_date", "date", true),
    ];
    let constraints = vec![make_pk_constraint("events", &["id"])];
    let table = make_table("events", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("from datetime import"));
    assert!(content.contains("datetime"));
    assert!(content.contains("date"));
    assert!(content.contains("created_at: datetime"));
    assert!(content.contains("birth_date: date | None"));
}

#[test]
fn test_generate_with_uuid_type() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "uuid", false),
        make_column("name", "text", false),
    ];
    let constraints = vec![make_pk_constraint("items", &["id"])];
    let table = make_table("items", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("from uuid import UUID"));
}

#[test]
fn test_generate_with_decimal_type() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("price", "numeric", false),
        make_column("discount", "numeric", true),
    ];
    let constraints = vec![make_pk_constraint("products", &["id"])];
    let table = make_table("products", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("from decimal import Decimal"));
    assert!(content.contains("price: Decimal"));
    assert!(content.contains("discount: Decimal | None"));
}

#[test]
fn test_generate_with_json_type() {
    use crate::python::PythonCodegen;

    let columns = vec![
        make_column("id", "int4", false),
        make_column("config", "jsonb", false),
        make_column("metadata", "json", true),
    ];
    let constraints = vec![make_pk_constraint("settings", &["id"])];
    let table = make_table("settings", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    let content = &output["models.py"];

    assert!(content.contains("from typing import Any"));
    assert!(content.contains("from sqlalchemy import JSON"));
    assert!(content.contains("dict[str, Any]"));
    assert!(content.contains("sa_type=JSON"));
}

#[test]
fn test_generate_empty_tables() {
    use crate::python::PythonCodegen;

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![]);

    assert!(output.contains_key("models.py"));
    assert!(output["models.py"].contains("No tables to generate"));
}

#[test]
fn test_config_default_values() {
    let config = PythonCodegenConfig::default();

    assert!(!config.generate_base_models);
    assert!(config.module_prefix.is_none());
    assert!(config.include_docstrings);
    assert!(!config.generate_relationships);
    assert_eq!(config.output_mode, OutputMode::SingleFile);
    assert_eq!(
        config.reserved_word_strategy,
        ReservedWordStrategy::AppendUnderscore
    );
}

#[test]
fn test_string_length_extraction() {
    assert_eq!(extract_string_length("character varying(100)"), Some(100));
    assert_eq!(extract_string_length("varchar(50)"), Some(50));
    assert_eq!(extract_string_length("character(10)"), Some(10));
    assert_eq!(extract_string_length("char(5)"), Some(5));
    assert_eq!(extract_string_length("text"), None);
    assert_eq!(extract_string_length("integer"), None);
}

#[test]
fn test_numeric_precision_extraction() {
    assert_eq!(extract_numeric_precision("numeric(10,2)"), Some((10, 2)));
    assert_eq!(extract_numeric_precision("decimal(15,4)"), Some((15, 4)));
    assert_eq!(extract_numeric_precision("numeric(8)"), Some((8, 0)));
    assert_eq!(extract_numeric_precision("numeric"), None);
}

#[test]
fn test_fixed_length_char_detection() {
    assert!(is_fixed_length_char("char"));
    assert!(is_fixed_length_char("character"));
    assert!(is_fixed_length_char("bpchar"));
    assert!(!is_fixed_length_char("varchar"));
    assert!(!is_fixed_length_char("text"));
}
