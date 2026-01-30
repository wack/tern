//! SQLModel class generation.
//!
//! This module handles generating complete SQLModel class definitions from table schemas.

use tern_ddl::{ConstraintKind, Table};

use super::PythonCodegenConfig;
use super::field::{FieldContext, FieldInfo, format_field_line, generate_field};
use super::imports::ImportCollector;
use super::naming::to_class_name;

/// Information about a generated SQLModel class.
#[derive(Debug)]
pub struct ModelInfo {
    /// The Python class name.
    pub class_name: String,
    /// The database table name.
    pub table_name: String,
    /// Generated field information.
    pub fields: Vec<FieldInfo>,
    /// Required imports.
    pub imports: ImportCollector,
    /// Table args entries (for composite constraints, indexes, etc.).
    pub table_args: Vec<String>,
    /// Docstring for the class (from table comment).
    pub docstring: Option<String>,
    /// Whether this table has any columns.
    pub has_columns: bool,
    /// Warning messages for unsupported features.
    pub warnings: Vec<String>,
}

/// Generates model information for a table.
pub fn generate_model(table: &Table, config: &PythonCodegenConfig) -> ModelInfo {
    let table_name = table.name.as_ref().to_string();
    let class_name = to_class_name(&table_name);

    let mut imports = ImportCollector::new();
    let mut table_args = Vec::new();
    let mut warnings = Vec::new();

    // Add base SQLModel import
    imports.add_sqlmodel();

    // Generate fields
    let ctx = FieldContext::from_table(table, &config.reserved_word_strategy);
    let mut fields: Vec<FieldInfo> = table
        .columns
        .iter()
        .map(|col| {
            let field = generate_field(col, &ctx);
            imports.merge(&field.imports);
            field
        })
        .collect();

    // Sort fields: primary keys first, then required fields, then optional fields
    fields.sort_by(|a, b| {
        // Primary keys come first
        match (a.is_primary_key, b.is_primary_key) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                // Then non-nullable before nullable (by checking if type contains "| None")
                let a_nullable = a.type_annotation.contains("| None");
                let b_nullable = b.type_annotation.contains("| None");
                match (a_nullable, b_nullable) {
                    (false, true) => std::cmp::Ordering::Less,
                    (true, false) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            }
        }
    });

    // Generate __table_args__ for composite constraints and indexes
    generate_table_args(table, &mut table_args, &mut imports, &mut warnings);

    // Generate docstring from table comment
    let docstring = if config.include_docstrings {
        table.comment.as_ref().map(|c| c.as_ref().to_string())
    } else {
        None
    };

    // Check for empty table
    let has_columns = !table.columns.is_empty();
    if !has_columns {
        warnings.push(format!(
            "Table '{}' has no columns. SQLModel requires at least one field.",
            table_name
        ));
    }

    ModelInfo {
        class_name,
        table_name,
        fields,
        imports,
        table_args,
        docstring,
        has_columns,
        warnings,
    }
}

/// Generates __table_args__ entries for composite constraints and indexes.
fn generate_table_args(
    table: &Table,
    table_args: &mut Vec<String>,
    imports: &mut ImportCollector,
    warnings: &mut Vec<String>,
) {
    // Check for composite primary key (already handled in fields via primary_key=True on each column)
    // But if there's only one PK column with identity, it's handled differently

    // Composite unique constraints
    for constraint in &table.constraints {
        match &constraint.kind {
            ConstraintKind::Unique(unique) if unique.columns.len() > 1 => {
                let columns: Vec<String> = unique
                    .columns
                    .iter()
                    .map(|c| format!("\"{}\"", c.as_ref()))
                    .collect();
                let constraint_name = constraint.name.as_ref();
                table_args.push(format!(
                    "UniqueConstraint({}, name=\"{}\")",
                    columns.join(", "),
                    constraint_name
                ));
                imports.add(&super::type_mapping::PythonImport::new(
                    "sqlalchemy",
                    "UniqueConstraint",
                ));
            }
            ConstraintKind::Check(check) => {
                let expr = check.expression.as_ref();
                let constraint_name = constraint.name.as_ref();
                table_args.push(format!(
                    "CheckConstraint(\"{}\", name=\"{}\")",
                    escape_python_string(expr),
                    constraint_name
                ));
                imports.add(&super::type_mapping::PythonImport::new(
                    "sqlalchemy",
                    "CheckConstraint",
                ));
            }
            ConstraintKind::Exclusion(excl) => {
                // Exclusion constraints are not supported by SQLModel
                let constraint_name = constraint.name.as_ref();
                let elements: Vec<String> = excl
                    .elements
                    .iter()
                    .map(|e| format!("{} WITH {}", e.expression.as_ref(), e.operator))
                    .collect();
                warnings.push(format!(
                    "Exclusion constraint '{}' not supported by SQLModel: EXCLUDE USING {} ({})",
                    constraint_name,
                    excl.index_method.as_str(),
                    elements.join(", ")
                ));
            }
            _ => {}
        }
    }

    // Composite indexes (non-constraint indexes with multiple columns)
    for index in &table.indexes {
        if !index.is_constraint_index && index.columns.len() > 1 {
            let columns: Vec<String> = index
                .columns
                .iter()
                .filter_map(|ic| ic.column.as_ref().map(|c| format!("\"{}\"", c.as_ref())))
                .collect();

            if !columns.is_empty() {
                let index_name = index.name.as_ref();
                table_args.push(format!("Index(\"{}\", {})", index_name, columns.join(", ")));
                imports.add(&super::type_mapping::PythonImport::new(
                    "sqlalchemy",
                    "Index",
                ));
            }
        }
    }
}

