//! SQLModel class generation.
//!
//! This module handles generating complete SQLModel class definitions from table schemas.
//! Also supports generating Pydantic-only base models for validation without DB coupling.

use tern_ddl::{ConstraintKind, Table};

use super::PythonCodegenConfig;
use super::field::{FieldContext, FieldInfo, format_field_line, generate_field};
use super::imports::ImportCollector;
use super::naming::to_class_name;
use super::relationship::{
    RelationshipInfo, add_relationship_imports, format_relationship_line,
    generate_back_relationships, generate_relationships,
};
use super::type_mapping::PythonImport;

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
    /// Base model info (when generate_base_models is enabled).
    pub base_model: Option<BaseModelInfo>,
    /// Relationship information (when generate_relationships is enabled).
    pub relationships: Vec<RelationshipInfo>,
}

/// Information about a generated Pydantic base model.
///
/// Base models are Pydantic-only classes without database coupling,
/// useful for request/response validation in APIs.
#[derive(Debug)]
pub struct BaseModelInfo {
    /// The Python class name (e.g., "UserBase").
    pub class_name: String,
    /// Fields for the base model (excludes PKs and FKs).
    pub fields: Vec<FieldInfo>,
    /// Required imports for the base model.
    /// Note: Currently merged into the main model's imports, but kept for future use.
    #[allow(dead_code)]
    pub imports: ImportCollector,
    /// Docstring for the class.
    pub docstring: Option<String>,
}

/// Generates model information for a table.
///
/// When `generate_relationships` is enabled, pass all tables to correctly
/// generate back_populates relationships.
pub fn generate_model(
    table: &Table,
    config: &PythonCodegenConfig,
    all_tables: Option<&[Table]>,
) -> ModelInfo {
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

    // Generate base model if configured
    let base_model = if config.generate_base_models && has_columns {
        Some(generate_base_model_info(&class_name, &fields, &docstring))
    } else {
        None
    };

    // Add BaseModel import if we're generating base models
    if base_model.is_some() {
        imports.add(&PythonImport::new("pydantic", "BaseModel"));
    }

    // Generate relationships if configured
    let relationships = if config.generate_relationships {
        let tables = all_tables.unwrap_or(&[]);
        let mut rels = generate_relationships(table, tables, true);

        // Also add back-relationships (the "one" side of one-to-many)
        let back_rels = generate_back_relationships(table, tables);
        rels.extend(back_rels);

        // Add relationship imports
        add_relationship_imports(&mut imports, &rels);

        rels
    } else {
        Vec::new()
    };

    ModelInfo {
        class_name,
        table_name,
        fields,
        imports,
        table_args,
        docstring,
        has_columns,
        base_model,
        relationships,
        warnings,
    }
}

/// Generates base model information for Pydantic-only validation models.
///
/// Base models exclude:
/// - Primary key fields (these are auto-generated in the DB)
/// - Foreign key fields (these are DB-specific)
///
/// This produces classes suitable for request/response validation in APIs.
fn generate_base_model_info(
    class_name: &str,
    fields: &[FieldInfo],
    docstring: &Option<String>,
) -> BaseModelInfo {
    let base_class_name = format!("{}Base", class_name);

    // Filter fields: exclude PKs and FKs
    // Base model fields should be simpler (no Field() with db-specific params)
    let base_fields: Vec<FieldInfo> = fields
        .iter()
        .filter(|f| !f.is_primary_key && f.foreign_key.is_none())
        .cloned()
        .collect();

    // Collect imports needed for base model fields
    let mut imports = ImportCollector::new();
    for field in &base_fields {
        imports.merge(&field.imports);
    }

    let docstring = docstring
        .as_ref()
        .map(|d| format!("Base model for {}. {}", class_name, d))
        .or_else(|| Some(format!("Base model for {} validation.", class_name)));

    BaseModelInfo {
        class_name: base_class_name,
        fields: base_fields,
        imports,
        docstring,
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

    // Relationships
    if !model.relationships.is_empty() {
        lines.push(String::new());
        lines.push("    # Relationships".to_string());
        for rel in &model.relationships {
            lines.push(format_relationship_line(rel));
        }
    }

    lines.join("\n")
}

/// Formats a Pydantic base model class definition.
///
/// Base models inherit from `BaseModel` instead of `SQLModel` and don't
/// have `table=True` or `__tablename__`. They're useful for request/response
/// validation in APIs without DB coupling.
pub fn format_base_model(base_model: &BaseModelInfo) -> String {
    let mut lines = Vec::new();

    // Class definition
    lines.push(format!("class {}(BaseModel):", base_model.class_name));

    // Docstring
    if let Some(ref doc) = base_model.docstring {
        lines.push(format!("    \"\"\"{}\"\"\"", escape_python_string(doc)));
        lines.push(String::new());
    }

    // Fields
    if base_model.fields.is_empty() {
        lines.push("    pass  # No fields for base model".to_string());
    } else {
        for field in &base_model.fields {
            lines.push(format_base_field_line(field));
        }
    }

    lines.join("\n")
}

/// Formats a field line for a Pydantic base model.
///
/// This is simpler than SQLModel fields - we remove db-specific parameters
/// like `primary_key`, `foreign_key`, etc.
fn format_base_field_line(field: &FieldInfo) -> String {
    // For base models, we use simpler field definitions without DB-specific params
    if let Some(ref decl) = field.field_declaration {
        // Check if the field declaration only has db-specific params
        // If so, we might want to simplify it
        if decl.contains("default_factory=")
            || decl.contains("min_length=")
            || decl.contains("max_length=")
        {
            // Keep validation-related params
            format!(
                "    {}: {} = {}",
                field.attr_name, field.type_annotation, decl
            )
        } else if field.type_annotation.contains("| None") {
            // Nullable field - use simple None default
            format!("    {}: {} = None", field.attr_name, field.type_annotation)
        } else {
            // Required field - no default
            format!("    {}: {}", field.attr_name, field.type_annotation)
        }
    } else if let Some(ref default) = field.simple_default {
        format!(
            "    {}: {} = {}",
            field.attr_name, field.type_annotation, default
        )
    } else if field.type_annotation.contains("| None") {
        format!("    {}: {} = None", field.attr_name, field.type_annotation)
    } else {
        format!("    {}: {}", field.attr_name, field.type_annotation)
    }
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

        let model = generate_model(&table, &config, None);

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

        let model = generate_model(&table, &config, None);

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

        let model = generate_model(&table, &config, None);

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

        let model = generate_model(&table, &config, None);

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

        let model = generate_model(&table, &config, None);
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

        let model = generate_model(&table, &config, None);
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

        let model = generate_model(&table, &config, None);

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
