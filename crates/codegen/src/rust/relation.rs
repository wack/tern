//! Relation generation for SeaORM code generation.
//!
//! This module provides utilities for analyzing foreign keys and generating
//! SeaORM relation enums and Related trait implementations.

use std::collections::HashMap;

use tern_ddl::{ColumnName, Constraint, ConstraintKind, ForeignKeyConstraint, Table};

use super::naming::{to_enum_variant, to_module_name, to_plural_relation_name, to_relation_name};

/// The kind of relation between tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationKind {
    /// This table has a foreign key to another table (many-to-one).
    BelongsTo,
    /// Another table has a foreign key to this table (one-to-many).
    HasMany,
    /// Another table has a unique foreign key to this table (one-to-one).
    HasOne,
}

/// Information about a relation for code generation.
#[derive(Debug, Clone)]
pub struct RelationInfo {
    /// The kind of relation.
    pub kind: RelationKind,
    /// The target table name (without schema).
    pub target_table: String,
    /// The target table's schema (if not public).
    #[allow(dead_code)]
    pub target_schema: Option<String>,
    /// The columns in this table that form the FK.
    pub from_columns: Vec<ColumnName>,
    /// The columns in the target table that are referenced.
    pub to_columns: Vec<ColumnName>,
    /// The FK constraint name (for belongs_to relations).
    #[allow(dead_code)]
    pub constraint_name: Option<String>,
    /// Whether this is a self-referential relation.
    pub is_self_referential: bool,
    /// The on_delete action.
    pub on_delete: Option<String>,
    /// The on_update action.
    pub on_update: Option<String>,
}

impl RelationInfo {
    /// Returns the relation enum variant name.
    pub fn variant_name(&self) -> String {
        match self.kind {
            RelationKind::BelongsTo | RelationKind::HasOne => to_relation_name(&self.target_table),
            RelationKind::HasMany => to_plural_relation_name(&self.target_table),
        }
    }

    /// Returns the target entity path for the relation attribute.
    pub fn target_entity_path(&self, is_self_ref: bool) -> String {
        if is_self_ref {
            "Entity".to_string()
        } else {
            let module = to_module_name(&self.target_table);
            format!("super::{module}::Entity")
        }
    }

    /// Returns the from column path for belongs_to relations.
    pub fn from_column_path(&self) -> String {
        if self.from_columns.len() == 1 {
            let variant = to_enum_variant(self.from_columns[0].as_ref());
            format!("Column::{variant}")
        } else {
            // Composite FK
            let variants: Vec<_> = self
                .from_columns
                .iter()
                .map(|c| format!("Column::{}", to_enum_variant(c.as_ref())))
                .collect();
            format!("({})", variants.join(", "))
        }
    }

    /// Returns the to column path for belongs_to relations.
    pub fn to_column_path(&self) -> String {
        if self.is_self_referential {
            if self.to_columns.len() == 1 {
                let variant = to_enum_variant(self.to_columns[0].as_ref());
                format!("Column::{variant}")
            } else {
                let variants: Vec<_> = self
                    .to_columns
                    .iter()
                    .map(|c| format!("Column::{}", to_enum_variant(c.as_ref())))
                    .collect();
                format!("({})", variants.join(", "))
            }
        } else {
            let module = to_module_name(&self.target_table);
            if self.to_columns.len() == 1 {
                let variant = to_enum_variant(self.to_columns[0].as_ref());
                format!("super::{module}::Column::{variant}")
            } else {
                let variants: Vec<_> = self
                    .to_columns
                    .iter()
                    .map(|c| format!("super::{module}::Column::{}", to_enum_variant(c.as_ref())))
                    .collect();
                format!("({})", variants.join(", "))
            }
        }
    }

