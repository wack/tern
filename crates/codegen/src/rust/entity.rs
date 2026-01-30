//! Entity generation for SeaORM code generation.
//!
//! This module provides utilities for generating SeaORM entity structs and
//! their associated code in both compact and expanded formats.

use tern_ddl::Table;

use super::column::{
    ColumnInfo, format_check_constraint_comment, format_composite_unique_comment,
    format_exclusion_constraint_warning, format_index_comment, get_check_constraints,
    get_composite_indexes, get_composite_unique_constraints, get_exclusion_constraints,
};
use super::imports::ImportCollector;
use super::naming::{to_enum_variant, to_module_name, to_struct_name};
use super::primary_key::{PrimaryKeyInfo, get_primary_key};
use super::relation::{RelationInfo, analyze_relations};
use super::{EntityFormat, RustSeaOrmCodegenConfig};

/// Information about a generated entity.
#[derive(Debug, Clone)]
pub struct EntityInfo {
    /// The table name (original).
    pub table_name: String,
    /// The Rust struct name (PascalCase, singular).
    pub struct_name: String,
    /// The module name (snake_case, singular).
    pub module_name: String,
    /// Column information for all columns.
    pub columns: Vec<ColumnInfo>,
    /// Primary key information.
    pub primary_key: Option<PrimaryKeyInfo>,
    /// Relations for this entity.
    pub relations: Vec<RelationInfo>,
    /// Imports required for this entity.
    pub imports: ImportCollector,
    /// Warning comments to include.
    pub warnings: Vec<String>,
    /// Constraint comments to include.
    pub constraint_comments: Vec<String>,
    /// Index comments to include.
    pub index_comments: Vec<String>,
    /// Doc comment for the entity (from table comment).
    pub doc_comment: Option<String>,
    /// The schema name (if not public).
    pub schema_name: Option<String>,
}

impl EntityInfo {
    /// Creates entity info from a table definition.
    pub fn from_table(
        table: &Table,
        config: &RustSeaOrmCodegenConfig,
        all_tables: &[Table],
    ) -> Self {
        let table_name = table.name.as_ref().to_string();
        let struct_name = to_struct_name(&table_name);
        let module_name = to_module_name(&table_name);

        // Process columns
        let columns: Vec<_> = table
            .columns
            .iter()
            .map(|c| {
                ColumnInfo::from_column(
                    c,
                    &table.constraints,
                    &table.indexes,
                    &config.reserved_word_strategy,
                )
            })
            .collect();

        // Get primary key info
        let primary_key = get_primary_key(table);

        // Collect imports
        let mut imports = ImportCollector::with_sea_orm_prelude();
        for col in &columns {
            imports.add_imports(&col.rust_type.imports);
            imports.add_features(&col.rust_type.required_features);
        }

        // Get relations if enabled
        let relations = if config.generate_relations {
            let all_relations = analyze_relations(all_tables);
            all_relations.get(&table_name).cloned().unwrap_or_default()
        } else {
            Vec::new()
        };

        // Collect warnings
        let mut warnings = Vec::new();

        // Check for missing primary key
        if primary_key.is_none() {
            warnings.push(format!(
                "// WARNING: Table '{}' has no primary key. SeaORM requires a primary key for entity operations.",
                table_name
            ));
        }

        // Check for generated columns
        for col in &columns {
            if col.is_generated {
                warnings.push(format!(
                    "// Note: Column '{}' is a generated column and will be ignored by SeaORM.",
                    col.original_name
                ));
            }
        }

        // Collect constraint comments
        let mut constraint_comments = Vec::new();

        // Composite unique constraints
        for (constraint, cols) in get_composite_unique_constraints(&table.constraints) {
            constraint_comments.push(format_composite_unique_comment(constraint, cols));
        }

        // Check constraints
        for (constraint, expr) in get_check_constraints(&table.constraints) {
            constraint_comments.push(format_check_constraint_comment(constraint, expr));
        }

        // Exclusion constraints
        for constraint in get_exclusion_constraints(&table.constraints) {
            constraint_comments.push(format_exclusion_constraint_warning(constraint));
        }

        // Collect index comments
        let mut index_comments = Vec::new();
        for index in get_composite_indexes(&table.indexes) {
            index_comments.push(format_index_comment(index));
        }

        // Doc comment
        let doc_comment = if config.include_doc_comments {
            table.comment.as_ref().map(|c| c.as_ref().to_string())
        } else {
            None
        };

        Self {
            table_name,
            struct_name,
            module_name,
            columns,
            primary_key,
            relations,
            imports,
            warnings,
            constraint_comments,
            index_comments,
            doc_comment,
            schema_name: config.schema_name.clone(),
        }
    }
}

