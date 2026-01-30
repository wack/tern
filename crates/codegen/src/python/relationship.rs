//! SQLModel relationship generation.
//!
//! This module handles generating `Relationship()` attributes for foreign key relationships
//! between SQLModel models.

use std::collections::HashMap;

use tern_ddl::{ConstraintKind, Table};

use super::imports::ImportCollector;
use super::naming::to_class_name;

/// Information about a generated relationship.
#[derive(Debug, Clone)]
pub struct RelationshipInfo {
    /// The Python attribute name for this relationship.
    pub attr_name: String,
    /// The type annotation (e.g., "User" or 'list["Post"]').
    pub type_annotation: String,
    /// The relationship declaration (e.g., `Relationship(back_populates="posts")`).
    pub declaration: String,
    /// Whether this is a forward reference (needs TYPE_CHECKING import).
    pub is_forward_ref: bool,
    /// The referenced class name (for forward reference imports).
    /// Note: Reserved for future use with TYPE_CHECKING imports.
    #[allow(dead_code)]
    pub referenced_class: String,
    /// The module containing the referenced class (for multi-file mode).
    /// Note: Reserved for future use with multi-file relationship imports.
    #[allow(dead_code)]
    pub referenced_module: Option<String>,
}

/// Generates relationships for a table based on its foreign key constraints.
///
/// Returns relationships for both sides:
/// - The "many" side (the table with the FK)
/// - The "one" side (the referenced table)
pub fn generate_relationships(
    table: &Table,
    all_tables: &[Table],
    generate_back_populates: bool,
) -> Vec<RelationshipInfo> {
    let mut relationships = Vec::new();
    let table_name = table.name.as_ref();

    // Build a map of table names to class names for lookup
    let table_class_map: HashMap<String, String> = all_tables
        .iter()
        .map(|t| (t.name.as_ref().to_string(), to_class_name(t.name.as_ref())))
        .collect();

    // Process each foreign key constraint
    for constraint in &table.constraints {
        if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
            // Only handle single-column FKs for now
            if fk.columns.len() != 1 {
                continue;
            }

            let fk_column = fk.columns[0].as_ref();
            let referenced_table = fk.referenced_table.name.as_ref();
            let referenced_class = table_class_map
                .get(referenced_table)
                .cloned()
                .unwrap_or_else(|| to_class_name(referenced_table));

            // Check if this is a self-referential relationship
            let _is_self_ref = table_name == referenced_table;

            // Generate relationship attribute name
            // For FK column "author_id", relationship would be "author"
            // For FK column "parent_id", relationship would be "parent"
            let attr_name = derive_relationship_name(fk_column);

            // Determine if we need a forward reference
            // Always use forward ref for safety (avoids circular import issues)
            let is_forward_ref = true;

            // Build the back_populates name (pluralized table name)
            let back_populates = if generate_back_populates {
                Some(derive_back_populates_name(table_name))
            } else {
                None
            };

            // Build the relationship declaration
            let declaration = if let Some(bp) = &back_populates {
                format!("Relationship(back_populates=\"{}\")", bp)
            } else {
                "Relationship()".to_string()
            };

            // Type annotation - reference to the related model
            let type_annotation = format!("\"{}\"", referenced_class);

            relationships.push(RelationshipInfo {
                attr_name,
                type_annotation,
                declaration,
                is_forward_ref,
                referenced_class,
                referenced_module: None, // Will be set by caller for multi-file mode
            });
        }
    }

    relationships
}