    /// Generates the `#[sea_orm(...)]` attribute for this relation.
    pub fn generate_relation_attr(&self) -> String {
        match self.kind {
            RelationKind::HasMany => {
                let target = self.target_entity_path(self.is_self_referential);
                format!("#[sea_orm(has_many = \"{target}\")]")
            }
            RelationKind::HasOne => {
                let target = self.target_entity_path(self.is_self_referential);
                format!("#[sea_orm(has_one = \"{target}\")]")
            }
            RelationKind::BelongsTo => {
                let target = self.target_entity_path(self.is_self_referential);
                let from = self.from_column_path();
                let to = self.to_column_path();

                let mut attrs = vec![
                    format!("belongs_to = \"{target}\""),
                    format!("from = \"{from}\""),
                    format!("to = \"{to}\""),
                ];

                // Add on_delete and on_update if not the default (NoAction)
                if let Some(on_delete) = &self.on_delete {
                    if on_delete != "NoAction" {
                        attrs.push(format!("on_delete = \"{on_delete}\""));
                    }
                }
                if let Some(on_update) = &self.on_update {
                    if on_update != "NoAction" {
                        attrs.push(format!("on_update = \"{on_update}\""));
                    }
                }

                format!("#[sea_orm(\n        {}\n    )]", attrs.join(",\n        "))
            }
        }
    }

    /// Generates the Related trait implementation for this relation.
    pub fn generate_related_impl(&self, _self_table: &str) -> String {
        let target = self.target_entity_path(self.is_self_referential);
        let variant = self.variant_name();

        if self.is_self_referential {
            format!(
                r#"impl Related<Entity> for Entity {{
    fn to() -> RelationDef {{
        Relation::{variant}.def()
    }}
}}"#
            )
        } else {
            format!(
                r#"impl Related<{target}> for Entity {{
    fn to() -> RelationDef {{
        Relation::{variant}.def()
    }}
}}"#
            )
        }
    }
}

/// Analyzes foreign keys to determine relations for all tables.
///
/// Returns a map from table name to list of relations.
pub fn analyze_relations(tables: &[Table]) -> HashMap<String, Vec<RelationInfo>> {
    let mut relations: HashMap<String, Vec<RelationInfo>> = HashMap::new();

    // Build a set of table names for self-reference detection
    let table_names: std::collections::HashSet<_> =
        tables.iter().map(|t| t.name.as_ref().to_string()).collect();

    for table in tables {
        let table_name = table.name.as_ref().to_string();

        for constraint in &table.constraints {
            if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                let target_table = fk.referenced_table.name.as_ref().to_string();
                let is_self_ref = target_table == table_name;

                // Table with FK: belongs_to
                let belongs_to = RelationInfo {
                    kind: RelationKind::BelongsTo,
                    target_table: target_table.clone(),
                    target_schema: if fk.referenced_table.schema.as_ref() != "public" {
                        Some(fk.referenced_table.schema.as_ref().to_string())
                    } else {
                        None
                    },
                    from_columns: fk.columns.clone(),
                    to_columns: fk.referenced_columns.clone(),
                    constraint_name: Some(constraint.name.as_ref().to_string()),
                    is_self_referential: is_self_ref,
                    on_delete: Some(fk_action_to_sea_orm(fk.on_delete)),
                    on_update: Some(fk_action_to_sea_orm(fk.on_update)),
                };

                relations
                    .entry(table_name.clone())
                    .or_default()
                    .push(belongs_to);

                // If the target table exists in our table list, add inverse relation
                if table_names.contains(&target_table) && !is_self_ref {
                    let inverse_kind = if is_one_to_one_fk(fk, &table.constraints) {
                        RelationKind::HasOne
                    } else {
                        RelationKind::HasMany
                    };

                    let inverse = RelationInfo {
                        kind: inverse_kind,
                        target_table: table_name.clone(),
                        target_schema: None, // Same schema
                        from_columns: fk.referenced_columns.clone(),
                        to_columns: fk.columns.clone(),
                        constraint_name: None,
                        is_self_referential: false,
                        on_delete: None,
                        on_update: None,
                    };

                    relations
                        .entry(target_table.clone())
                        .or_default()
                        .push(inverse);
                }
            }
        }
    }

    relations
}

