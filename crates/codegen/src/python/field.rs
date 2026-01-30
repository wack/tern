//! SQLModel field generation.
//!
//! This module handles generating Field() declarations for SQLModel columns,
//! including type annotations, default values, and field parameters.

use tern_ddl::{Column, ConstraintKind, Table};

use super::ReservedWordStrategy;
use super::imports::ImportCollector;
use super::naming::to_attribute_name;
use super::type_mapping::{extract_string_length, is_fixed_length_char, map_pg_type};

/// Information about a generated field.
#[derive(Debug)]
pub struct FieldInfo {
    /// The Python attribute name (may differ from column name).
    pub attr_name: String,
    /// The original database column name.
    /// Kept for debugging and future relationship generation.
    #[allow(dead_code)]
    pub db_column_name: String,
    /// Whether an alias is needed (attr_name != db_column_name).
    /// Kept for debugging and validation.
    #[allow(dead_code)]
    pub needs_alias: bool,
    /// The Python type annotation.
    pub type_annotation: String,
    /// The Field() declaration (if any parameters are needed).
    pub field_declaration: Option<String>,
    /// Default value without Field() (e.g., "= None").
    pub simple_default: Option<String>,
    /// Required imports for this field.
    pub imports: ImportCollector,
    /// Whether this field is a primary key.
    pub is_primary_key: bool,
    /// Whether this field has a foreign key constraint.
    /// Kept for future relationship generation.
    #[allow(dead_code)]
    pub foreign_key: Option<String>,
    /// Whether this field is indexed (non-constraint index).
    /// Kept for debugging and validation.
    #[allow(dead_code)]
    pub is_indexed: bool,
    /// Whether this field has a unique constraint.
    /// Kept for debugging and validation.
    #[allow(dead_code)]
    pub is_unique: bool,
}

/// Context for field generation containing table-level information.
pub struct FieldContext<'a> {
    /// The table containing the column.
    /// Kept for future relationship generation.
    #[allow(dead_code)]
    pub table: &'a Table,
    /// Reserved word handling strategy.
    pub strategy: &'a ReservedWordStrategy,
    /// Set of column names that are primary keys.
    pub primary_key_columns: Vec<&'a str>,
    /// Map of column name to foreign key reference (e.g., "users.id").
    pub foreign_key_columns: Vec<(&'a str, String)>,
    /// Set of column names with unique constraints (single-column only).
    pub unique_columns: Vec<&'a str>,
    /// Set of column names with indexes (non-constraint indexes).
    pub indexed_columns: Vec<&'a str>,
}

impl<'a> FieldContext<'a> {
    /// Creates a new field context from a table.
    pub fn from_table(table: &'a Table, strategy: &'a ReservedWordStrategy) -> Self {
        let mut primary_key_columns = Vec::new();
        let mut foreign_key_columns = Vec::new();
        let mut unique_columns = Vec::new();
        let mut indexed_columns = Vec::new();

        // Extract constraint information
        for constraint in &table.constraints {
            match &constraint.kind {
                ConstraintKind::PrimaryKey(pk) => {
                    for col in &pk.columns {
                        primary_key_columns.push(col.as_ref());
                    }
                }
                ConstraintKind::ForeignKey(fk) => {
                    // For single-column FKs, we can use Field(foreign_key=...)
                    if fk.columns.len() == 1 && fk.referenced_columns.len() == 1 {
                        let col_name = fk.columns[0].as_ref();
                        let ref_table = &fk.referenced_table;
                        let ref_col = fk.referenced_columns[0].as_ref();
                        let fk_ref = format!(
                            "{}.{}.{}",
                            ref_table.schema.as_ref(),
                            ref_table.name.as_ref(),
                            ref_col
                        );
                        foreign_key_columns.push((col_name, fk_ref));
                    }
                }
                ConstraintKind::Unique(unique) => {
                    // Single-column unique constraints can use Field(unique=True)
                    if unique.columns.len() == 1 {
                        unique_columns.push(unique.columns[0].as_ref());
                    }
                }
                _ => {}
            }
        }

        // Extract index information (non-constraint indexes)
        for index in &table.indexes {
            if !index.is_constraint_index && index.columns.len() == 1 {
                if let Some(col) = &index.columns[0].column {
                    indexed_columns.push(col.as_ref());
                }
            }
        }

        Self {
            table,
            strategy,
            primary_key_columns,
            foreign_key_columns,
            unique_columns,
            indexed_columns,
        }
    }

    /// Checks if a column is a primary key.
    pub fn is_primary_key(&self, column_name: &str) -> bool {
        self.primary_key_columns.contains(&column_name)
    }

