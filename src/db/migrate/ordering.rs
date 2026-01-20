//! Topological sorting of operations based on dependencies.
//!
//! This module provides functions to sort migration operations into an order
//! that respects dependencies between database objects.

use std::collections::HashMap;

use super::operation::{ObjectKind, Operation};
use crate::db::model::constraint::ConstraintKind;
use crate::db::schema::{SchemaName, TableName};

/// Sort operations to respect dependencies.
///
/// The algorithm:
/// 1. Group operations by phase (drops, creates, alters, renames, comments)
/// 2. Within drops, sort by reverse dependency order (views before tables, etc.)
/// 3. Within creates, sort by dependency order (enums before tables, etc.)
/// 4. Within creates, respect FK dependencies between tables
/// 5. Combine all phases in order: drops, renames, creates, alters, comments
pub fn topological_sort(operations: Vec<Operation>) -> Vec<Operation> {
    let mut drops = Vec::new();
    let mut creates = Vec::new();
    let mut alters = Vec::new();
    let mut renames = Vec::new();
    let mut comments = Vec::new();

    // Partition by operation type
    for op in operations {
        match &op {
            Operation::DropTable { .. }
            | Operation::DropEnum { .. }
            | Operation::DropSequence { .. }
            | Operation::DropView { .. }
            | Operation::DropIndex { .. }
            | Operation::DropColumn { .. }
            | Operation::DropConstraint { .. } => drops.push(op),

            Operation::CreateTable { .. }
            | Operation::CreateEnum { .. }
            | Operation::CreateSequence { .. }
            | Operation::CreateView { .. }
            | Operation::CreateIndex { .. }
            | Operation::AddColumn { .. }
            | Operation::AddConstraint { .. }
            | Operation::AddEnumValue { .. } => creates.push(op),

            Operation::RenameTable { .. }
            | Operation::RenameEnum { .. }
            | Operation::RenameSequence { .. }
            | Operation::RenameView { .. }
            | Operation::RenameIndex { .. }
            | Operation::RenameColumn { .. }
            | Operation::RenameConstraint { .. } => renames.push(op),

            Operation::AlterColumn { .. }
            | Operation::AlterSequence { .. }
            | Operation::ReplaceView { .. }
            | Operation::RefreshMaterializedView { .. } => alters.push(op),

            Operation::SetComment { .. } => comments.push(op),
        }
    }

    // Sort drops in reverse dependency order
    // (views before tables, constraints before tables, indexes before tables, etc.)
    drops.sort_by_key(|op| std::cmp::Reverse(op.object_kind()));

    // Sort renames by object kind
    renames.sort_by_key(|op| op.object_kind());

    // Sort creates in dependency order (with FK awareness)
    let creates = sort_creates_by_dependency(creates);

    // Alters are generally order-independent, but sort by object kind for consistency
    alters.sort_by_key(|op| op.object_kind());

    // Combine: drops first, then renames, then creates, then alters, then comments
    let mut result = Vec::with_capacity(
        drops.len() + renames.len() + creates.len() + alters.len() + comments.len(),
    );
    result.extend(drops);
    result.extend(renames);
    result.extend(creates);
    result.extend(alters);
    result.extend(comments);

    result
}

/// Sort create operations respecting foreign key dependencies.
fn sort_creates_by_dependency(operations: Vec<Operation>) -> Vec<Operation> {
    // First, partition into tables, constraints/indexes, and other creates
    let mut enums_and_sequences = Vec::new();
    let mut tables = Vec::new();
    let mut constraints_and_indexes = Vec::new();
    let mut views = Vec::new();
    let mut other = Vec::new();

    for op in operations {
        match op.object_kind() {
            ObjectKind::Enum | ObjectKind::Sequence => enums_and_sequences.push(op),
            ObjectKind::Table => {
                if matches!(&op, Operation::CreateTable { .. }) {
                    tables.push(op);
                } else {
                    // AddColumn operations
                    other.push(op);
                }
            }
            ObjectKind::Constraint | ObjectKind::Index => constraints_and_indexes.push(op),
            ObjectKind::View => views.push(op),
            ObjectKind::Comment => {
                // Comments shouldn't be in creates, but handle gracefully
                other.push(op);
            }
        }
    }

    // Sort enums and sequences by kind
    enums_and_sequences.sort_by_key(|op| op.object_kind());

    // Sort tables with FK awareness
    let tables = sort_tables_by_fk_dependencies(tables);

    // Constraints and indexes go after their tables
    // Sort by object kind (constraints before indexes)
    constraints_and_indexes.sort_by_key(|op| op.object_kind());

    // Views depend on tables, so they go last
    // Views could have dependencies on each other, but we don't track that

    // Combine in order
    let mut result = Vec::with_capacity(
        enums_and_sequences.len()
            + tables.len()
            + other.len()
            + constraints_and_indexes.len()
            + views.len(),
    );
    result.extend(enums_and_sequences);
    result.extend(tables);
    result.extend(other);
    result.extend(constraints_and_indexes);
    result.extend(views);

    result
}