/// Determines if a foreign key implies a one-to-one relationship.
///
/// This is true if the FK columns have a unique constraint.
fn is_one_to_one_fk(fk: &ForeignKeyConstraint, constraints: &[Constraint]) -> bool {
    constraints.iter().any(|c| match &c.kind {
        ConstraintKind::Unique(u) => u.columns == fk.columns,
        ConstraintKind::PrimaryKey(pk) => pk.columns == fk.columns,
        _ => false,
    })
}

/// Converts a ForeignKeyAction to SeaORM string representation.
fn fk_action_to_sea_orm(action: tern_ddl::ForeignKeyAction) -> String {
    match action {
        tern_ddl::ForeignKeyAction::NoAction => "NoAction".to_string(),
        tern_ddl::ForeignKeyAction::Restrict => "Restrict".to_string(),
        tern_ddl::ForeignKeyAction::Cascade => "Cascade".to_string(),
        tern_ddl::ForeignKeyAction::SetNull => "SetNull".to_string(),
        tern_ddl::ForeignKeyAction::SetDefault => "SetDefault".to_string(),
    }
}

/// Checks if a table is likely a junction (many-to-many) table.
///
/// A junction table typically:
/// 1. Has exactly two foreign keys
/// 2. Those foreign keys together form the primary key
/// 3. Has few or no other non-FK columns
pub fn is_junction_table(table: &Table) -> bool {
    // Get FK constraints
    let fks: Vec<_> = table
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::ForeignKey(fk) = &c.kind {
                Some(fk)
            } else {
                None
            }
        })
        .collect();

    if fks.len() != 2 {
        return false;
    }

    // Get PK constraint
    let pk = table.constraints.iter().find_map(|c| {
        if let ConstraintKind::PrimaryKey(pk) = &c.kind {
            Some(pk)
        } else {
            None
        }
    });

    let Some(pk) = pk else {
        return false;
    };

    // Check if PK columns match FK columns
    let mut fk_columns: Vec<_> = fks.iter().flat_map(|fk| &fk.columns).collect();
    fk_columns.sort();

    let mut pk_columns: Vec<_> = pk.columns.iter().collect();
    pk_columns.sort();

    if fk_columns != pk_columns {
        return false;
    }

    // Check if there are few other columns
    let non_pk_columns = table
        .columns
        .iter()
        .filter(|c| !pk.columns.contains(&c.name))
        .count();

    non_pk_columns <= 1 // Allow one additional column (like created_at)
}

/// Gets the two target tables for a junction table.
pub fn get_junction_targets(table: &Table) -> Option<(String, String)> {
    if !is_junction_table(table) {
        return None;
    }

    let fks: Vec<_> = table
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::ForeignKey(fk) = &c.kind {
                Some(fk)
            } else {
                None
            }
        })
        .collect();

    if fks.len() == 2 {
        Some((
            fks[0].referenced_table.name.as_ref().to_string(),
            fks[1].referenced_table.name.as_ref().to_string(),
        ))
    } else {
        None
    }
}

