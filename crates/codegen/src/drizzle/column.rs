//! Drizzle column builder chain generation.
//!
//! This module handles generating the chained method calls for Drizzle column
//! definitions, including type builders, constraints, and defaults.

use tern_ddl::{Column, ConstraintKind, Table};

use super::imports::ImportCollector;
use super::naming::to_column_js_name;
use super::type_mapping::map_pg_type;

/// Information about a generated Drizzle column definition.
#[derive(Debug)]
pub struct ColumnInfo {
    /// The JavaScript variable name for the column.
    pub js_name: String,
    /// The original database column name.
    pub db_name: String,
    /// The complete builder chain expression (e.g., `serial("id").primaryKey()`).
    pub builder_chain: String,
    /// Required imports from "drizzle-orm/pg-core".
    pub imports: Vec<String>,
    /// Warnings for this column.
    pub warnings: Vec<String>,
}

/// Context for column generation containing table-level constraint information.
pub struct ColumnContext<'a> {
    /// Set of column names that form the primary key.
    pub primary_key_columns: Vec<&'a str>,
    /// Whether the primary key is composite (more than one column).
    pub is_composite_pk: bool,
    /// Map of column name to `(referenced_table_js_name, referenced_column_js_name)`.
    pub foreign_key_columns: Vec<(&'a str, String, String)>,
    /// Set of column names with single-column unique constraints.
    pub unique_columns: Vec<&'a str>,
}

impl<'a> ColumnContext<'a> {
    /// Creates a new column context from a table.
    pub fn from_table(table: &'a Table) -> Self {
        let mut primary_key_columns = Vec::new();
        let mut foreign_key_columns = Vec::new();
        let mut unique_columns = Vec::new();

        for constraint in &table.constraints {
            match &constraint.kind {
                ConstraintKind::PrimaryKey(pk) => {
                    for col in &pk.columns {
                        primary_key_columns.push(col.as_ref());
                    }
                }
                ConstraintKind::ForeignKey(fk) => {
                    if fk.columns.len() == 1 && fk.referenced_columns.len() == 1 {
                        let col_name = fk.columns[0].as_ref();
                        let ref_table = fk.referenced_table.name.as_ref();
                        let ref_col = fk.referenced_columns[0].as_ref();
                        foreign_key_columns.push((
                            col_name,
                            ref_table.to_string(),
                            ref_col.to_string(),
                        ));
                    }
                }
                ConstraintKind::Unique(unique) => {
                    if unique.columns.len() == 1 {
                        unique_columns.push(unique.columns[0].as_ref());
                    }
                }
                _ => {}
            }
        }

        let is_composite_pk = primary_key_columns.len() > 1;

        Self {
            primary_key_columns,
            is_composite_pk,
            foreign_key_columns,
            unique_columns,
        }
    }

    /// Checks if a column is a primary key.
    fn is_primary_key(&self, column_name: &str) -> bool {
        self.primary_key_columns.contains(&column_name)
    }

    /// Gets the foreign key reference for a column, if any.
    fn get_foreign_key(&self, column_name: &str) -> Option<(&str, &str)> {
        self.foreign_key_columns
            .iter()
            .find(|(col, _, _)| *col == column_name)
            .map(|(_, table, col)| (table.as_str(), col.as_str()))
    }

    /// Checks if a column has a unique constraint.
    fn is_unique(&self, column_name: &str) -> bool {
        self.unique_columns.contains(&column_name)
    }
}

/// Generates column information for a single column.
pub fn generate_column(column: &Column, ctx: &ColumnContext<'_>, camel_case: bool) -> ColumnInfo {
    let db_name = column.name.as_ref();
    let (js_name, _needs_explicit) = to_column_js_name(db_name, camel_case);

    // Map PG type to Drizzle builder
    let drizzle_type = map_pg_type(&column.type_info);

    let mut imports = ImportCollector::new();
    imports.add_pg_core_all(&drizzle_type.imports);

    let mut warnings = Vec::new();
    if let Some(ref warning) = drizzle_type.warning {
        warnings.push(warning.clone());
    }

    // Build the chain
    let mut chain = String::new();

    // Builder function call: builder("column_name", config?)
    chain.push_str(&drizzle_type.builder);
    chain.push('(');
    chain.push('"');
    chain.push_str(db_name);
    chain.push('"');
    if let Some(ref config) = drizzle_type.config {
        chain.push_str(", ");
        chain.push_str(config);
    }
    chain.push(')');

    // .primaryKey() — only for single-column PKs
    let is_pk = ctx.is_primary_key(db_name);
    if is_pk && !ctx.is_composite_pk {
        chain.push_str(".primaryKey()");
    }

    // .notNull() — serial types are implicitly not null, so skip for them
    let is_serial = is_serial_type(&column.type_info.name.as_ref());
    if !column.is_nullable && !is_serial && !is_pk_serial(is_pk, is_serial) {
        chain.push_str(".notNull()");
    }

    // .unique()
    if ctx.is_unique(db_name) && !is_pk {
        chain.push_str(".unique()");
    }

    // .default(...)
    if let Some(ref default) = column.default {
        let default_expr = default.as_ref();
        // Skip defaults that are just sequence nextval (serial-like)
        if !default_expr.contains("nextval(") {
            chain.push_str(&format!(".default(sql`{default_expr}`)"));
            imports.add_pg_core("sql");
        }
    }

    // .references(() => table.column)
    if let Some((ref_table, ref_col)) = ctx.get_foreign_key(db_name) {
        let ref_col_camel = if camel_case {
            super::naming::to_camel_case(ref_col)
        } else {
            ref_col.to_string()
        };
        chain.push_str(&format!(".references(() => {ref_table}.{ref_col_camel})"));
    }

    ColumnInfo {
        js_name,
        db_name: db_name.to_string(),
        builder_chain: chain,
        imports: imports
            .generate()
            .split('\n')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        warnings,
    }
}

