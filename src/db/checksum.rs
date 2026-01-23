//! Schema checksum computation.
//!
//! This module provides functions to compute deterministic xxhash3 checksums
//! from the domain model types. These checksums are used for schema drift
//! detection and are designed to be consistent across different execution
//! contexts (CLI, WASM, etc.).
//!
//! The checksum uses the `SchemaHasher` from `tern-migration-wit` to ensure
//! the same algorithm is used across all components.

use tern_migration_wit::checksum::SchemaHasher;

use super::model::constraint::ConstraintKind;
use super::model::{EnumType, Index, Namespace, Sequence, Table, View};

/// Computes an xxhash3 checksum of a namespace.
///
/// The checksum is computed by iterating through all schema objects in a
/// deterministic order (sorted by name) and feeding their structural
/// properties into the hasher.
///
/// # Returns
///
/// A 64-bit xxhash3 checksum as a hexadecimal string.
pub fn compute_schema_checksum(namespace: &Namespace) -> String {
    let mut hasher = SchemaHasher::new();

    hasher.add_namespace(namespace.name.as_ref());

    // Add enums (sorted by name for determinism)
    let mut enums: Vec<_> = namespace.enums.iter().collect();
    enums.sort_by_key(|e| e.name.as_ref());
    for enum_type in enums {
        add_enum_to_hasher(&mut hasher, enum_type);
    }

    // Add sequences (sorted by name for determinism)
    let mut sequences: Vec<_> = namespace.sequences.iter().collect();
    sequences.sort_by_key(|s| s.name.as_ref());
    for sequence in sequences {
        add_sequence_to_hasher(&mut hasher, sequence);
    }

    // Add tables (sorted by name for determinism)
    let mut tables: Vec<_> = namespace.tables.iter().collect();
    tables.sort_by_key(|t| t.name.as_ref());
    for table in tables {
        add_table_to_hasher(&mut hasher, table);
    }

    // Add views (sorted by name for determinism)
    let mut views: Vec<_> = namespace.views.iter().collect();
    views.sort_by_key(|v| v.name.as_ref());
    for view in views {
        add_view_to_hasher(&mut hasher, view);
    }

    hasher.end_namespace();
    hasher.finish_hex()
}

/// Computes an xxhash3 checksum and returns it as a u64.
pub fn compute_schema_checksum_u64(namespace: &Namespace) -> u64 {
    let mut hasher = SchemaHasher::new();

    hasher.add_namespace(namespace.name.as_ref());

    // Add enums (sorted by name for determinism)
    let mut enums: Vec<_> = namespace.enums.iter().collect();
    enums.sort_by_key(|e| e.name.as_ref());
    for enum_type in enums {
        add_enum_to_hasher(&mut hasher, enum_type);
    }

    // Add sequences (sorted by name for determinism)
    let mut sequences: Vec<_> = namespace.sequences.iter().collect();
    sequences.sort_by_key(|s| s.name.as_ref());
    for sequence in sequences {
        add_sequence_to_hasher(&mut hasher, sequence);
    }

    // Add tables (sorted by name for determinism)
    let mut tables: Vec<_> = namespace.tables.iter().collect();
    tables.sort_by_key(|t| t.name.as_ref());
    for table in tables {
        add_table_to_hasher(&mut hasher, table);
    }

    // Add views (sorted by name for determinism)
    let mut views: Vec<_> = namespace.views.iter().collect();
    views.sort_by_key(|v| v.name.as_ref());
    for view in views {
        add_view_to_hasher(&mut hasher, view);
    }

    hasher.end_namespace();
    hasher.finish()
}

fn add_enum_to_hasher(hasher: &mut SchemaHasher, enum_type: &EnumType) {
    let values: Vec<&str> = enum_type.values.iter().map(|s| s.as_str()).collect();
    hasher.add_enum(enum_type.name.as_ref(), &values);
}

fn add_sequence_to_hasher(hasher: &mut SchemaHasher, sequence: &Sequence) {
    hasher.add_sequence(
        sequence.name.as_ref(),
        &sequence.data_type.formatted,
        sequence.start_value,
        sequence.increment,
        sequence.min_value,
        sequence.max_value,
        sequence.cache_size,
        sequence.is_cyclic,
    );
}

