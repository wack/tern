//! Snapshot tests for Python code generation.
//!
//! These tests use insta to capture and verify the full output of the code generator,
//! making it easy to review changes to the generated code format.

use crate::Codegen;
use crate::python::{OutputMode, PythonCodegen, PythonCodegenConfig};

use tern_ddl::types::{
    Comment, ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedName, SqlExpr,
};
use tern_ddl::{
    CheckConstraint, CollationName, Column, ColumnName, Constraint, ConstraintKind, ConstraintName,
    ExclusionConstraint, ExclusionElement, ForeignKeyConstraint, IdentityKind, IndexName, Oid,
    PrimaryKeyConstraint, SchemaName, Table, TableKind, TableName, TypeInfo, TypeName,
    UniqueConstraint,
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

// =============================================================================
// Snapshot Tests
// =============================================================================

#[test]
fn snapshot_simple_users_table() {
    let columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("email", "text", false),
        make_column("name", "text", true),
        {
            let mut col = make_column("created_at", "timestamptz", false);
            col.type_info = make_type_info("timestamptz", "timestamp with time zone", false);
            col
        },
    ];

    let constraints = vec![
        make_pk_constraint("users", &["id"]),
        Constraint {
            name: ConstraintName::try_new("users_email_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![ColumnName::try_new("email".to_string()).unwrap()],
                index_name: IndexName::try_new("users_email_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        },
    ];

    let mut table = make_table("users", columns, constraints);
    table.comment = Some(Comment::new(
        "User accounts for the application.".to_string(),
    ));

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_blog_schema() {
    // Users table
    let user_columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("username", "text", false),
        make_column("email", "text", false),
        make_column("bio", "text", true),
        make_column("created_at", "timestamptz", false),
    ];
    let user_constraints = vec![
        make_pk_constraint("users", &["id"]),
        Constraint {
            name: ConstraintName::try_new("users_username_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![ColumnName::try_new("username".to_string()).unwrap()],
                index_name: IndexName::try_new("users_username_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        },
        Constraint {
            name: ConstraintName::try_new("users_email_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![ColumnName::try_new("email".to_string()).unwrap()],
                index_name: IndexName::try_new("users_email_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        },
    ];
    let users = make_table("users", user_columns, user_constraints);

    // Posts table
    let post_columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("title", "text", false),
        make_column("content", "text", false),
        make_column("author_id", "int4", false),
        make_column("published_at", "timestamptz", true),
        make_column("created_at", "timestamptz", false),
    ];
    let post_constraints = vec![
        make_pk_constraint("posts", &["id"]),
        Constraint {
            name: ConstraintName::try_new("posts_author_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("author_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("users".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::Cascade,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        },
    ];
    let posts = make_table("posts", post_columns, post_constraints);

    // Comments table
    let comment_columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("content", "text", false),
        make_column("post_id", "int4", false),
        make_column("author_id", "int4", false),
        make_column("created_at", "timestamptz", false),
    ];
    let comment_constraints = vec![
        make_pk_constraint("comments", &["id"]),
        Constraint {
            name: ConstraintName::try_new("comments_post_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("post_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("posts".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::Cascade,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        },
        Constraint {
            name: ConstraintName::try_new("comments_author_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("author_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("users".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::Cascade,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        },
    ];
    let comments = make_table("comments", comment_columns, comment_constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![users, posts, comments]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_all_postgres_types() {
    let columns = vec![
        // Integer types
        make_column("col_int2", "int2", false),
        make_column("col_int4", "int4", false),
        make_column("col_int8", "int8", false),
        // Float types
        make_column("col_float4", "float4", false),
        make_column("col_float8", "float8", false),
        // Decimal
        {
            let mut col = make_column("col_numeric", "numeric", false);
            col.type_info = make_type_info("numeric", "numeric(10,2)", false);
            col
        },
        // Boolean
        make_column("col_bool", "bool", false),
        // Text types
        make_column("col_text", "text", false),
        {
            let mut col = make_column("col_varchar", "varchar", false);
            col.type_info = make_type_info("varchar", "character varying(255)", false);
            col
        },
        // Date/time types
        make_column("col_date", "date", false),
        make_column("col_time", "time", false),
        make_column("col_timestamp", "timestamp", false),
        make_column("col_timestamptz", "timestamptz", false),
        make_column("col_interval", "interval", false),
        // UUID
        make_column("col_uuid", "uuid", false),
        // JSON
        make_column("col_json", "json", true),
        make_column("col_jsonb", "jsonb", true),
        // Binary
        make_column("col_bytea", "bytea", true),
        // Network types
        make_column("col_inet", "inet", true),
        // Array types
        {
            let mut col = make_column("col_int_array", "int4", false);
            col.type_info = make_type_info("int4", "integer[]", true);
            col
        },
        {
            let mut col = make_column("col_text_array", "text", true);
            col.type_info = make_type_info("text", "text[]", true);
            col
        },
    ];

    // Add a simple PK
    let mut all_columns = vec![{
        let mut col = make_column("id", "int4", false);
        col.identity = Some(IdentityKind::Always);
        col
    }];
    all_columns.extend(columns);

    let constraints = vec![make_pk_constraint("all_types", &["id"])];
    let table = make_table("all_types", all_columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_reserved_words_table() {
    let columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("class", "text", false),
        make_column("from", "text", false),
        make_column("import", "text", true),
        make_column("def", "int4", false),
        make_column("return", "bool", false),
        make_column("yield", "text", true),
        make_column("async", "bool", false),
        make_column("await", "text", true),
        make_column("match", "text", true),
        make_column("case", "int4", true),
    ];

    let constraints = vec![make_pk_constraint("reserved_words", &["id"])];
    let table = make_table("reserved_words", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_complex_constraints() {
    let columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("email", "text", false),
        make_column("tenant_id", "int4", false),
        make_column("status", "text", false),
        make_column("price", "numeric", false),
        make_column("quantity", "int4", false),
    ];

    let constraints = vec![
        make_pk_constraint("products", &["id"]),
        // Composite unique
        Constraint {
            name: ConstraintName::try_new("products_email_tenant_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![
                    ColumnName::try_new("email".to_string()).unwrap(),
                    ColumnName::try_new("tenant_id".to_string()).unwrap(),
                ],
                index_name: IndexName::try_new("products_email_tenant_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        },
        // Check constraints
        Constraint {
            name: ConstraintName::try_new("products_price_positive".to_string()).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new("price > 0".to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        },
        Constraint {
            name: ConstraintName::try_new("products_quantity_non_negative".to_string()).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new("quantity >= 0".to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        },
        Constraint {
            name: ConstraintName::try_new("products_status_valid".to_string()).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new("status IN ('draft', 'active', 'archived')".to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        },
    ];

    let table = make_table("products", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_self_referential_table() {
    let columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("name", "text", false),
        make_column("manager_id", "int4", true),
    ];

    let constraints = vec![
        make_pk_constraint("employees", &["id"]),
        Constraint {
            name: ConstraintName::try_new("employees_manager_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("manager_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("employees".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::SetNull,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        },
    ];

    let table = make_table("employees", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_multi_file_output() {
    let user_columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("name", "text", false),
        make_column("email", "text", false),
    ];
    let users = make_table(
        "users",
        user_columns,
        vec![make_pk_constraint("users", &["id"])],
    );

    let post_columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("title", "text", false),
        make_column("user_id", "int4", false),
    ];
    let posts = make_table(
        "posts",
        post_columns,
        vec![
            make_pk_constraint("posts", &["id"]),
            Constraint {
                name: ConstraintName::try_new("posts_user_id_fkey".to_string()).unwrap(),
                kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                    columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
                    referenced_table: QualifiedName::new(
                        SchemaName::try_new("public".to_string()).unwrap(),
                        TableName::try_new("users".to_string()).unwrap(),
                    ),
                    referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                    on_delete: ForeignKeyAction::Cascade,
                    on_update: ForeignKeyAction::NoAction,
                    is_deferrable: false,
                    is_initially_deferred: false,
                }),
                comment: None,
            },
        ],
    );

    let config = PythonCodegenConfig {
        output_mode: OutputMode::MultiFile,
        ..Default::default()
    };
    let codegen = PythonCodegen::new(config);
    let output = codegen.generate(vec![users, posts]);

    // Snapshot each file separately
    insta::assert_snapshot!("multi_file_init", output.get("__init__.py").unwrap());
    insta::assert_snapshot!("multi_file_user", output.get("user.py").unwrap());
    insta::assert_snapshot!("multi_file_post", output.get("post.py").unwrap());
}

#[test]
fn snapshot_exclusion_constraint_warning() {
    let columns = vec![
        {
            let mut col = make_column("id", "int4", false);
            col.identity = Some(IdentityKind::Always);
            col
        },
        make_column("room_id", "int4", false),
        make_column("start_time", "timestamptz", false),
        make_column("end_time", "timestamptz", false),
    ];

    let constraints = vec![
        make_pk_constraint("meetings", &["id"]),
        Constraint {
            name: ConstraintName::try_new("meetings_no_overlap".to_string()).unwrap(),
            kind: ConstraintKind::Exclusion(ExclusionConstraint {
                elements: vec![
                    ExclusionElement {
                        expression: SqlExpr::new("room_id".to_string()),
                        operator: "=".to_string(),
                    },
                    ExclusionElement {
                        expression: SqlExpr::new("tsrange(start_time, end_time)".to_string()),
                        operator: "&&".to_string(),
                    },
                ],
                index_method: IndexMethod::Gist,
                index_name: IndexName::try_new("meetings_no_overlap_idx".to_string()).unwrap(),
                predicate: None,
            }),
            comment: None,
        },
    ];

    let table = make_table("meetings", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_composite_primary_key() {
    let columns = vec![
        make_column("order_id", "int4", false),
        make_column("product_id", "int4", false),
        make_column("quantity", "int4", false),
        make_column("unit_price", "numeric", false),
    ];

    let constraints = vec![make_pk_constraint(
        "order_items",
        &["order_id", "product_id"],
    )];

    let table = make_table("order_items", columns, constraints);

    let codegen = PythonCodegen::with_defaults();
    let output = codegen.generate(vec![table]);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}