/// Generates a complete entity module file (compact format).
pub fn generate_entity_compact(entity: &EntityInfo, config: &RustSeaOrmCodegenConfig) -> String {
    let mut lines = Vec::new();

    // Module doc comment
    lines.push(format!(
        "//! SeaORM entity for `{}` table.",
        entity.table_name
    ));
    lines.push("//!".to_string());
    lines.push("//! Generated by Tern.".to_string());
    lines.push(String::new());

    // Imports
    let import_block = entity.imports.generate();
    if !import_block.is_empty() {
        lines.push(import_block);
        lines.push(String::new());
    }

    // Warnings
    for warning in &entity.warnings {
        lines.push(warning.clone());
    }
    if !entity.warnings.is_empty() {
        lines.push(String::new());
    }

    // Constraint comments
    for comment in &entity.constraint_comments {
        lines.push(comment.clone());
    }
    if !entity.constraint_comments.is_empty() {
        lines.push(String::new());
    }

    // Index comments
    for comment in &entity.index_comments {
        lines.push(comment.clone());
    }
    if !entity.index_comments.is_empty() {
        lines.push(String::new());
    }

    // Entity doc comment
    if let Some(doc) = &entity.doc_comment {
        for line in doc.lines() {
            lines.push(format!("/// {line}"));
        }
    }

    // Model struct with DeriveEntityModel
    lines.push("#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]".to_string());

    // Table name attribute
    let mut sea_orm_attrs = vec![format!("table_name = \"{}\"", entity.table_name)];
    if let Some(schema) = &entity.schema_name {
        sea_orm_attrs.push(format!("schema_name = \"{schema}\""));
    }
    lines.push(format!("#[sea_orm({})]", sea_orm_attrs.join(", ")));

    lines.push("pub struct Model {".to_string());

    // Fields
    for col in &entity.columns {
        // Skip generated columns (they're marked with ignore)
        let field_name = &col.field_name.identifier;
        let type_ann = col.type_annotation();

        // Doc comment for field
        if config.include_doc_comments {
            if let Some(doc) = &col.doc_comment {
                for line in doc.lines() {
                    lines.push(format!("    /// {line}"));
                }
            }
        }

        // SeaORM attributes
        if let Some(attrs) = col.generate_sea_orm_attrs() {
            lines.push(format!("    {attrs}"));
        }

        // Field declaration
        lines.push(format!("    pub {field_name}: {type_ann},"));
    }

    lines.push("}".to_string());
    lines.push(String::new());

    // Relation enum
    if config.generate_relations {
        lines.push(generate_relation_enum(&entity.relations));
        lines.push(String::new());

        // Related impls
        for relation in &entity.relations {
            lines.push(relation.generate_related_impl(&entity.table_name));
            lines.push(String::new());
        }
    } else {
        // Empty relation enum
        lines.push("#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]".to_string());
        lines.push("pub enum Relation {}".to_string());
        lines.push(String::new());
    }

    // ActiveModelBehavior impl
    lines.push("impl ActiveModelBehavior for ActiveModel {}".to_string());
    lines.push(String::new());

    lines.join("\n")
}

/// Generates the Relation enum.
fn generate_relation_enum(relations: &[RelationInfo]) -> String {
    let mut lines = Vec::new();

    lines.push("#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]".to_string());

    if relations.is_empty() {
        lines.push("pub enum Relation {}".to_string());
    } else {
        lines.push("pub enum Relation {".to_string());

        for relation in relations {
            let attr = relation.generate_relation_attr();
            let variant = relation.variant_name();
            lines.push(format!("    {attr}"));
            lines.push(format!("    {variant},"));
        }

        lines.push("}".to_string());
    }

    lines.join("\n")
}