/// Returns the collected imports for a column (for merging into table-level imports).
pub fn collect_column_imports(column: &Column, ctx: &ColumnContext<'_>) -> ImportCollector {
    let db_name = column.name.as_ref();
    let drizzle_type = map_pg_type(&column.type_info);

    let mut imports = ImportCollector::new();
    imports.add_pg_core_all(&drizzle_type.imports);

    // sql import if there's a non-serial default
    if let Some(ref default) = column.default {
        if !default.as_ref().contains("nextval(") {
            imports.add_pg_core("sql");
        }
    }

    // FK references don't require additional imports (the referenced table is just a variable)
    let _ = ctx.get_foreign_key(db_name);

    imports
}

/// Checks if a type is a serial (auto-increment) type.
fn is_serial_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "serial" | "serial2" | "serial4" | "serial8" | "smallserial" | "bigserial"
    )
}

/// Serial PKs are implicitly not null.
fn is_pk_serial(is_pk: bool, is_serial: bool) -> bool {
    is_pk && is_serial
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::QualifiedCollationName;
    use tern_ddl::{
        CollationName, ColumnName, Constraint, ConstraintName, ForeignKeyAction,
        ForeignKeyConstraint, IndexName, Oid, PrimaryKeyConstraint, QualifiedTableName, SchemaName,
        TableKind, TableName, TypeInfo, TypeName, UniqueConstraint,
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
                CollationName::try_new("default".to_string()).unwrap(),
            ),
            comment: None,
        }
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

    #[test]
    fn test_serial_primary_key() {
        let columns = vec![make_column("id", "serial", false)];
        let table = make_table_with_pk("users", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[0], &ctx, true);
        assert_eq!(col.builder_chain, "serial(\"id\").primaryKey()");
        // Serial PK should NOT have .notNull()
        assert!(!col.builder_chain.contains(".notNull()"));
    }

    #[test]
    fn test_not_null_column() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert_eq!(col.builder_chain, "text(\"name\").notNull()");
    }

    #[test]
    fn test_nullable_column() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("bio", "text", true),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert_eq!(col.builder_chain, "text(\"bio\")");
        assert!(!col.builder_chain.contains(".notNull()"));
    }

    #[test]
    fn test_unique_column() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("email", "text", false),
        ];
        let unique = Constraint {
            name: ConstraintName::try_new("users_email_key".to_string()).unwrap(),
            kind: ConstraintKind::Unique(UniqueConstraint {
                columns: vec![ColumnName::try_new("email".to_string()).unwrap()],
                index_name: IndexName::try_new("users_email_key".to_string()).unwrap(),
                nulls_not_distinct: false,
            }),
            comment: None,
        };
        let mut table = make_table_with_pk("users", columns, &["id"]);
        table.constraints.push(unique);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert!(col.builder_chain.contains(".unique()"));
        assert!(col.builder_chain.contains(".notNull()"));
    }

    #[test]
    fn test_foreign_key_column() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("user_id", "int4", false),
        ];
        let fk = Constraint {
            name: ConstraintName::try_new("posts_user_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
                referenced_table: QualifiedTableName::new(
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
        let mut table = make_table_with_pk("posts", columns, &["id"]);
        table.constraints.push(fk);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert_eq!(col.js_name, "userId");
        assert!(col.builder_chain.contains(".references(() => users.id)"));
        assert!(col.builder_chain.contains(".notNull()"));
    }

    #[test]
    fn test_camel_case_column_name() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("created_at", "timestamptz", false),
        ];
        let table = make_table_with_pk("events", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert_eq!(col.js_name, "createdAt");
        assert!(col.builder_chain.contains("\"created_at\""));
    }

    #[test]
    fn test_snake_case_column_name() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("created_at", "timestamptz", false),
        ];
        let table = make_table_with_pk("events", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, false);
        assert_eq!(col.js_name, "created_at");
    }

    #[test]
    fn test_composite_pk_no_primary_key_chain() {
        let columns = vec![
            make_column("order_id", "int4", false),
            make_column("product_id", "int4", false),
        ];
        let table = make_table_with_pk("order_items", columns, &["order_id", "product_id"]);
        let ctx = ColumnContext::from_table(&table);

        // Composite PK columns should NOT have .primaryKey()
        let col1 = generate_column(&table.columns[0], &ctx, true);
        assert!(!col1.builder_chain.contains(".primaryKey()"));
        assert!(col1.builder_chain.contains(".notNull()"));

        let col2 = generate_column(&table.columns[1], &ctx, true);
        assert!(!col2.builder_chain.contains(".primaryKey()"));
    }

    #[test]
    fn test_varchar_with_length() {
        let mut col = make_column("email", "varchar", false);
        col.type_info.formatted = "character varying(255)".to_string();
        let columns = vec![make_column("id", "serial", false), col];
        let table = make_table_with_pk("users", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert!(
            col.builder_chain
                .contains("varchar(\"email\", { length: 255 })")
        );
    }

    #[test]
    fn test_timestamp_with_timezone() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("created_at", "timestamptz", false),
        ];
        let table = make_table_with_pk("events", columns, &["id"]);
        let ctx = ColumnContext::from_table(&table);

        let col = generate_column(&table.columns[1], &ctx, true);
        assert!(
            col.builder_chain
                .contains("timestamp(\"created_at\", { withTimezone: true })")
        );
    }
}