    /// Gets the foreign key reference for a column, if any.
    pub fn get_foreign_key(&self, column_name: &str) -> Option<&str> {
        self.foreign_key_columns
            .iter()
            .find(|(col, _)| *col == column_name)
            .map(|(_, fk)| fk.as_str())
    }

    /// Checks if a column has a unique constraint.
    pub fn is_unique(&self, column_name: &str) -> bool {
        self.unique_columns.contains(&column_name)
    }

    /// Checks if a column is indexed.
    pub fn is_indexed(&self, column_name: &str) -> bool {
        self.indexed_columns.contains(&column_name)
    }
}

/// Generates field information for a column.
pub fn generate_field(column: &Column, ctx: &FieldContext<'_>) -> FieldInfo {
    let column_name = column.name.as_ref();

    // Convert column name to Python attribute name
    let (attr_name, needs_alias) = to_attribute_name(column_name, ctx.strategy);

    // Map PostgreSQL type to Python type
    let py_type = map_pg_type(&column.type_info);

    // Collect imports
    let mut imports = ImportCollector::new();
    imports.add_all(&py_type.imports);
    imports.add_all(&py_type.sa_imports);

    // Check constraints for this column
    let is_primary_key = ctx.is_primary_key(column_name);
    let foreign_key = ctx.get_foreign_key(column_name).map(|s| s.to_string());
    let is_unique = ctx.is_unique(column_name);
    let is_indexed = ctx.is_indexed(column_name);

    // Determine if we're dealing with an auto-generated primary key (identity or serial)
    let is_auto_pk = is_primary_key
        && (column.identity.is_some() || is_serial_type(&column.type_info.name.as_ref()));

    // Build field parameters
    let mut field_params = Vec::new();

    // Handle nullability and primary key
    let type_annotation = if is_auto_pk {
        // Auto-generated PKs: type is Optional with default=None
        field_params.push("default=None".to_string());
        format!("{} | None", py_type.annotation)
    } else if column.is_nullable {
        format!("{} | None", py_type.annotation)
    } else {
        py_type.annotation.clone()
    };

    // Primary key
    if is_primary_key {
        field_params.push("primary_key=True".to_string());
    }

    // Foreign key
    if let Some(ref fk) = foreign_key {
        field_params.push(format!("foreign_key=\"{}\"", fk));
    }

    // Unique constraint
    if is_unique && !is_primary_key {
        field_params.push("unique=True".to_string());
    }

    // Index
    if is_indexed && !is_primary_key && !is_unique {
        field_params.push("index=True".to_string());
    }

    // String length constraints
    if let Some(length) = extract_string_length(&column.type_info.formatted) {
        if is_fixed_length_char(column.type_info.name.as_ref()) {
            // Fixed-length char: both min and max
            field_params.push(format!("min_length={length}"));
            field_params.push(format!("max_length={length}"));
        } else {
            // Variable-length varchar: only max
            field_params.push(format!("max_length={length}"));
        }
    }

    // SQLAlchemy type (for JSON, ARRAY, etc.)
    if let Some(ref sa_type) = py_type.sa_type {
        field_params.push(format!("sa_type={sa_type}"));
    }

    // Alias for reserved words or sanitized names
    if needs_alias {
        field_params.push(format!("alias=\"{}\"", column_name));
    }

    // Handle default values for nullable non-PK fields
    let simple_default =
        if !is_auto_pk && column.is_nullable && column.default.is_none() && field_params.is_empty()
        {
            Some("None".to_string())
        } else {
            None
        };

    // Determine if we need a Field() declaration
    let field_declaration = if !field_params.is_empty() {
        imports.add_field();
        Some(format!("Field({})", field_params.join(", ")))
    } else {
        None
    };

    FieldInfo {
        attr_name,
        db_column_name: column_name.to_string(),
        needs_alias,
        type_annotation,
        field_declaration,
        simple_default,
        imports,
        is_primary_key,
        foreign_key,
        is_indexed,
        is_unique,
    }
}

/// Checks if a type name represents a serial (auto-increment) type.
fn is_serial_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "serial" | "serial2" | "serial4" | "serial8" | "smallserial" | "bigserial"
    )
}

