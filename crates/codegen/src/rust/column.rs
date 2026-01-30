//! Column attribute generation for SeaORM code generation.
//!
//! This module provides utilities for generating column attributes and
//! detecting column properties from PostgreSQL table definitions.

use tern_ddl::{Column, Constraint, ConstraintKind, Index};

use super::ReservedWordStrategy;
use super::naming::{SanitizedName, to_field_name};
use super::primary_key::{is_column_auto_increment, is_primary_key_column};
use super::type_mapping::{RustType, map_pg_type};

/// Information about a column needed for code generation.
#[derive(Debug, Clone)]
pub struct ColumnInfo {
    /// The sanitized field name.
    pub field_name: SanitizedName,
    /// The Rust type for this column.
    pub rust_type: RustType,
    /// Whether this column is nullable.
    pub is_nullable: bool,
    /// Whether this column is a primary key.
    pub is_primary_key: bool,
    /// Whether this column has auto-increment behavior.
    pub is_auto_increment: bool,
    /// Whether this column has a unique constraint.
    pub is_unique: bool,
    /// Whether this column is indexed.
    pub is_indexed: bool,
    /// Whether this is a generated column.
    pub is_generated: bool,
    /// The doc comment for this column (from database comment).
    pub doc_comment: Option<String>,
    /// The original column name from the database.
    pub original_name: String,
}

impl ColumnInfo {
    /// Creates column info from a DDL column and table constraints.
    pub fn from_column(
        column: &Column,
        constraints: &[Constraint],
        indexes: &[Index],
        strategy: &ReservedWordStrategy,
    ) -> Self {
        let field_name = to_field_name(column.name.as_ref(), strategy);
        let rust_type = map_pg_type(&column.type_info);
        let is_pk = is_primary_key_column(column, constraints);
        let is_auto = if is_pk {
            is_column_auto_increment(column)
        } else {
            false
        };

        Self {
            field_name,
            rust_type,
            is_nullable: column.is_nullable,
            is_primary_key: is_pk,
            is_auto_increment: is_auto,
            is_unique: has_unique_constraint(column, constraints),
            is_indexed: has_single_column_index(column, indexes),
            is_generated: column.generated.is_some(),
            doc_comment: column.comment.as_ref().map(|c| c.as_ref().to_string()),
            original_name: column.name.as_ref().to_string(),
        }
    }

    /// Returns the Rust type annotation, wrapped in Option if nullable.
    pub fn type_annotation(&self) -> String {
        if self.is_nullable {
            self.rust_type.as_optional().annotation
        } else {
            self.rust_type.annotation.clone()
        }
    }

    /// Generates the `#[sea_orm(...)]` attributes for this column.
    ///
    /// Returns `None` if no attributes are needed.
    pub fn generate_sea_orm_attrs(&self) -> Option<String> {
        let mut attrs = Vec::new();

        // Primary key attribute
        if self.is_primary_key {
            attrs.push("primary_key".to_string());
            if !self.is_auto_increment {
                attrs.push("auto_increment = false".to_string());
            }
        }

        // Unique attribute (only for single-column unique, not PKs)
        if self.is_unique && !self.is_primary_key {
            attrs.push("unique".to_string());
        }

        // Indexed attribute (only for non-constraint indexes)
        if self.is_indexed && !self.is_primary_key && !self.is_unique {
            attrs.push("indexed".to_string());
        }

        // Column name attribute (if field name differs from original)
        if self.field_name.needs_rename_attr {
            attrs.push(format!("column_name = \"{}\"", self.field_name.original));
        }

        // Column type attribute (if needed)
        if let Some(col_type) = self.get_column_type_attr() {
            attrs.push(format!("column_type = \"{col_type}\""));
        }

        // Nullable attribute (explicit for types that need it)
        if self.is_nullable && self.rust_type.needs_column_type_attr {
            attrs.push("nullable".to_string());
        }

        // Ignore attribute for generated columns
        if self.is_generated {
            // Generated columns should be marked as ignore
            // Clear other attrs and just use ignore
            return Some("#[sea_orm(ignore)]".to_string());
        }

        if attrs.is_empty() {
            None
        } else {
            Some(format!("#[sea_orm({})]", attrs.join(", ")))
        }
    }

    /// Gets the column_type attribute value if needed.
    fn get_column_type_attr(&self) -> Option<String> {
        // For compact format, we need explicit column_type for some types
        if self.rust_type.needs_column_type_attr {
            // Extract short form from ColumnType::X
            let col_type = &self.rust_type.column_type;
            if col_type.starts_with("ColumnType::") {
                let rest = &col_type["ColumnType::".len()..];
                // For types with arguments, just return the variant name
                if let Some(paren_pos) = rest.find('(') {
                    return Some(rest[..paren_pos].to_string());
                }
                return Some(rest.to_string());
            }
        }
        None
    }
}

/// Checks if a column has a single-column unique constraint.
pub fn has_unique_constraint(column: &Column, constraints: &[Constraint]) -> bool {
    constraints.iter().any(|c| {
        if let ConstraintKind::Unique(unique) = &c.kind {
            unique.columns.len() == 1 && unique.columns.contains(&column.name)
        } else {
            false
        }
    })
}

