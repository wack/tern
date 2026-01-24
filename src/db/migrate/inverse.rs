//! Inverse operation computation for migration reversal.
//!
//! This module provides functions to compute the inverse (reverse) operations
//! needed to revert a migration. Inverse operations are computed at migration
//! generation time and stored alongside the forward operations.

use super::Operation;

/// Error type for inverse operation computation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum InverseError {
    /// The operation cannot be reversed.
    #[error("{0}")]
    Irreversible(String),
}

/// Result of computing inverse operations.
#[derive(Debug, Clone)]
pub struct InverseResult {
    /// The computed inverse operations, in reverse order.
    pub operations: Vec<Operation>,

    /// Warnings about operations that could not be reversed.
    /// If this is non-empty, the `operations` list will be empty.
    pub errors: Vec<InverseError>,
}

impl InverseResult {
    /// Returns true if all operations were successfully inverted.
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns true if any operations could not be inverted.
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// Computes the inverse operations needed to revert a migration.
///
/// Returns the operations in reverse order (last applied first reverted).
/// If any operation is irreversible, returns an empty operations list with errors.
pub fn compute_inverse_operations(operations: &[Operation]) -> InverseResult {
    let mut inverse_ops = Vec::new();
    let mut errors = Vec::new();

    // Process operations in reverse order
    for op in operations.iter().rev() {
        match compute_single_inverse(op) {
            Ok(inverse) => inverse_ops.push(inverse),
            Err(e) => errors.push(e),
        }
    }

    // If there were any errors, return empty operations
    if !errors.is_empty() {
        InverseResult {
            operations: vec![],
            errors,
        }
    } else {
        InverseResult {
            operations: inverse_ops,
            errors: vec![],
        }
    }
}

/// Computes the inverse of a single operation.
fn compute_single_inverse(op: &Operation) -> Result<Operation, InverseError> {
    match op {
        // Enum operations
        Operation::CreateEnum { schema, enum_type } => Ok(Operation::DropEnum {
            schema: schema.clone(),
            name: enum_type.name.clone(),
        }),
        Operation::DropEnum { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropEnum: enum definition is not preserved".to_string(),
        )),
        Operation::RenameEnum { schema, from, to } => Ok(Operation::RenameEnum {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AddEnumValue { .. } => Err(InverseError::Irreversible(
            "Cannot revert AddEnumValue: PostgreSQL does not support removing enum values"
                .to_string(),
        )),

        // Sequence operations
        Operation::CreateSequence { schema, sequence } => Ok(Operation::DropSequence {
            schema: schema.clone(),
            name: sequence.name.clone(),
        }),
        Operation::DropSequence { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropSequence: sequence definition is not preserved".to_string(),
        )),
        Operation::RenameSequence { schema, from, to } => Ok(Operation::RenameSequence {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AlterSequence { .. } => Err(InverseError::Irreversible(
            "Cannot revert AlterSequence: previous values are not preserved".to_string(),
        )),

        // Table operations
        Operation::CreateTable { schema, table } => Ok(Operation::DropTable {
            schema: schema.clone(),
            name: table.name.clone(),
        }),
        Operation::DropTable { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropTable: table definition and data are not preserved".to_string(),
        )),
        Operation::RenameTable { schema, from, to } => Ok(Operation::RenameTable {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // Column operations
        Operation::AddColumn {
            schema,
            table,
            column,
        } => Ok(Operation::DropColumn {
            schema: schema.clone(),
            table: table.clone(),
            name: column.name.clone(),
        }),
        Operation::DropColumn { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropColumn: column definition and data are not preserved".to_string(),
        )),
        Operation::RenameColumn {
            schema,
            table,
            from,
            to,
        } => Ok(Operation::RenameColumn {
            schema: schema.clone(),
            table: table.clone(),
            from: to.clone(),
            to: from.clone(),
        }),
        Operation::AlterColumn { .. } => Err(InverseError::Irreversible(
            "Cannot revert AlterColumn: previous values are not preserved".to_string(),
        )),

        // Constraint operations
        Operation::AddConstraint {
            schema,
            table,
            constraint,
        } => Ok(Operation::DropConstraint {
            schema: schema.clone(),
            table: table.clone(),
            name: constraint.name.clone(),
        }),
        Operation::DropConstraint { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropConstraint: constraint definition is not preserved".to_string(),
        )),
        Operation::RenameConstraint {
            schema,
            table,
            from,
            to,
        } => Ok(Operation::RenameConstraint {
            schema: schema.clone(),
            table: table.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // Index operations
        Operation::CreateIndex {
            schema,
            index,
            concurrently,
            ..
        } => Ok(Operation::DropIndex {
            schema: schema.clone(),
            name: index.name.clone(),
            concurrently: *concurrently,
        }),
        Operation::DropIndex { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropIndex: index definition is not preserved".to_string(),
        )),
        Operation::RenameIndex { schema, from, to } => Ok(Operation::RenameIndex {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
        }),

        // View operations
        Operation::CreateView { schema, view } => Ok(Operation::DropView {
            schema: schema.clone(),
            name: view.name.clone(),
            is_materialized: view.is_materialized,
        }),
        Operation::DropView { .. } => Err(InverseError::Irreversible(
            "Cannot revert DropView: view definition is not preserved".to_string(),
        )),
        Operation::RenameView {
            schema,
            from,
            to,
            is_materialized,
        } => Ok(Operation::RenameView {
            schema: schema.clone(),
            from: to.clone(),
            to: from.clone(),
            is_materialized: *is_materialized,
        }),
        Operation::ReplaceView { .. } => Err(InverseError::Irreversible(
            "Cannot revert ReplaceView: previous view definition is not preserved".to_string(),
        )),
        Operation::RefreshMaterializedView { .. } => Err(InverseError::Irreversible(
            "Cannot revert RefreshMaterializedView: this operation cannot be undone".to_string(),
        )),

        // Comment operations
        Operation::SetComment { .. } => Err(InverseError::Irreversible(
            "Cannot revert SetComment: previous comment is not preserved".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::{EnumType, Table, TableKind};
    use crate::db::schema::{Oid, SchemaName, TableName, TypeName};

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn test_table_name() -> TableName {
        TableName::try_new("users".to_string()).unwrap()
    }

    fn test_table() -> Table {
        Table {
            oid: Oid::new(1),
            name: test_table_name(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        }
    }

    fn test_enum() -> EnumType {
        EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new("status".to_string()).unwrap(),
            values: vec!["active".to_string(), "inactive".to_string()],
            comment: None,
        }
    }

    #[test]
    fn create_table_inverse_is_drop_table() {
        let op = Operation::CreateTable {
            schema: test_schema(),
            table: test_table(),
        };
        let result = compute_inverse_operations(&[op]);

        assert!(result.is_success());
        assert_eq!(result.operations.len(), 1);
        assert!(matches!(
            &result.operations[0],
            Operation::DropTable { schema, name }
            if schema.as_ref() == "public" && name.as_ref() == "users"
        ));
    }

    #[test]
    fn drop_table_is_irreversible() {
        let op = Operation::DropTable {
            schema: test_schema(),
            name: test_table_name(),
        };
        let result = compute_inverse_operations(&[op]);

        assert!(result.has_errors());
        assert!(result.operations.is_empty());
        assert_eq!(result.errors.len(), 1);
    }

    #[test]
    fn rename_table_swaps_names() {
        let op = Operation::RenameTable {
            schema: test_schema(),
            from: TableName::try_new("old_name".to_string()).unwrap(),
            to: TableName::try_new("new_name".to_string()).unwrap(),
        };
        let result = compute_inverse_operations(&[op]);

        assert!(result.is_success());
        assert!(matches!(
            &result.operations[0],
            Operation::RenameTable { from, to, .. }
            if from.as_ref() == "new_name" && to.as_ref() == "old_name"
        ));
    }

    #[test]
    fn create_enum_inverse_is_drop_enum() {
        let op = Operation::CreateEnum {
            schema: test_schema(),
            enum_type: test_enum(),
        };
        let result = compute_inverse_operations(&[op]);

        assert!(result.is_success());
        assert!(matches!(
            &result.operations[0],
            Operation::DropEnum { name, .. }
            if name.as_ref() == "status"
        ));
    }

    #[test]
    fn add_enum_value_is_irreversible() {
        let op = Operation::AddEnumValue {
            schema: test_schema(),
            enum_name: TypeName::try_new("status".to_string()).unwrap(),
            value: "pending".to_string(),
            position: crate::db::migrate::EnumValuePosition::End,
        };
        let result = compute_inverse_operations(&[op]);

        assert!(result.has_errors());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn multiple_operations_reversed_in_order() {
        let ops = vec![
            Operation::CreateEnum {
                schema: test_schema(),
                enum_type: test_enum(),
            },
            Operation::CreateTable {
                schema: test_schema(),
                table: test_table(),
            },
        ];
        let result = compute_inverse_operations(&ops);

        assert!(result.is_success());
        assert_eq!(result.operations.len(), 2);

        // Order should be reversed: DropTable first, then DropEnum
        assert!(matches!(&result.operations[0], Operation::DropTable { .. }));
        assert!(matches!(&result.operations[1], Operation::DropEnum { .. }));
    }

    #[test]
    fn mixed_reversible_irreversible_fails() {
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: test_table(),
            },
            Operation::DropTable {
                schema: test_schema(),
                name: TableName::try_new("other".to_string()).unwrap(),
            },
        ];
        let result = compute_inverse_operations(&ops);

        // Should fail because DropTable is irreversible
        assert!(result.has_errors());
        assert!(result.operations.is_empty());
    }

    #[test]
    fn empty_operations_returns_empty() {
        let result = compute_inverse_operations(&[]);
        assert!(result.is_success());
        assert!(result.operations.is_empty());
    }
}