fn add_table_to_hasher(hasher: &mut SchemaHasher, table: &Table) {
    let kind = match table.kind {
        super::model::table::TableKind::Regular => "regular",
        super::model::table::TableKind::Partitioned => "partitioned",
    };
    hasher.add_table(table.name.as_ref(), kind);

    // Add columns (in position order)
    let mut columns: Vec<_> = table.columns.iter().collect();
    columns.sort_by_key(|c| c.position);
    for column in columns {
        let identity = column.identity.map(|i| match i {
            super::model::column::IdentityKind::Always => "always",
            super::model::column::IdentityKind::ByDefault => "by_default",
        });
        let generated = column.generated.as_ref().map(|g| g.expression.as_ref());
        let collation = if column.collation.name.as_ref() == "default" {
            None
        } else {
            Some(column.collation.to_string())
        };

        hasher.add_column_full(
            column.name.as_ref(),
            &column.type_info.formatted,
            column.is_nullable,
            column.default.as_ref().map(|d| d.as_ref()),
            identity,
            generated,
            collation.as_deref(),
        );
    }

    // Add constraints (sorted by name for determinism)
    let mut constraints: Vec<_> = table.constraints.iter().collect();
    constraints.sort_by_key(|c| c.name.as_ref());
    for constraint in constraints {
        let (kind, definition) = match &constraint.kind {
            ConstraintKind::PrimaryKey(pk) => {
                let cols: Vec<&str> = pk.columns.iter().map(|c| c.as_ref()).collect();
                ("primary_key".to_string(), cols.join(", "))
            }
            ConstraintKind::ForeignKey(fk) => {
                let cols: Vec<&str> = fk.columns.iter().map(|c| c.as_ref()).collect();
                let ref_cols: Vec<&str> =
                    fk.referenced_columns.iter().map(|c| c.as_ref()).collect();
                let def = format!(
                    "({}) REFERENCES {} ({})",
                    cols.join(", "),
                    fk.referenced_table,
                    ref_cols.join(", ")
                );
                ("foreign_key".to_string(), def)
            }
            ConstraintKind::Unique(u) => {
                let cols: Vec<&str> = u.columns.iter().map(|c| c.as_ref()).collect();
                ("unique".to_string(), cols.join(", "))
            }
            ConstraintKind::Check(c) => ("check".to_string(), c.expression.as_ref().to_string()),
            ConstraintKind::Exclusion(e) => {
                let elements: Vec<String> = e
                    .elements
                    .iter()
                    .map(|el| format!("{} WITH {}", el.expression.as_ref(), el.operator))
                    .collect();
                ("exclusion".to_string(), elements.join(", "))
            }
        };
        hasher.add_constraint(constraint.name.as_ref(), &kind, &definition);
    }

    // Add indexes (sorted by name for determinism)
    let mut indexes: Vec<_> = table.indexes.iter().collect();
    indexes.sort_by_key(|i| i.name.as_ref());
    for index in indexes {
        add_index_to_hasher(hasher, index);
    }

    hasher.end_table();
}

fn add_index_to_hasher(hasher: &mut SchemaHasher, index: &Index) {
    let method = index.method.as_str();

    // Build the definition from columns
    let cols: Vec<String> = index
        .columns
        .iter()
        .map(|c| {
            let base = c
                .column
                .as_ref()
                .map(|col| col.as_ref().to_string())
                .or_else(|| c.expression.as_ref().map(|e| e.as_ref().to_string()))
                .unwrap_or_default();

            let mut parts = vec![base];
            if let Some(sql) = c.order.as_sql() {
                parts.push(sql.to_string());
            }
            if let Some(sql) = c.nulls.as_sql(c.order) {
                parts.push(sql.to_string());
            }
            parts.join(" ")
        })
        .collect();

    let mut definition = cols.join(", ");
    if let Some(predicate) = &index.predicate {
        definition.push_str(" WHERE ");
        definition.push_str(predicate.as_ref());
    }

    hasher.add_index(
        index.name.as_ref(),
        method,
        &definition,
        index.is_unique,
        index.is_constraint_index,
    );
}