/// Formats a field as a Python class attribute line.
pub fn format_field_line(field: &FieldInfo) -> String {
    let type_ann = &field.type_annotation;
    let name = &field.attr_name;

    if let Some(ref field_decl) = field.field_declaration {
        format!("    {name}: {type_ann} = {field_decl}")
    } else if let Some(ref default) = field.simple_default {
        format!("    {name}: {type_ann} = {default}")
    } else {
        format!("    {name}: {type_ann}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::{ForeignKeyAction, QualifiedCollationName, QualifiedName};
    use tern_ddl::{
        ColumnName, Constraint, ConstraintName, ForeignKeyConstraint, IndexName, Oid,
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

    #[test]
    fn test_simple_field() {
        let column = make_column("name", "text", false);
        let table = make_table("users", vec![column.clone()], vec![]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert_eq!(field.attr_name, "name");
        assert_eq!(field.type_annotation, "str");
        assert!(!field.needs_alias);
        assert!(field.field_declaration.is_none());
    }

    #[test]
    fn test_nullable_field() {
        let column = make_column("bio", "text", true);
        let table = make_table("users", vec![column.clone()], vec![]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert_eq!(field.type_annotation, "str | None");
        assert_eq!(field.simple_default, Some("None".to_string()));
    }

    #[test]
    fn test_primary_key_field() {
        let column = make_column("id", "int4", false);
        let pk = Constraint {
            name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
            }),
            comment: None,
        };
        let table = make_table("users", vec![column.clone()], vec![pk]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert!(field.is_primary_key);
        assert!(field.field_declaration.is_some());
        let decl = field.field_declaration.unwrap();
        assert!(decl.contains("primary_key=True"));
    }

    #[test]
    fn test_reserved_word_field() {
        let column = make_column("class", "text", false);
        let table = make_table("items", vec![column.clone()], vec![]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert_eq!(field.attr_name, "class_");
        assert!(field.needs_alias);
        assert!(field.field_declaration.is_some());
        let decl = field.field_declaration.unwrap();
        assert!(decl.contains("alias=\"class\""));
    }

    #[test]
    fn test_foreign_key_field() {
        let column = make_column("user_id", "int4", true);
        let fk = Constraint {
            name: ConstraintName::try_new("posts_user_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("users".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::NoAction,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        };
        let table = make_table("posts", vec![column.clone()], vec![fk]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert!(field.foreign_key.is_some());
        assert!(field.field_declaration.is_some());
        let decl = field.field_declaration.unwrap();
        assert!(decl.contains("foreign_key="));
    }

    #[test]
    fn test_unique_field() {
        let column = make_column("email", "text", false);
        let unique = Constraint {
            name: ConstraintName::try_new("users_email_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![ColumnName::try_new("email".to_string()).unwrap()],
                index_name: IndexName::try_new("users_email_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        };
        let table = make_table("users", vec![column.clone()], vec![unique]);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let ctx = FieldContext::from_table(&table, &strategy);

        let field = generate_field(&column, &ctx);

        assert!(field.is_unique);
        assert!(field.field_declaration.is_some());
        let decl = field.field_declaration.unwrap();
        assert!(decl.contains("unique=True"));
    }

    #[test]
    fn test_format_field_line() {
        let field = FieldInfo {
            attr_name: "name".to_string(),
            db_column_name: "name".to_string(),
            needs_alias: false,
            type_annotation: "str".to_string(),
            field_declaration: None,
            simple_default: None,
            imports: ImportCollector::new(),
            is_primary_key: false,
            foreign_key: None,
            is_indexed: false,
            is_unique: false,
        };

        let line = format_field_line(&field);
        assert_eq!(line, "    name: str");
    }

    #[test]
    fn test_format_field_line_with_default() {
        let field = FieldInfo {
            attr_name: "bio".to_string(),
            db_column_name: "bio".to_string(),
            needs_alias: false,
            type_annotation: "str | None".to_string(),
            field_declaration: None,
            simple_default: Some("None".to_string()),
            imports: ImportCollector::new(),
            is_primary_key: false,
            foreign_key: None,
            is_indexed: false,
            is_unique: false,
        };

        let line = format_field_line(&field);
        assert_eq!(line, "    bio: str | None = None");
    }

    #[test]
    fn test_format_field_line_with_field_declaration() {
        let field = FieldInfo {
            attr_name: "id".to_string(),
            db_column_name: "id".to_string(),
            needs_alias: false,
            type_annotation: "int | None".to_string(),
            field_declaration: Some("Field(default=None, primary_key=True)".to_string()),
            simple_default: None,
            imports: ImportCollector::new(),
            is_primary_key: true,
            foreign_key: None,
            is_indexed: false,
            is_unique: false,
        };

        let line = format_field_line(&field);
        assert_eq!(
            line,
            "    id: int | None = Field(default=None, primary_key=True)"
        );
    }
}