/// Generates a complete entity module file (expanded format).
pub fn generate_entity_expanded(entity: &EntityInfo, config: &RustSeaOrmCodegenConfig) -> String {
    let mut lines = Vec::new();

    // Module doc comment
    lines.push(format!(
        "//! SeaORM entity for `{}` table.",
        entity.table_name
    ));
    lines.push("//!".to_string());
    lines.push("//! Generated by Tern.".to_string());
    lines.push(String::new());

    // Imports
    let import_block = entity.imports.generate();
    if !import_block.is_empty() {
        lines.push(import_block);
        lines.push(String::new());
    }

    // Warnings
    for warning in &entity.warnings {
        lines.push(warning.clone());
    }
    if !entity.warnings.is_empty() {
        lines.push(String::new());
    }

    // Entity struct
    lines.push("#[derive(Copy, Clone, Default, Debug, DeriveEntity)]".to_string());
    lines.push("pub struct Entity;".to_string());
    lines.push(String::new());

    // EntityName impl
    lines.push("impl EntityName for Entity {".to_string());
    if let Some(schema) = &entity.schema_name {
        lines.push(format!("    fn schema_name(&self) -> Option<&str> {{"));
        lines.push(format!("        Some(\"{schema}\")"));
        lines.push("    }".to_string());
        lines.push(String::new());
    } else {
        lines.push("    fn schema_name(&self) -> Option<&str> {".to_string());
        lines.push("        None".to_string());
        lines.push("    }".to_string());
        lines.push(String::new());
    }
    lines.push("    fn table_name(&self) -> &str {".to_string());
    lines.push(format!("        \"{}\"", entity.table_name));
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    // Column enum
    lines.push("#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]".to_string());
    lines.push("pub enum Column {".to_string());
    for col in &entity.columns {
        let variant = to_enum_variant(&col.original_name);
        lines.push(format!("    {variant},"));
    }
    lines.push("}".to_string());
    lines.push(String::new());

    // ColumnTrait impl
    lines.push("impl ColumnTrait for Column {".to_string());
    lines.push("    type EntityName = Entity;".to_string());
    lines.push(String::new());
    lines.push("    fn def(&self) -> ColumnDef {".to_string());
    lines.push("        match self {".to_string());
    for col in &entity.columns {
        let variant = to_enum_variant(&col.original_name);
        let col_type = &col.rust_type.column_type;
        let mut def = format!("{col_type}.def()");
        if col.is_nullable {
            def.push_str(".nullable()");
        }
        if col.is_unique && !col.is_primary_key {
            def.push_str(".unique()");
        }
        lines.push(format!("            Self::{variant} => {def},"));
    }
    lines.push("        }".to_string());
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    // PrimaryKey enum
    lines.push("#[derive(Copy, Clone, Debug, EnumIter, DerivePrimaryKey)]".to_string());
    lines.push("pub enum PrimaryKey {".to_string());
    if let Some(pk) = &entity.primary_key {
        for col_name in &pk.columns {
            let variant = to_enum_variant(col_name.as_ref());
            lines.push(format!("    {variant},"));
        }
    }
    lines.push("}".to_string());
    lines.push(String::new());

    // PrimaryKeyTrait impl
    lines.push("impl PrimaryKeyTrait for PrimaryKey {".to_string());

    // Determine the PK value type
    let pk_type = if let Some(pk) = &entity.primary_key {
        if pk.columns.len() == 1 {
            // Find the column type
            entity
                .columns
                .iter()
                .find(|c| c.original_name == pk.columns[0].as_ref())
                .map(|c| c.rust_type.annotation.clone())
                .unwrap_or_else(|| "i32".to_string())
        } else {
            // Composite PK - tuple type
            let types: Vec<_> = pk
                .columns
                .iter()
                .filter_map(|col_name| {
                    entity
                        .columns
                        .iter()
                        .find(|c| c.original_name == col_name.as_ref())
                        .map(|c| c.rust_type.annotation.clone())
                })
                .collect();
            format!("({})", types.join(", "))
        }
    } else {
        "i32".to_string()
    };

    lines.push(format!("    type ValueType = {pk_type};"));
    lines.push(String::new());
    lines.push("    fn auto_increment() -> bool {".to_string());
    let auto_inc = entity
        .primary_key
        .as_ref()
        .map(|pk| pk.is_auto_increment)
        .unwrap_or(false);
    lines.push(format!("        {auto_inc}"));
    lines.push("    }".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    // Model struct
    if let Some(doc) = &entity.doc_comment {
        for line in doc.lines() {
            lines.push(format!("/// {line}"));
        }
    }
    lines
        .push("#[derive(Clone, Debug, PartialEq, Eq, DeriveModel, DeriveActiveModel)]".to_string());
    lines.push("pub struct Model {".to_string());
    for col in &entity.columns {
        if col.is_generated {
            continue; // Skip generated columns in expanded format too
        }
        let field_name = &col.field_name.identifier;
        let type_ann = col.type_annotation();

        if config.include_doc_comments {
            if let Some(doc) = &col.doc_comment {
                for line in doc.lines() {
                    lines.push(format!("    /// {line}"));
                }
            }
        }

        lines.push(format!("    pub {field_name}: {type_ann},"));
    }
    lines.push("}".to_string());
    lines.push(String::new());

    // Relation enum
    if config.generate_relations {
        lines.push(generate_relation_enum(&entity.relations));
        lines.push(String::new());

        // Related impls
        for relation in &entity.relations {
            lines.push(relation.generate_related_impl(&entity.table_name));
            lines.push(String::new());
        }
    } else {
        lines.push("#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]".to_string());
        lines.push("pub enum Relation {}".to_string());
        lines.push(String::new());
    }

    // ActiveModelBehavior impl
    lines.push("impl ActiveModelBehavior for ActiveModel {}".to_string());
    lines.push(String::new());

    lines.join("\n")
}

/// Generates an entity based on the configured format.
pub fn generate_entity(entity: &EntityInfo, config: &RustSeaOrmCodegenConfig) -> String {
    match config.entity_format {
        EntityFormat::Compact => generate_entity_compact(entity, config),
        EntityFormat::Expanded => generate_entity_expanded(entity, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::QualifiedCollationName;
    use tern_ddl::{
        CollationName, Column, ColumnName, Constraint, ConstraintKind, ConstraintName,
        IdentityKind, IndexName, Oid, PrimaryKeyConstraint, SchemaName, TableKind, TableName,
        TypeInfo, TypeName,
    };

    fn make_column(
        name: &str,
        type_name: &str,
        is_nullable: bool,
        identity: Option<IdentityKind>,
    ) -> Column {
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
            identity,
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
    fn test_entity_info_from_table() {
        let columns = vec![
            make_column("id", "int4", false, Some(IdentityKind::Always)),
            make_column("name", "text", false, None),
            make_column("email", "text", true, None),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = RustSeaOrmCodegenConfig::default();

        let entity = EntityInfo::from_table(&table, &config, &[table.clone()]);

        assert_eq!(entity.table_name, "users");
        assert_eq!(entity.struct_name, "User");
        assert_eq!(entity.module_name, "user");
        assert_eq!(entity.columns.len(), 3);
        assert!(entity.primary_key.is_some());
    }

    #[test]
    fn test_generate_entity_compact() {
        let columns = vec![
            make_column("id", "int4", false, Some(IdentityKind::Always)),
            make_column("name", "text", false, None),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = RustSeaOrmCodegenConfig::default();

        let entity = EntityInfo::from_table(&table, &config, &[table.clone()]);
        let code = generate_entity_compact(&entity, &config);

        assert!(code.contains("DeriveEntityModel"));
        assert!(code.contains("table_name = \"users\""));
        assert!(code.contains("pub struct Model"));
        assert!(code.contains("pub id: i32"));
        assert!(code.contains("pub name: String"));
        assert!(code.contains("primary_key"));
    }

    #[test]
    fn test_generate_entity_expanded() {
        let columns = vec![
            make_column("id", "int4", false, Some(IdentityKind::Always)),
            make_column("name", "text", false, None),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let mut config = RustSeaOrmCodegenConfig::default();
        config.entity_format = EntityFormat::Expanded;

        let entity = EntityInfo::from_table(&table, &config, &[table.clone()]);
        let code = generate_entity_expanded(&entity, &config);

        assert!(code.contains("DeriveEntity"));
        assert!(code.contains("pub struct Entity;"));
        assert!(code.contains("impl EntityName for Entity"));
        assert!(code.contains("pub enum Column"));
        assert!(code.contains("pub enum PrimaryKey"));
        assert!(code.contains("impl ColumnTrait for Column"));
        assert!(code.contains("impl PrimaryKeyTrait for PrimaryKey"));
        assert!(code.contains("DeriveModel"));
    }

    #[test]
    fn test_generate_entity_with_nullable() {
        let columns = vec![
            make_column("id", "int4", false, Some(IdentityKind::Always)),
            make_column("bio", "text", true, None),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);
        let config = RustSeaOrmCodegenConfig::default();

        let entity = EntityInfo::from_table(&table, &config, &[table.clone()]);
        let code = generate_entity_compact(&entity, &config);

        assert!(code.contains("pub bio: Option<String>"));
    }

    #[test]
    fn test_generate_entity_without_pk_warning() {
        let columns = vec![make_column("data", "text", false, None)];
        let table = Table {
            oid: Oid::new(1),
            name: TableName::try_new("log_entries".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns,
            constraints: vec![],
            indexes: vec![],
            comment: None,
        };
        let config = RustSeaOrmCodegenConfig::default();

        let entity = EntityInfo::from_table(&table, &config, &[table.clone()]);

        assert!(entity.warnings.iter().any(|w| w.contains("no primary key")));
    }
}