/// Checks if a column has a single-column index (non-constraint).
pub fn has_single_column_index(column: &Column, indexes: &[Index]) -> bool {
    indexes.iter().any(|idx| {
        // Only consider non-constraint indexes
        !idx.is_constraint_index
            && idx.columns.len() == 1
            && idx.columns[0]
                .column
                .as_ref()
                .is_some_and(|c| c == &column.name)
    })
}

/// Gets all unique constraints that span multiple columns.
pub fn get_composite_unique_constraints(
    constraints: &[Constraint],
) -> Vec<(&Constraint, &[tern_ddl::ColumnName])> {
    constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::Unique(unique) = &c.kind {
                if unique.columns.len() > 1 {
                    return Some((c, unique.columns.as_slice()));
                }
            }
            None
        })
        .collect()
}

/// Gets all check constraints.
pub fn get_check_constraints(constraints: &[Constraint]) -> Vec<(&Constraint, &str)> {
    constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::Check(check) = &c.kind {
                Some((c, check.expression.as_ref()))
            } else {
                None
            }
        })
        .collect()
}

/// Gets all exclusion constraints.
pub fn get_exclusion_constraints(constraints: &[Constraint]) -> Vec<&Constraint> {
    constraints
        .iter()
        .filter(|c| matches!(c.kind, ConstraintKind::Exclusion(_)))
        .collect()
}

/// Gets all non-constraint indexes.
#[allow(dead_code)]
pub fn get_non_constraint_indexes(indexes: &[Index]) -> Vec<&Index> {
    indexes
        .iter()
        .filter(|idx| !idx.is_constraint_index)
        .collect()
}

/// Gets composite indexes (more than one column).
pub fn get_composite_indexes(indexes: &[Index]) -> Vec<&Index> {
    indexes
        .iter()
        .filter(|idx| !idx.is_constraint_index && idx.columns.len() > 1)
        .collect()
}

/// Generates a comment for a composite unique constraint.
pub fn format_composite_unique_comment(
    constraint: &Constraint,
    columns: &[tern_ddl::ColumnName],
) -> String {
    let col_names: Vec<_> = columns.iter().map(|c| c.as_ref()).collect();
    format!(
        "// Composite unique constraint: {} ({})\n// Note: Composite unique constraints are enforced at database level",
        constraint.name.as_ref(),
        col_names.join(", ")
    )
}

/// Generates a comment for a check constraint.
pub fn format_check_constraint_comment(constraint: &Constraint, expression: &str) -> String {
    format!(
        "// Check constraint: {} ({})\n// Note: Check constraints are enforced at database level",
        constraint.name.as_ref(),
        expression
    )
}

/// Generates a warning comment for an exclusion constraint.
pub fn format_exclusion_constraint_warning(constraint: &Constraint) -> String {
    format!(
        "// WARNING: Exclusion constraint '{}' not supported by SeaORM.",
        constraint.name.as_ref()
    )
}