/// Generates the back-populates relationships for tables that are referenced by FKs.
///
/// This creates the "one" side of one-to-many relationships.
pub fn generate_back_relationships(table: &Table, all_tables: &[Table]) -> Vec<RelationshipInfo> {
    let mut relationships = Vec::new();
    let table_name = table.name.as_ref();

    // Find all tables that have FKs pointing to this table
    for other_table in all_tables {
        let other_table_name = other_table.name.as_ref();
        let other_class_name = to_class_name(other_table_name);

        for constraint in &other_table.constraints {
            if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                // Only handle single-column FKs
                if fk.columns.len() != 1 {
                    continue;
                }

                let referenced_table = fk.referenced_table.name.as_ref();
                if referenced_table != table_name {
                    continue;
                }

                let fk_column = fk.columns[0].as_ref();

                // This table is referenced by other_table
                // Generate the "one" side relationship
                let attr_name = derive_back_populates_name(other_table_name);
                let back_populates = derive_relationship_name(fk_column);

                // Type annotation is a list of the referencing model
                let type_annotation = format!("list[\"{}\"]", &other_class_name);

                let declaration = format!("Relationship(back_populates=\"{}\")", back_populates);

                relationships.push(RelationshipInfo {
                    attr_name,
                    type_annotation,
                    declaration,
                    is_forward_ref: true,
                    referenced_class: other_class_name.clone(),
                    referenced_module: None,
                });
            }
        }
    }

    relationships
}

/// Derives a relationship attribute name from a foreign key column name.
///
/// Examples:
/// - "author_id" -> "author"
/// - "user_id" -> "user"
/// - "parent_id" -> "parent"
/// - "category_fk" -> "category"
fn derive_relationship_name(fk_column: &str) -> String {
    // Remove common FK suffixes
    let name = fk_column
        .strip_suffix("_id")
        .or_else(|| fk_column.strip_suffix("_fk"))
        .or_else(|| fk_column.strip_suffix("Id"))
        .unwrap_or(fk_column);

    name.to_string()
}

/// Derives the back_populates name from a table name.
///
/// This is typically the pluralized, snake_case table name.
/// Examples:
/// - "user" -> "users"
/// - "users" -> "users"
/// - "post" -> "posts"
fn derive_back_populates_name(table_name: &str) -> String {
    // Simple pluralization - just use the table name as-is
    // since table names are usually already plural
    table_name.to_lowercase()
}

/// Adds relationship-related imports to the import collector.
pub fn add_relationship_imports(imports: &mut ImportCollector, relationships: &[RelationshipInfo]) {
    if relationships.is_empty() {
        return;
    }

    // Add Relationship import from sqlmodel
    imports.add_relationship();

    // Add TYPE_CHECKING for forward references if needed
    for rel in relationships {
        if rel.is_forward_ref {
            imports.add(&super::type_mapping::PythonImport::new(
                "typing",
                "TYPE_CHECKING",
            ));
            break; // Only need to add once
        }
    }
}

/// Formats a relationship as a Python class attribute line.
pub fn format_relationship_line(rel: &RelationshipInfo) -> String {
    format!(
        "    {}: {} = {}",
        rel.attr_name, rel.type_annotation, rel.declaration
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_relationship_name() {
        assert_eq!(derive_relationship_name("author_id"), "author");
        assert_eq!(derive_relationship_name("user_id"), "user");
        assert_eq!(derive_relationship_name("parent_id"), "parent");
        assert_eq!(derive_relationship_name("category_fk"), "category");
        assert_eq!(derive_relationship_name("userId"), "user");
        assert_eq!(derive_relationship_name("name"), "name"); // No suffix
    }

    #[test]
    fn test_derive_back_populates_name() {
        assert_eq!(derive_back_populates_name("user"), "user");
        assert_eq!(derive_back_populates_name("users"), "users");
        assert_eq!(derive_back_populates_name("Post"), "post");
        assert_eq!(derive_back_populates_name("UserProfile"), "userprofile");
    }

    #[test]
    fn test_format_relationship_line() {
        let rel = RelationshipInfo {
            attr_name: "author".to_string(),
            type_annotation: "\"User\"".to_string(),
            declaration: "Relationship(back_populates=\"posts\")".to_string(),
            is_forward_ref: true,
            referenced_class: "User".to_string(),
            referenced_module: None,
        };

        let line = format_relationship_line(&rel);
        assert_eq!(
            line,
            "    author: \"User\" = Relationship(back_populates=\"posts\")"
        );
    }
}