/// Generates Related impl with via() for many-to-many relations.
#[allow(dead_code)]
pub fn generate_many_to_many_related(
    junction_table: &str,
    from_table: &str,
    to_table: &str,
) -> String {
    let junction_module = to_module_name(junction_table);
    let to_module = to_module_name(to_table);
    let from_relation = to_relation_name(from_table);
    let to_relation = to_relation_name(to_table);

    format!(
        r#"impl Related<super::{to_module}::Entity> for Entity {{
    fn to() -> RelationDef {{
        super::{junction_module}::Relation::{to_relation}.def()
    }}

    fn via() -> Option<RelationDef> {{
        Some(super::{junction_module}::Relation::{from_relation}.def().rev())
    }}
}}"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::Column;
    use tern_ddl::types::QualifiedCollationName;
    use tern_ddl::{
        CollationName, ColumnName, ConstraintName, ForeignKeyAction, IndexName, Oid,
        PrimaryKeyConstraint, QualifiedTableName, SchemaName, TableKind, TableName, TypeInfo,
        TypeName,
    };

    fn make_column(name: &str, type_name: &str) -> Column {
        Column {
            name: ColumnName::try_new(name.to_string()).unwrap(),
            position: 1,
            type_info: TypeInfo {
                name: TypeName::try_new(type_name.to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: type_name.to_string(),
                is_array: false,
            },
            is_nullable: false,
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

    fn make_pk_constraint(name: &str, columns: &[&str]) -> Constraint {
        Constraint {
            name: ConstraintName::try_new(name.to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: columns
                    .iter()
                    .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                    .collect(),
                index_name: IndexName::try_new(name.to_string()).unwrap(),
            }),
            comment: None,
        }
    }

    fn make_fk_constraint(
        name: &str,
        from_columns: &[&str],
        to_table: &str,
        to_columns: &[&str],
    ) -> Constraint {
        Constraint {
            name: ConstraintName::try_new(name.to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: from_columns
                    .iter()
                    .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                    .collect(),
                referenced_table: QualifiedTableName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new(to_table.to_string()).unwrap(),
                ),
                referenced_columns: to_columns
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
    fn test_relation_info_variant_name() {
        let relation = RelationInfo {
            kind: RelationKind::BelongsTo,
            target_table: "users".to_string(),
            target_schema: None,
            from_columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
            to_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            constraint_name: None,
            is_self_referential: false,
            on_delete: None,
            on_update: None,
        };
        assert_eq!(relation.variant_name(), "User");

        let has_many = RelationInfo {
            kind: RelationKind::HasMany,
            target_table: "posts".to_string(),
            ..relation.clone()
        };
        assert_eq!(has_many.variant_name(), "Posts");
    }

    #[test]
    fn test_relation_info_target_entity_path() {
        let relation = RelationInfo {
            kind: RelationKind::BelongsTo,
            target_table: "users".to_string(),
            target_schema: None,
            from_columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
            to_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            constraint_name: None,
            is_self_referential: false,
            on_delete: None,
            on_update: None,
        };
        assert_eq!(relation.target_entity_path(false), "super::user::Entity");
        assert_eq!(relation.target_entity_path(true), "Entity");
    }

    #[test]
    fn test_relation_info_column_paths() {
        let relation = RelationInfo {
            kind: RelationKind::BelongsTo,
            target_table: "users".to_string(),
            target_schema: None,
            from_columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
            to_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            constraint_name: None,
            is_self_referential: false,
            on_delete: None,
            on_update: None,
        };
        assert_eq!(relation.from_column_path(), "Column::UserId");
        assert_eq!(relation.to_column_path(), "super::user::Column::Id");
    }

    #[test]
    fn test_relation_info_composite_fk() {
        let relation = RelationInfo {
            kind: RelationKind::BelongsTo,
            target_table: "items".to_string(),
            target_schema: None,
            from_columns: vec![
                ColumnName::try_new("left_id".to_string()).unwrap(),
                ColumnName::try_new("right_id".to_string()).unwrap(),
            ],
            to_columns: vec![
                ColumnName::try_new("left_id".to_string()).unwrap(),
                ColumnName::try_new("right_id".to_string()).unwrap(),
            ],
            constraint_name: None,
            is_self_referential: false,
            on_delete: None,
            on_update: None,
        };
        assert_eq!(
            relation.from_column_path(),
            "(Column::LeftId, Column::RightId)"
        );
    }

    #[test]
    fn test_analyze_relations_simple() {
        let users = make_table(
            "users",
            vec![make_column("id", "int4"), make_column("name", "text")],
            vec![make_pk_constraint("users_pkey", &["id"])],
        );

        let posts = make_table(
            "posts",
            vec![
                make_column("id", "int4"),
                make_column("user_id", "int4"),
                make_column("title", "text"),
            ],
            vec![
                make_pk_constraint("posts_pkey", &["id"]),
                make_fk_constraint("posts_user_id_fkey", &["user_id"], "users", &["id"]),
            ],
        );

        let relations = analyze_relations(&[users, posts]);

        // posts should have belongs_to users
        let post_relations = relations.get("posts").unwrap();
        assert_eq!(post_relations.len(), 1);
        assert_eq!(post_relations[0].kind, RelationKind::BelongsTo);
        assert_eq!(post_relations[0].target_table, "users");

        // users should have has_many posts
        let user_relations = relations.get("users").unwrap();
        assert_eq!(user_relations.len(), 1);
        assert_eq!(user_relations[0].kind, RelationKind::HasMany);
        assert_eq!(user_relations[0].target_table, "posts");
    }

    #[test]
    fn test_analyze_relations_self_referential() {
        let employees = make_table(
            "employees",
            vec![
                make_column("id", "int4"),
                make_column("manager_id", "int4"),
                make_column("name", "text"),
            ],
            vec![
                make_pk_constraint("employees_pkey", &["id"]),
                make_fk_constraint(
                    "employees_manager_id_fkey",
                    &["manager_id"],
                    "employees",
                    &["id"],
                ),
            ],
        );

        let relations = analyze_relations(&[employees]);

        let employee_relations = relations.get("employees").unwrap();
        assert_eq!(employee_relations.len(), 1);
        assert!(employee_relations[0].is_self_referential);
        assert_eq!(employee_relations[0].kind, RelationKind::BelongsTo);
    }

    #[test]
    fn test_is_junction_table() {
        let post_tags = make_table(
            "post_tags",
            vec![
                make_column("post_id", "int4"),
                make_column("tag_id", "int4"),
            ],
            vec![
                make_pk_constraint("post_tags_pkey", &["post_id", "tag_id"]),
                make_fk_constraint("post_tags_post_id_fkey", &["post_id"], "posts", &["id"]),
                make_fk_constraint("post_tags_tag_id_fkey", &["tag_id"], "tags", &["id"]),
            ],
        );

        assert!(is_junction_table(&post_tags));

        let regular = make_table(
            "posts",
            vec![make_column("id", "int4"), make_column("title", "text")],
            vec![make_pk_constraint("posts_pkey", &["id"])],
        );

        assert!(!is_junction_table(&regular));
    }

    #[test]
    fn test_get_junction_targets() {
        let post_tags = make_table(
            "post_tags",
            vec![
                make_column("post_id", "int4"),
                make_column("tag_id", "int4"),
            ],
            vec![
                make_pk_constraint("post_tags_pkey", &["post_id", "tag_id"]),
                make_fk_constraint("post_tags_post_id_fkey", &["post_id"], "posts", &["id"]),
                make_fk_constraint("post_tags_tag_id_fkey", &["tag_id"], "tags", &["id"]),
            ],
        );

        let targets = get_junction_targets(&post_tags).unwrap();
        assert_eq!(targets.0, "posts");
        assert_eq!(targets.1, "tags");
    }

    #[test]
    fn test_generate_relation_attr_has_many() {
        let relation = RelationInfo {
            kind: RelationKind::HasMany,
            target_table: "posts".to_string(),
            target_schema: None,
            from_columns: vec![],
            to_columns: vec![],
            constraint_name: None,
            is_self_referential: false,
            on_delete: None,
            on_update: None,
        };

        let attr = relation.generate_relation_attr();
        assert!(attr.contains("has_many"));
        assert!(attr.contains("super::post::Entity"));
    }

    #[test]
    fn test_generate_relation_attr_belongs_to() {
        let relation = RelationInfo {
            kind: RelationKind::BelongsTo,
            target_table: "users".to_string(),
            target_schema: None,
            from_columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
            to_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            constraint_name: None,
            is_self_referential: false,
            on_delete: Some("Cascade".to_string()),
            on_update: None,
        };

        let attr = relation.generate_relation_attr();
        assert!(attr.contains("belongs_to"));
        assert!(attr.contains("from"));
        assert!(attr.contains("to"));
        assert!(attr.contains("on_delete"));
    }
}