/// Generates a comment for a complex index.
pub fn format_index_comment(index: &Index) -> String {
    let col_names: Vec<_> = index
        .columns
        .iter()
        .map(|ic| {
            let name = ic
                .column
                .as_ref()
                .map(|c| c.as_ref().to_string())
                .unwrap_or_else(|| {
                    ic.expression
                        .as_ref()
                        .map(|e| e.as_ref().to_string())
                        .unwrap_or_else(|| "?".to_string())
                });

            let order = ic.order.as_sql().unwrap_or("");
            if order.is_empty() {
                name
            } else {
                format!("{name} {order}")
            }
        })
        .collect();

    let mut comment = format!(
        "// Index: {} ({})",
        index.name.as_ref(),
        col_names.join(", ")
    );

    if let Some(pred) = &index.predicate {
        comment.push_str(&format!(" WHERE {}", pred.as_ref()));
    }

    comment
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::{IndexMethod, QualifiedCollationName};
    use tern_ddl::{
        CollationName, ColumnName, ConstraintName, IndexColumn, IndexName, NullsOrder, Oid,
        PrimaryKeyConstraint, SchemaName, SortOrder, TypeInfo, TypeName, UniqueConstraint,
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

    fn make_index(name: &str, columns: &[&str], is_constraint: bool) -> Index {
        Index {
            oid: Oid::new(1),
            name: IndexName::try_new(name.to_string()).unwrap(),
            method: IndexMethod::BTree,
            is_unique: false,
            is_constraint_index: is_constraint,
            columns: columns
                .iter()
                .map(|c| IndexColumn {
                    column: Some(ColumnName::try_new(c.to_string()).unwrap()),
                    expression: None,
                    order: SortOrder::Ascending,
                    nulls: NullsOrder::Last,
                })
                .collect(),
            predicate: None,
            comment: None,
        }
    }

    #[test]
    fn test_column_info_simple() {
        let col = make_column("id", "int4", false);
        let constraints = vec![make_pk_constraint("pk", &["id"])];
        let indexes = vec![];
        let strategy = ReservedWordStrategy::AppendUnderscore;

        let info = ColumnInfo::from_column(&col, &constraints, &indexes, &strategy);
        assert_eq!(info.field_name.identifier, "id");
        assert!(info.is_primary_key);
        assert!(!info.is_nullable);
    }

    #[test]
    fn test_column_info_nullable() {
        let col = make_column("email", "text", true);
        let constraints = vec![];
        let indexes = vec![];
        let strategy = ReservedWordStrategy::AppendUnderscore;

        let info = ColumnInfo::from_column(&col, &constraints, &indexes, &strategy);
        assert!(info.is_nullable);
        assert_eq!(info.type_annotation(), "Option<String>");
    }

    #[test]
    fn test_column_info_unique() {
        let col = make_column("email", "text", false);
        let constraints = vec![make_unique_constraint("uq_email", &["email"])];
        let indexes = vec![];
        let strategy = ReservedWordStrategy::AppendUnderscore;

        let info = ColumnInfo::from_column(&col, &constraints, &indexes, &strategy);
        assert!(info.is_unique);
    }

    #[test]
    fn test_column_info_indexed() {
        let col = make_column("name", "text", false);
        let constraints = vec![];
        let indexes = vec![make_index("idx_name", &["name"], false)];
        let strategy = ReservedWordStrategy::AppendUnderscore;

        let info = ColumnInfo::from_column(&col, &constraints, &indexes, &strategy);
        assert!(info.is_indexed);
    }

    #[test]
    fn test_generate_sea_orm_attrs_pk() {
        let col = make_column("id", "int4", false);
        let mut col_info =
            ColumnInfo::from_column(&col, &[], &[], &ReservedWordStrategy::AppendUnderscore);
        col_info.is_primary_key = true;
        col_info.is_auto_increment = true;

        let attrs = col_info.generate_sea_orm_attrs().unwrap();
        assert!(attrs.contains("primary_key"));
        assert!(!attrs.contains("auto_increment = false"));
    }

    #[test]
    fn test_generate_sea_orm_attrs_pk_no_auto() {
        let col = make_column("id", "uuid", false);
        let mut col_info =
            ColumnInfo::from_column(&col, &[], &[], &ReservedWordStrategy::AppendUnderscore);
        col_info.is_primary_key = true;
        col_info.is_auto_increment = false;

        let attrs = col_info.generate_sea_orm_attrs().unwrap();
        assert!(attrs.contains("primary_key"));
        assert!(attrs.contains("auto_increment = false"));
    }

    #[test]
    fn test_generate_sea_orm_attrs_unique() {
        let col = make_column("email", "text", false);
        let mut col_info =
            ColumnInfo::from_column(&col, &[], &[], &ReservedWordStrategy::AppendUnderscore);
        col_info.is_unique = true;

        let attrs = col_info.generate_sea_orm_attrs().unwrap();
        assert!(attrs.contains("unique"));
    }

    #[test]
    fn test_generate_sea_orm_attrs_column_name() {
        let col = make_column("type", "text", false);
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let col_info = ColumnInfo::from_column(&col, &[], &[], &strategy);

        let attrs = col_info.generate_sea_orm_attrs().unwrap();
        assert!(attrs.contains("column_name = \"type\""));
    }

    #[test]
    fn test_has_unique_constraint() {
        let col = make_column("email", "text", false);
        let constraints = vec![make_unique_constraint("uq_email", &["email"])];
        assert!(has_unique_constraint(&col, &constraints));

        let constraints_composite = vec![make_unique_constraint(
            "uq_composite",
            &["email", "tenant_id"],
        )];
        assert!(!has_unique_constraint(&col, &constraints_composite));
    }

    #[test]
    fn test_get_composite_unique_constraints() {
        let constraints = vec![
            make_unique_constraint("uq_email", &["email"]),
            make_unique_constraint("uq_composite", &["email", "tenant_id"]),
        ];

        let composite = get_composite_unique_constraints(&constraints);
        assert_eq!(composite.len(), 1);
        assert_eq!(composite[0].1.len(), 2);
    }

    #[test]
    fn test_has_single_column_index() {
        let col = make_column("name", "text", false);
        let indexes = vec![
            make_index("idx_name", &["name"], false),
            make_index("idx_multi", &["name", "email"], false),
            make_index("idx_pk", &["id"], true), // constraint index
        ];

        assert!(has_single_column_index(&col, &indexes));

        let col2 = make_column("id", "int4", false);
        assert!(!has_single_column_index(&col2, &indexes)); // constraint indexes don't count
    }

    #[test]
    fn test_format_composite_unique_comment() {
        let constraint = make_unique_constraint("uq_email_tenant", &["email", "tenant_id"]);
        if let ConstraintKind::Unique(u) = &constraint.kind {
            let comment = format_composite_unique_comment(&constraint, &u.columns);
            assert!(comment.contains("uq_email_tenant"));
            assert!(comment.contains("email, tenant_id"));
        }
    }

    #[test]
    fn test_format_index_comment() {
        let index = make_index("idx_name_email", &["name", "email"], false);
        let comment = format_index_comment(&index);
        assert!(comment.contains("idx_name_email"));
        assert!(comment.contains("name"));
        assert!(comment.contains("email"));
    }
}
