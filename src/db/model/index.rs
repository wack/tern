//! Index definition types.

use serde::{Deserialize, Serialize};

use crate::db::schema::{ColumnName, IndexName, Oid};

use super::types::{Comment, IndexMethod, SqlExpr};

/// An index on a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    /// The index OID.
    pub oid: Oid,
    /// The index name.
    pub name: IndexName,
    /// The index access method (btree, hash, gin, etc.).
    pub method: IndexMethod,
    /// Whether this is a unique index.
    pub is_unique: bool,
    /// Whether this index backs a PRIMARY KEY or UNIQUE constraint.
    ///
    /// Constraint-backing indexes should not generate separate CREATE INDEX
    /// statements, as they are created implicitly by the constraint.
    pub is_constraint_index: bool,
    /// The indexed columns and/or expressions.
    pub columns: Vec<IndexColumn>,
    /// Partial index predicate (WHERE clause), if any.
    pub predicate: Option<SqlExpr>,
    /// Comment on this index, if any.
    pub comment: Option<Comment>,
}

/// A column or expression in an index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexColumn {
    /// The column name, if this is a simple column reference.
    /// None for expression indexes.
    pub column: Option<ColumnName>,
    /// For expression indexes, the expression text.
    /// Also populated for simple columns to capture any function application.
    pub expression: Option<SqlExpr>,
    /// Sort order for this column.
    pub order: SortOrder,
    /// Nulls ordering for this column.
    pub nulls: NullsOrder,
}

/// Sort order for an index column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// Ascending order (default).
    #[default]
    Ascending,
    /// Descending order.
    Descending,
}

impl SortOrder {
    /// Returns the SQL keyword for this sort order, or None for default (ASC).
    #[must_use]
    pub const fn as_sql(&self) -> Option<&'static str> {
        match self {
            Self::Ascending => None, // Default, no keyword needed
            Self::Descending => Some("DESC"),
        }
    }
}

/// Nulls ordering for an index column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NullsOrder {
    /// NULLS FIRST - nulls sort before non-null values.
    First,
    /// NULLS LAST - nulls sort after non-null values.
    Last,
}

impl NullsOrder {
    /// Returns the SQL clause for this nulls ordering, or None if it's the default
    /// for the given sort order (NULLS LAST for ASC, NULLS FIRST for DESC).
    #[must_use]
    pub const fn as_sql(&self, sort_order: SortOrder) -> Option<&'static str> {
        match (self, sort_order) {
            // NULLS LAST is default for ASC
            (Self::Last, SortOrder::Ascending) => None,
            // NULLS FIRST is default for DESC
            (Self::First, SortOrder::Descending) => None,
            // Non-default orderings need explicit clause
            (Self::First, SortOrder::Ascending) => Some("NULLS FIRST"),
            (Self::Last, SortOrder::Descending) => Some("NULLS LAST"),
        }
    }
}

impl Default for NullsOrder {
    /// Default is NULLS LAST (matching PostgreSQL's default for ASC).
    fn default() -> Self {
        Self::Last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_order_sql() {
        assert_eq!(SortOrder::Ascending.as_sql(), None);
        assert_eq!(SortOrder::Descending.as_sql(), Some("DESC"));
    }

    #[test]
    fn nulls_order_sql_defaults() {
        // Default combinations don't need explicit SQL
        assert_eq!(NullsOrder::Last.as_sql(SortOrder::Ascending), None);
        assert_eq!(NullsOrder::First.as_sql(SortOrder::Descending), None);
    }

    #[test]
    fn nulls_order_sql_non_defaults() {
        // Non-default combinations need explicit SQL
        assert_eq!(
            NullsOrder::First.as_sql(SortOrder::Ascending),
            Some("NULLS FIRST")
        );
        assert_eq!(
            NullsOrder::Last.as_sql(SortOrder::Descending),
            Some("NULLS LAST")
        );
    }

    #[test]
    fn index_construction() {
        let index = Index {
            oid: Oid::new(12345),
            name: IndexName::try_new("users_email_idx".to_string()).unwrap(),
            method: IndexMethod::BTree,
            is_unique: true,
            is_constraint_index: false,
            columns: vec![IndexColumn {
                column: Some(ColumnName::try_new("email".to_string()).unwrap()),
                expression: None,
                order: SortOrder::Ascending,
                nulls: NullsOrder::Last,
            }],
            predicate: None,
            comment: None,
        };

        assert!(index.is_unique);
        assert!(!index.is_constraint_index);
        assert_eq!(index.columns.len(), 1);
    }

    #[test]
    fn partial_index() {
        let index = Index {
            oid: Oid::new(12346),
            name: IndexName::try_new("orders_active_idx".to_string()).unwrap(),
            method: IndexMethod::BTree,
            is_unique: false,
            is_constraint_index: false,
            columns: vec![IndexColumn {
                column: Some(ColumnName::try_new("created_at".to_string()).unwrap()),
                expression: None,
                order: SortOrder::Descending,
                nulls: NullsOrder::First,
            }],
            predicate: Some(SqlExpr::new("status = 'active'".to_string())),
            comment: None,
        };

        assert!(index.predicate.is_some());
        assert_eq!(
            index.predicate.as_ref().unwrap().as_ref(),
            "status = 'active'"
        );
    }
}