/// Escapes a string for use in a Python string literal.
fn escape_python_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Formats a complete SQLModel class definition.
pub fn format_model(model: &ModelInfo) -> String {
    let mut lines = Vec::new();

    // Warning comments
    for warning in &model.warnings {
        lines.push(format!("# WARNING: {}", warning));
    }

    // Class definition
    lines.push(format!("class {}(SQLModel, table=True):", model.class_name));

    // Docstring
    if let Some(ref doc) = model.docstring {
        lines.push(format!("    \"\"\"{}\"\"\"", escape_python_string(doc)));
        lines.push(String::new());
    }

    // __tablename__
    lines.push(format!("    __tablename__ = \"{}\"", model.table_name));

    // __table_args__
    if !model.table_args.is_empty() {
        if model.table_args.len() == 1 {
            lines.push(format!("    __table_args__ = ({},)", model.table_args[0]));
        } else {
            lines.push("    __table_args__ = (".to_string());
            for arg in &model.table_args {
                lines.push(format!("        {},", arg));
            }
            lines.push("    )".to_string());
        }
    }

    lines.push(String::new());

    // Fields
    if model.has_columns {
        for field in &model.fields {
            lines.push(format_field_line(field));
        }
    } else {
        lines.push("    pass  # No columns defined".to_string());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::{QualifiedCollationName, SqlExpr};
    use tern_ddl::{
        CheckConstraint, Column, ColumnName, Constraint, ConstraintName, IndexName, Oid,
        PrimaryKeyConstraint, SchemaName, TableKind, TableName, TypeInfo, TypeName,
        UniqueConstraint,
    };

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
                tern_ddl::CollationName::try_new("default".to_string()).unwrap(),
            ),
            comment: None,
        }
    }

    fn make_table_with_pk(name: &str, columns: Vec<Column>, pk_columns: &[&str]) -> Table {
        let pk = Constraint {
            name: ConstraintName::try_new(format!("{}_pkey", name)).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: pk_columns
                    .iter()
                    .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                    .collect(),
                index_name: IndexName::try_new(format!("{}_pkey", name)).unwrap(),
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

    #[test]
    fn test_generate_simple_model() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
            make_column("email", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);

        assert_eq!(model.class_name, "User");
        assert_eq!(model.table_name, "users");
        assert_eq!(model.fields.len(), 3);
        assert!(model.has_columns);
        assert!(model.warnings.is_empty());
    }

    #[test]
    fn test_generate_model_with_nullable() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
            make_column("bio", "text", true),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);

        // Fields should be sorted: PK first, then non-nullable, then nullable
        assert!(model.fields[0].is_primary_key);
        assert!(!model.fields[1].type_annotation.contains("| None"));
        assert!(model.fields[2].type_annotation.contains("| None"));
    }

    #[test]
    fn test_generate_model_with_composite_unique() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("email", "text", false),
            make_column("tenant_id", "int4", false),
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
            name: ConstraintName::try_new("users_email_tenant_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![
                    ColumnName::try_new("email".to_string()).unwrap(),
                    ColumnName::try_new("tenant_id".to_string()).unwrap(),
                ],
                index_name: IndexName::try_new("users_email_tenant_key".to_string()).unwrap(),
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
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);

        assert!(!model.table_args.is_empty());
        assert!(model.table_args[0].contains("UniqueConstraint"));
        assert!(model.table_args[0].contains("email"));
        assert!(model.table_args[0].contains("tenant_id"));
    }

    #[test]
    fn test_generate_model_with_check_constraint() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("price", "numeric", false),
        ];
        let pk = Constraint {
            name: ConstraintName::try_new("products_pkey".to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                index_name: IndexName::try_new("products_pkey".to_string()).unwrap(),
            }),
            comment: None,
        };
        let check = Constraint {
            name: ConstraintName::try_new("products_price_positive".to_string()).unwrap(),
            kind: ConstraintKind::Check(CheckConstraint {
                expression: SqlExpr::new("price > 0".to_string()),
                is_no_inherit: false,
            }),
            comment: None,
        };
        let table = Table {
            oid: Oid::new(1),
            name: TableName::try_new("products".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns,
            constraints: vec![pk, check],
            indexes: vec![],
            comment: None,
        };
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);

        assert!(!model.table_args.is_empty());
        assert!(model.table_args[0].contains("CheckConstraint"));
        assert!(model.table_args[0].contains("price > 0"));
    }

    #[test]
    fn test_format_model() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);
        let output = format_model(&model);

        assert!(output.contains("class User(SQLModel, table=True):"));
        assert!(output.contains("__tablename__ = \"users\""));
        assert!(output.contains("id:"));
        assert!(output.contains("name:"));
    }

    #[test]
    fn test_format_model_with_docstring() {
        let columns = vec![make_column("id", "int4", false)];
        let mut table = make_table_with_pk("users", columns, &["id"]);
        table.comment = Some(tern_ddl::types::Comment::new(
            "User accounts table".to_string(),
        ));

        let config = PythonCodegenConfig {
            include_docstrings: true,
            ..Default::default()
        };

        let model = generate_model(&table, &config);
        let output = format_model(&model);

        assert!(output.contains("\"\"\"User accounts table\"\"\""));
    }

    #[test]
    fn test_empty_table_warning() {
        let table = Table {
            oid: Oid::new(1),
            name: TableName::try_new("empty".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        };
        let config = PythonCodegenConfig::default();

        let model = generate_model(&table, &config);

        assert!(!model.has_columns);
        assert!(!model.warnings.is_empty());
        assert!(model.warnings[0].contains("no columns"));
    }

    #[test]
    fn test_escape_python_string() {
        assert_eq!(escape_python_string("hello"), "hello");
        assert_eq!(escape_python_string("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_python_string("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_python_string("path\\to\\file"), "path\\\\to\\\\file");
    }
}