/// Sort table creation operations respecting FK dependencies.
///
/// Uses a simple topological sort based on FK references.
/// Tables that are referenced by FKs must be created before the referencing tables.
fn sort_tables_by_fk_dependencies(operations: Vec<Operation>) -> Vec<Operation> {
    if operations.is_empty() {
        return operations;
    }

    // Build a map of table names to their operations
    let mut table_ops: HashMap<(SchemaName, TableName), Operation> = HashMap::new();
    let mut fk_deps: Vec<((SchemaName, TableName), (SchemaName, TableName))> = Vec::new();

    for op in operations {
        if let Operation::CreateTable { schema, table } = &op {
            let key = (schema.clone(), table.name.clone());

            // Find FK dependencies
            for constraint in &table.constraints {
                if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                    fk_deps.push((
                        key.clone(),
                        (
                            fk.referenced_table.schema.clone(),
                            fk.referenced_table.name.clone(),
                        ),
                    ));
                }
            }

            table_ops.insert(key, op);
        }
    }

    // Simple approach: assign order numbers based on FK depth
    let mut order: HashMap<(SchemaName, TableName), usize> = HashMap::new();
    for key in table_ops.keys() {
        order.insert(key.clone(), 0);
    }

    // Iterate to propagate dependency depths
    // A table that references another table must have a higher order number
    let mut changed = true;
    let max_iterations = table_ops.len() + 1; // Prevent infinite loops
    let mut iteration = 0;

    while changed && iteration < max_iterations {
        changed = false;
        iteration += 1;

        for (from, to) in &fk_deps {
            if let (Some(&from_order), Some(&to_order)) = (order.get(from), order.get(to)) {
                // The referencing table must come after the referenced table
                if from_order <= to_order {
                    order.insert(from.clone(), to_order + 1);
                    changed = true;
                }
            }
        }
    }

    // Sort by order number
    let mut sorted_keys: Vec<_> = table_ops.keys().cloned().collect();
    sorted_keys.sort_by_key(|k| order.get(k).copied().unwrap_or(0));

    // Collect operations in sorted order
    sorted_keys
        .into_iter()
        .filter_map(|key| table_ops.remove(&key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::constraint::{Constraint, ForeignKeyConstraint};
    use crate::db::model::types::{
        ForeignKeyAction, QualifiedTableName,
    };
    use crate::db::model::{EnumType, Table, TableKind};
    use crate::db::schema::{
        ColumnName, ConstraintName, Oid, TableName, TypeName,
    };

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn simple_table(name: &str) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        }
    }

    fn table_with_fk(name: &str, refs_schema: &str, refs_table: &str) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![Constraint {
                name: ConstraintName::try_new(format!("{}_fk", name)).unwrap(),
                kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                    columns: vec![ColumnName::try_new("ref_id".to_string()).unwrap()],
                    referenced_table: QualifiedTableName::new(
                        SchemaName::try_new(refs_schema.to_string()).unwrap(),
                        TableName::try_new(refs_table.to_string()).unwrap(),
                    ),
                    referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                    on_delete: ForeignKeyAction::NoAction,
                    on_update: ForeignKeyAction::NoAction,
                    is_deferrable: false,
                    is_initially_deferred: false,
                }),
                comment: None,
            }],
            indexes: vec![],
            comment: None,
        }
    }

    fn simple_enum(name: &str) -> EnumType {
        EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new(name.to_string()).unwrap(),
            values: vec!["a".to_string(), "b".to_string()],
            comment: None,
        }
    }

    #[test]
    fn empty_operations() {
        let ops = vec![];
        let sorted = topological_sort(ops);
        assert!(sorted.is_empty());
    }

    #[test]
    fn drops_before_creates() {
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("new_table"),
            },
            Operation::DropTable {
                schema: test_schema(),
                name: TableName::try_new("old_table".to_string()).unwrap(),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 2);
        assert!(matches!(&sorted[0], Operation::DropTable { .. }));
        assert!(matches!(&sorted[1], Operation::CreateTable { .. }));
    }

    #[test]
    fn enums_before_tables() {
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("users"),
            },
            Operation::CreateEnum {
                schema: test_schema(),
                enum_type: simple_enum("status"),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 2);
        assert!(matches!(&sorted[0], Operation::CreateEnum { .. }));
        assert!(matches!(&sorted[1], Operation::CreateTable { .. }));
    }

    #[test]
    fn fk_dependent_tables_ordered() {
        let ops = vec![
            // orders references users
            Operation::CreateTable {
                schema: test_schema(),
                table: table_with_fk("orders", "public", "users"),
            },
            // users has no dependencies
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("users"),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 2);

        // users should come before orders
        if let (
            Operation::CreateTable { table: t1, .. },
            Operation::CreateTable { table: t2, .. },
        ) = (&sorted[0], &sorted[1])
        {
            assert_eq!(t1.name.as_ref(), "users");
            assert_eq!(t2.name.as_ref(), "orders");
        } else {
            panic!("Expected CreateTable operations");
        }
    }

    #[test]
    fn chain_of_fk_dependencies() {
        // C -> B -> A
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: table_with_fk("c", "public", "b"),
            },
            Operation::CreateTable {
                schema: test_schema(),
                table: table_with_fk("b", "public", "a"),
            },
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("a"),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 3);

        // Order should be: a, b, c
        let names: Vec<_> = sorted
            .iter()
            .filter_map(|op| {
                if let Operation::CreateTable { table, .. } = op {
                    Some(table.name.as_ref().to_string())
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn drops_in_reverse_dependency_order() {
        // When dropping, objects with higher ObjectKind values drop first
        // ObjectKind order: Enum < Sequence < Table < View < Index < Constraint < Comment
        // So drops go in reverse: Index, View, Table
        let ops = vec![
            Operation::DropTable {
                schema: test_schema(),
                name: TableName::try_new("table1".to_string()).unwrap(),
            },
            Operation::DropView {
                schema: test_schema(),
                name: TableName::try_new("view1".to_string()).unwrap(),
                is_materialized: false,
            },
            Operation::DropIndex {
                schema: test_schema(),
                name: crate::db::schema::IndexName::try_new("idx1".to_string()).unwrap(),
                concurrently: false,
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 3);

        // Index has highest ObjectKind value, so drops first (reverse order)
        // Then view, then table
        assert!(matches!(&sorted[0], Operation::DropIndex { .. }));
        assert!(matches!(&sorted[1], Operation::DropView { .. }));
        assert!(matches!(&sorted[2], Operation::DropTable { .. }));
    }

    #[test]
    fn comments_come_last() {
        let ops = vec![
            Operation::SetComment {
                target: crate::db::migrate::operation::CommentTarget::Table {
                    schema: test_schema(),
                    table: TableName::try_new("users".to_string()).unwrap(),
                },
                comment: Some("User table".to_string()),
            },
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("users"),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 2);
        assert!(matches!(&sorted[0], Operation::CreateTable { .. }));
        assert!(matches!(&sorted[1], Operation::SetComment { .. }));
    }

    #[test]
    fn renames_between_drops_and_creates() {
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("new_table"),
            },
            Operation::RenameTable {
                schema: test_schema(),
                from: TableName::try_new("old_name".to_string()).unwrap(),
                to: TableName::try_new("new_name".to_string()).unwrap(),
            },
            Operation::DropTable {
                schema: test_schema(),
                name: TableName::try_new("deleted_table".to_string()).unwrap(),
            },
        ];

        let sorted = topological_sort(ops);
        assert_eq!(sorted.len(), 3);

        // Order: drop, rename, create
        assert!(matches!(&sorted[0], Operation::DropTable { .. }));
        assert!(matches!(&sorted[1], Operation::RenameTable { .. }));
        assert!(matches!(&sorted[2], Operation::CreateTable { .. }));
    }
}