fn add_view_to_hasher(hasher: &mut SchemaHasher, view: &View) {
    hasher.add_view(
        view.name.as_ref(),
        view.definition.as_ref(),
        view.is_materialized,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::column::{Column, IdentityKind};
    use crate::db::model::constraint::{Constraint, PrimaryKeyConstraint};
    use crate::db::model::table::TableKind;
    use crate::db::model::types::{QualifiedCollationName, TypeInfo};
    use crate::db::schema::{
        CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, TableName, TypeName,
    };

    fn make_collation() -> QualifiedCollationName {
        QualifiedCollationName::new(
            SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            CollationName::try_new("default".to_string()).unwrap(),
        )
    }

    fn make_type_info(formatted: &str) -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new("int4".to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: formatted.to_string(),
            is_array: false,
        }
    }

    #[test]
    fn empty_namespace_has_consistent_checksum() {
        let ns1 = Namespace::empty("public");
        let ns2 = Namespace::empty("public");

        let checksum1 = compute_schema_checksum(&ns1);
        let checksum2 = compute_schema_checksum(&ns2);

        assert_eq!(checksum1, checksum2);
        assert_eq!(checksum1.len(), 16); // 64-bit hash as hex
    }

    #[test]
    fn different_namespaces_have_different_checksums() {
        let ns1 = Namespace::empty("public");
        let ns2 = Namespace::empty("private");

        let checksum1 = compute_schema_checksum(&ns1);
        let checksum2 = compute_schema_checksum(&ns2);

        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn table_changes_checksum() {
        let ns1 = Namespace::empty("public");

        let mut ns2 = Namespace::empty("public");
        ns2.tables.push(Table {
            oid: Oid::new(1),
            name: TableName::try_new("users".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![Column {
                name: ColumnName::try_new("id".to_string()).unwrap(),
                position: 1,
                type_info: make_type_info("integer"),
                is_nullable: false,
                default: None,
                generated: None,
                identity: None,
                collation: make_collation(),
                comment: None,
            }],
            constraints: vec![Constraint {
                name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
                kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                    columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                    index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
                }),
                comment: None,
            }],
            indexes: vec![],
            comment: None,
        });

        let checksum1 = compute_schema_checksum(&ns1);
        let checksum2 = compute_schema_checksum(&ns2);

        assert_ne!(checksum1, checksum2);
    }

    #[test]
    fn column_type_change_affects_checksum() {
        let make_namespace = |col_type: &str| {
            let mut ns = Namespace::empty("public");
            ns.tables.push(Table {
                oid: Oid::new(1),
                name: TableName::try_new("users".to_string()).unwrap(),
                kind: TableKind::Regular,
                columns: vec![Column {
                    name: ColumnName::try_new("id".to_string()).unwrap(),
                    position: 1,
                    type_info: make_type_info(col_type),
                    is_nullable: false,
                    default: None,
                    generated: None,
                    identity: None,
                    collation: make_collation(),
                    comment: None,
                }],
                constraints: vec![],
                indexes: vec![],
                comment: None,
            });
            ns
        };

        let ns_int = make_namespace("integer");
        let ns_bigint = make_namespace("bigint");

        let checksum_int = compute_schema_checksum(&ns_int);
        let checksum_bigint = compute_schema_checksum(&ns_bigint);

        assert_ne!(checksum_int, checksum_bigint);
    }

    #[test]
    fn nullable_change_affects_checksum() {
        let make_namespace = |nullable: bool| {
            let mut ns = Namespace::empty("public");
            ns.tables.push(Table {
                oid: Oid::new(1),
                name: TableName::try_new("users".to_string()).unwrap(),
                kind: TableKind::Regular,
                columns: vec![Column {
                    name: ColumnName::try_new("name".to_string()).unwrap(),
                    position: 1,
                    type_info: make_type_info("text"),
                    is_nullable: nullable,
                    default: None,
                    generated: None,
                    identity: None,
                    collation: make_collation(),
                    comment: None,
                }],
                constraints: vec![],
                indexes: vec![],
                comment: None,
            });
            ns
        };

        let ns_nullable = make_namespace(true);
        let ns_not_nullable = make_namespace(false);

        let checksum_nullable = compute_schema_checksum(&ns_nullable);
        let checksum_not_nullable = compute_schema_checksum(&ns_not_nullable);

        assert_ne!(checksum_nullable, checksum_not_nullable);
    }

    #[test]
    fn identity_column_affects_checksum() {
        let make_namespace = |identity: Option<IdentityKind>| {
            let mut ns = Namespace::empty("public");
            ns.tables.push(Table {
                oid: Oid::new(1),
                name: TableName::try_new("users".to_string()).unwrap(),
                kind: TableKind::Regular,
                columns: vec![Column {
                    name: ColumnName::try_new("id".to_string()).unwrap(),
                    position: 1,
                    type_info: make_type_info("integer"),
                    is_nullable: false,
                    default: None,
                    generated: None,
                    identity,
                    collation: make_collation(),
                    comment: None,
                }],
                constraints: vec![],
                indexes: vec![],
                comment: None,
            });
            ns
        };

        let ns_no_identity = make_namespace(None);
        let ns_always = make_namespace(Some(IdentityKind::Always));
        let ns_by_default = make_namespace(Some(IdentityKind::ByDefault));

        let checksum_none = compute_schema_checksum(&ns_no_identity);
        let checksum_always = compute_schema_checksum(&ns_always);
        let checksum_by_default = compute_schema_checksum(&ns_by_default);

        assert_ne!(checksum_none, checksum_always);
        assert_ne!(checksum_none, checksum_by_default);
        assert_ne!(checksum_always, checksum_by_default);
    }

    #[test]
    fn checksum_is_deterministic() {
        let mut ns = Namespace::empty("public");
        ns.tables.push(Table {
            oid: Oid::new(1),
            name: TableName::try_new("users".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![
                Column {
                    name: ColumnName::try_new("id".to_string()).unwrap(),
                    position: 1,
                    type_info: make_type_info("integer"),
                    is_nullable: false,
                    default: None,
                    generated: None,
                    identity: Some(IdentityKind::Always),
                    collation: make_collation(),
                    comment: None,
                },
                Column {
                    name: ColumnName::try_new("name".to_string()).unwrap(),
                    position: 2,
                    type_info: make_type_info("text"),
                    is_nullable: true,
                    default: None,
                    generated: None,
                    identity: None,
                    collation: make_collation(),
                    comment: None,
                },
            ],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        });

        // Compute checksum multiple times
        let checksums: Vec<_> = (0..5).map(|_| compute_schema_checksum(&ns)).collect();

        // All checksums should be identical
        for checksum in &checksums[1..] {
            assert_eq!(&checksums[0], checksum);
        }
    }
}
