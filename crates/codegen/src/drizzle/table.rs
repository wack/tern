//! Drizzle table definition generation.
//!
//! This module handles generating `pgTable(...)` calls for Drizzle ORM schema
//! definitions, including columns, composite constraints, and relations.

use tern_ddl::{ConstraintKind, Table};

use super::DrizzleCodegenConfig;
use super::column::{ColumnContext, ColumnInfo, collect_column_imports, generate_column};
use super::imports::ImportCollector;
use super::naming::{
    to_camel_case, to_column_js_name, to_plural_relation_name, to_relation_name, to_table_js_name,
};

/// Information about a generated Drizzle table definition.
#[derive(Debug)]
pub struct TableInfo {
    /// The JavaScript variable name for the table (e.g., `users`).
    pub js_name: String,
    /// The database table name.
    pub db_name: String,
    /// Column definitions.
    pub columns: Vec<ColumnInfo>,
    /// Extra constraint entries for the third pgTable argument.
    pub extra_constraints: Vec<String>,
    /// Required imports.
    pub imports: ImportCollector,
    /// Warnings.
    pub warnings: Vec<String>,
}

/// Generates table information from a Table definition.
pub fn generate_table(table: &Table, config: &DrizzleCodegenConfig) -> TableInfo {
    let db_name = table.name.as_ref().to_string();
    let (js_name, _) = to_table_js_name(&db_name);

    let ctx = ColumnContext::from_table(table);
    let mut imports = ImportCollector::new();
    let mut warnings = Vec::new();

    // Add pgTable or pgSchema import
    if config.schema_name.is_some() {
        imports.add_pg_schema();
    } else {
        imports.add_pg_table();
    }

    // Generate columns
    let columns: Vec<ColumnInfo> = table
        .columns
        .iter()
        .map(|col| {
            let col_imports = collect_column_imports(col, &ctx);
            imports.merge(&col_imports);
            let col_info = generate_column(col, &ctx, config.camel_case_columns);
            for w in &col_info.warnings {
                warnings.push(format!("Column '{}': {}", col_info.db_name, w));
            }
            col_info
        })
        .collect();

    // Generate extra constraints (composite PK, composite unique, check constraints)
    let extra_constraints = generate_extra_constraints(table, config, &mut imports, &mut warnings);

    TableInfo {
        js_name,
        db_name,
        columns,
        extra_constraints,
        imports,
        warnings,
    }
}

/// Generates relations code for a table, if applicable.
pub fn generate_relations(
    table: &Table,
    all_tables: &[Table],
    config: &DrizzleCodegenConfig,
) -> Option<String> {
    let db_name = table.name.as_ref();
    let (table_js_name, _) = to_table_js_name(db_name);

    let mut belongs_to_relations = Vec::new();
    let mut has_many_relations = Vec::new();

    // Find belongs-to relations (this table has FKs to other tables)
    for constraint in &table.constraints {
        if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
            if fk.columns.len() == 1 && fk.referenced_columns.len() == 1 {
                let fk_col = fk.columns[0].as_ref();
                let ref_table = fk.referenced_table.name.as_ref();
                let (ref_table_js, _) = to_table_js_name(ref_table);
                let ref_col = fk.referenced_columns[0].as_ref();

                let relation_name = to_relation_name(fk_col);
                let (fk_col_js, _) = to_column_js_name(fk_col, config.camel_case_columns);
                let (ref_col_js, _) = to_column_js_name(ref_col, config.camel_case_columns);

                belongs_to_relations.push(format!(
                    "  {relation_name}: one({ref_table_js}, {{\n    fields: [{table_js_name}.{fk_col_js}],\n    references: [{ref_table_js}.{ref_col_js}],\n  }})"
                ));
            }
        }
    }

    // Find has-many relations (other tables have FKs to this table)
    for other_table in all_tables {
        if other_table.name.as_ref() == db_name {
            continue;
        }
        for constraint in &other_table.constraints {
            if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                if fk.referenced_table.name.as_ref() == db_name && fk.columns.len() == 1 {
                    let other_name = other_table.name.as_ref();
                    let (other_js, _) = to_table_js_name(other_name);
                    let relation_name = to_plural_relation_name(other_name);
                    has_many_relations.push(format!("  {relation_name}: many({other_js})"));
                }
            }
        }
    }

    if belongs_to_relations.is_empty() && has_many_relations.is_empty() {
        return None;
    }

    // Determine destructured helpers
    let mut helpers = Vec::new();
    if !belongs_to_relations.is_empty() {
        helpers.push("one");
    }
    if !has_many_relations.is_empty() {
        helpers.push("many");
    }
    let helpers_str = helpers.join(", ");

    let mut all_entries = Vec::new();
    all_entries.extend(has_many_relations);
    all_entries.extend(belongs_to_relations);

    let entries = all_entries.join(",\n");

    Some(format!(
        "export const {table_js_name}Relations = relations({table_js_name}, ({{ {helpers_str} }}) => ({{\n{entries},\n}}));"
    ))
}

