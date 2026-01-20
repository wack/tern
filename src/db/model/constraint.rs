//! Constraint definition types.

use serde::{Deserialize, Serialize};

use crate::db::schema::{ColumnName, ConstraintName, IndexName};

use super::types::{Comment, ForeignKeyAction, IndexMethod, QualifiedTableName, SqlExpr};

/// A table constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    /// The constraint name.
    pub name: ConstraintName,
    /// The specific kind of constraint with its associated data.
    pub kind: ConstraintKind,
    /// Comment on this constraint, if any.
    pub comment: Option<Comment>,
}

/// The specific kind of constraint with its associated data.
///
/// Each variant contains the data specific to that constraint type,
/// making invalid states unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConstraintKind {
    /// Primary key constraint.
    PrimaryKey(PrimaryKeyConstraint),
    /// Foreign key constraint.
    ForeignKey(ForeignKeyConstraint),
    /// Unique constraint.
    Unique(UniqueConstraint),
    /// Check constraint.
    Check(CheckConstraint),
    /// Exclusion constraint.
    Exclusion(ExclusionConstraint),
}

/// Primary key constraint data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryKeyConstraint {
    /// The columns that make up the primary key.
    pub columns: Vec<ColumnName>,
    /// The name of the index that backs this constraint.
    pub index_name: IndexName,
}

/// Foreign key constraint data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeyConstraint {
    /// The columns in this table that reference the foreign table.
    pub columns: Vec<ColumnName>,
    /// The referenced table (schema-qualified).
    pub referenced_table: QualifiedTableName,
    /// The columns in the referenced table.
    pub referenced_columns: Vec<ColumnName>,
    /// Action to take on DELETE of referenced row.
    pub on_delete: ForeignKeyAction,
    /// Action to take on UPDATE of referenced row.
    pub on_update: ForeignKeyAction,
    /// Whether this constraint is deferrable.
    pub is_deferrable: bool,
    /// Whether this constraint is initially deferred (only relevant if deferrable).
    pub is_initially_deferred: bool,
}

/// Unique constraint data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UniqueConstraint {
    /// The columns that must be unique together.
    pub columns: Vec<ColumnName>,
    /// The name of the index that backs this constraint.
    pub index_name: IndexName,
    /// Whether NULL values are treated as not distinct (PostgreSQL 15+).
    /// When true, only one NULL is allowed in the unique columns.
    pub nulls_not_distinct: bool,
}

/// Check constraint data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckConstraint {
    /// The check expression (e.g., "age >= 0").
    pub expression: SqlExpr,
    /// Whether this constraint is marked NO INHERIT.
    /// When true, child tables do not inherit this constraint.
    pub is_no_inherit: bool,
}

/// Exclusion constraint data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExclusionConstraint {
    /// The elements of the exclusion constraint (column/expression + operator pairs).
    pub elements: Vec<ExclusionElement>,
    /// The index method used (typically GiST or SP-GiST).
    pub index_method: IndexMethod,
    /// The name of the index that backs this constraint.
    pub index_name: IndexName,
    /// Optional WHERE clause limiting which rows are checked.
    pub predicate: Option<SqlExpr>,
}

/// An element of an exclusion constraint.
///
/// Each element specifies a column or expression and the operator used
/// for the exclusion check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExclusionElement {
    /// The column or expression being compared.
    pub expression: SqlExpr,
    /// The operator used for comparison (e.g., "=", "&&", "~=").
    pub operator: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::SchemaName;

    #[test]
    fn primary_key_constraint() {
        let pk = PrimaryKeyConstraint {
            columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
        };

        let constraint = Constraint {
            name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(pk),
            comment: None,
        };

        assert!(matches!(constraint.kind, ConstraintKind::PrimaryKey(_)));
    }

    #[test]
    fn foreign_key_constraint() {
        use crate::db::model::types::QualifiedName;
        use crate::db::schema::TableName;

        let fk = ForeignKeyConstraint {
            columns: vec![ColumnName::try_new("user_id".to_string()).unwrap()],
            referenced_table: QualifiedName::new(
                SchemaName::try_new("public".to_string()).unwrap(),
                TableName::try_new("users".to_string()).unwrap(),
            ),
            referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
            on_delete: ForeignKeyAction::Cascade,
            on_update: ForeignKeyAction::NoAction,
            is_deferrable: false,
            is_initially_deferred: false,
        };

        let constraint = Constraint {
            name: ConstraintName::try_new("orders_user_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(fk),
            comment: None,
        };

        if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
            assert_eq!(fk.on_delete, ForeignKeyAction::Cascade);
            assert_eq!(fk.referenced_table.to_string(), "public.users");
        } else {
            panic!("Expected ForeignKey constraint");
        }
    }

    #[test]
    fn check_constraint() {
        let check = CheckConstraint {
            expression: SqlExpr::new("age >= 0".to_string()),
            is_no_inherit: false,
        };

        let constraint = Constraint {
            name: ConstraintName::try_new("users_age_check".to_string()).unwrap(),
            kind: ConstraintKind::Check(check),
            comment: None,
        };

        if let ConstraintKind::Check(c) = &constraint.kind {
            assert_eq!(c.expression.as_ref(), "age >= 0");
        } else {
            panic!("Expected Check constraint");
        }
    }
}