/// Formats a complete pgTable definition.
pub fn format_table(table_info: &TableInfo, config: &DrizzleCodegenConfig) -> String {
    let mut lines = Vec::new();

    // Warning comments
    for warning in &table_info.warnings {
        lines.push(format!("// WARNING: {warning}"));
    }

    // Determine table constructor
    let table_constructor = if let Some(ref schema) = config.schema_name {
        let schema_var = to_camel_case(schema);
        format!("{schema_var}.table")
    } else {
        "pgTable".to_string()
    };

    // Start pgTable call
    lines.push(format!(
        "export const {} = {table_constructor}(\"{}\", {{",
        table_info.js_name, table_info.db_name
    ));

    // Column definitions
    for col in &table_info.columns {
        lines.push(format!("  {}: {},", col.js_name, col.builder_chain));
    }

    if table_info.extra_constraints.is_empty() {
        lines.push("});".to_string());
    } else {
        // Close columns object, open constraints callback
        lines.push("}, (table) => [".to_string());
        for constraint in &table_info.extra_constraints {
            lines.push(format!("  {constraint},"));
        }
        lines.push("]);".to_string());
    }

    lines.join("\n")
}

/// Generates extra constraint entries for the third pgTable argument.
fn generate_extra_constraints(
    table: &Table,
    config: &DrizzleCodegenConfig,
    imports: &mut ImportCollector,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut constraints = Vec::new();

    // Composite primary key
    let pk_columns = get_pk_columns(table);
    if pk_columns.len() > 1 {
        imports.add_primary_key();
        let cols: Vec<String> = pk_columns
            .iter()
            .map(|c| {
                let (js, _) = to_column_js_name(c, config.camel_case_columns);
                format!("table.{js}")
            })
            .collect();
        constraints.push(format!("primaryKey({{ columns: [{}] }})", cols.join(", ")));
    }

    // Composite unique constraints
    for constraint in &table.constraints {
        match &constraint.kind {
            ConstraintKind::Unique(unique) if unique.columns.len() > 1 => {
                imports.add_unique();
                let constraint_name = constraint.name.as_ref();
                let cols: Vec<String> = unique
                    .columns
                    .iter()
                    .map(|c| {
                        let (js, _) = to_column_js_name(c.as_ref(), config.camel_case_columns);
                        format!("table.{js}")
                    })
                    .collect();
                constraints.push(format!(
                    "unique(\"{constraint_name}\").on({})",
                    cols.join(", ")
                ));
            }
            ConstraintKind::Check(check) => {
                let constraint_name = constraint.name.as_ref();
                let expr = check.expression.as_ref();
                warnings.push(format!(
                    "Check constraint '{constraint_name}' ({expr}) cannot be expressed in Drizzle schema; apply via SQL migration"
                ));
            }
            ConstraintKind::Exclusion(excl) => {
                let constraint_name = constraint.name.as_ref();
                let elements: Vec<String> = excl
                    .elements
                    .iter()
                    .map(|e| format!("{} WITH {}", e.expression.as_ref(), e.operator))
                    .collect();
                warnings.push(format!(
                    "Exclusion constraint '{constraint_name}' (EXCLUDE USING {} ({})) not supported by Drizzle; apply via SQL migration",
                    excl.index_method.as_str(),
                    elements.join(", ")
                ));
            }
            _ => {}
        }
    }

    // Composite indexes (non-constraint indexes with multiple columns)
    for idx in &table.indexes {
        if !idx.is_constraint_index && idx.columns.len() > 1 {
            imports.add_index();
            let idx_name = idx.name.as_ref();
            let cols: Vec<String> = idx
                .columns
                .iter()
                .filter_map(|ic| {
                    ic.column.as_ref().map(|c| {
                        let (js, _) = to_column_js_name(c.as_ref(), config.camel_case_columns);
                        format!("table.{js}")
                    })
                })
                .collect();
            if !cols.is_empty() {
                constraints.push(format!("index(\"{idx_name}\").on({})", cols.join(", ")));
            }
        }
    }

    constraints
}

/// Gets the primary key columns for a table.
fn get_pk_columns(table: &Table) -> Vec<&str> {
    for constraint in &table.constraints {
        if let ConstraintKind::PrimaryKey(pk) = &constraint.kind {
            return pk.columns.iter().map(|c| c.as_ref()).collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::QualifiedCollationName;
    use tern_ddl::{
        CollationName, Column, ColumnName, Constraint, ConstraintName, ForeignKeyAction,
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

    fn default_config() -> DrizzleCodegenConfig {
        DrizzleCodegenConfig::default()
    }

    #[test]
    fn test_generate_simple_table() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
            make_column("email", "text", true),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let info = generate_table(&table, &default_config());
        assert_eq!(info.js_name, "users");
        assert_eq!(info.db_name, "users");
        assert_eq!(info.columns.len(), 3);
        assert!(info.extra_constraints.is_empty());
    }

    #[test]
    fn test_format_simple_table() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let info = generate_table(&table, &default_config());
        let output = format_table(&info, &default_config());

        assert!(output.contains("export const users = pgTable(\"users\", {"));
        assert!(output.contains("id: serial(\"id\").primaryKey()"));
        assert!(output.contains("name: text(\"name\").notNull()"));
        assert!(output.contains("});"));
    }

    #[test]
    fn test_composite_pk_table() {
        let columns = vec![
            make_column("order_id", "int4", false),
            make_column("product_id", "int4", false),
            make_column("quantity", "int4", false),
        ];
        let table = make_table_with_pk("order_items", columns, &["order_id", "product_id"]);

        let info = generate_table(&table, &default_config());
        assert!(!info.extra_constraints.is_empty());

        let output = format_table(&info, &default_config());
        assert!(output.contains("}, (table) => ["));
        assert!(output.contains("primaryKey({ columns: [table.orderId, table.productId] })"));
        assert!(output.contains("]);"));
    }

    #[test]
    fn test_composite_unique_constraint() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("email", "text", false),
            make_column("tenant_id", "int4", false),
        ];
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
        let mut table = make_table_with_pk("users", columns, &["id"]);
        table.constraints.push(unique);

        let info = generate_table(&table, &default_config());
        let output = format_table(&info, &default_config());
        assert!(
            output.contains("unique(\"users_email_tenant_key\").on(table.email, table.tenantId)")
        );
    }

    #[test]
    fn test_relations_generation() {
        let user_columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
        ];
        let user_table = make_table_with_pk("users", user_columns, &["id"]);

        let post_columns = vec![
            make_column("id", "serial", false),
            make_column("title", "text", false),
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
        let mut post_table = make_table_with_pk("posts", post_columns, &["id"]);
        post_table.constraints.push(fk);

        let all_tables = vec![user_table.clone(), post_table.clone()];
        let config = DrizzleCodegenConfig {
            generate_relations: true,
            ..Default::default()
        };

        // User should have has-many posts
        let user_rels = generate_relations(&user_table, &all_tables, &config);
        assert!(user_rels.is_some());
        let user_rels_str = user_rels.unwrap();
        assert!(user_rels_str.contains("usersRelations"));
        assert!(user_rels_str.contains("many(posts)"));

        // Posts should have belongs-to user
        let post_rels = generate_relations(&post_table, &all_tables, &config);
        assert!(post_rels.is_some());
        let post_rels_str = post_rels.unwrap();
        assert!(post_rels_str.contains("postsRelations"));
        assert!(post_rels_str.contains("one(users"));
        assert!(post_rels_str.contains("posts.userId"));
        assert!(post_rels_str.contains("users.id"));
    }

    #[test]
    fn test_no_relations_when_no_fks() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let config = DrizzleCodegenConfig {
            generate_relations: true,
            ..Default::default()
        };

        let rels = generate_relations(&table, &[table.clone()], &config);
        assert!(rels.is_none());
    }

    #[test]
    fn test_schema_name_uses_pg_schema() {
        let columns = vec![
            make_column("id", "serial", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let config = DrizzleCodegenConfig {
            schema_name: Some("myschema".to_string()),
            ..Default::default()
        };

        let info = generate_table(&table, &config);
        let output = format_table(&info, &config);
        assert!(output.contains("myschema.table(\"users\""));
    }
}
