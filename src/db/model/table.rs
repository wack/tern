//! Table definition types.

use serde::{Deserialize, Serialize};

use crate::db::schema::{Oid, TableName};

use super::column::Column;
use super::constraint::Constraint;
use super::index::Index;
use super::types::Comment;

/// A table definition.
///
/// This represents a regular table or partitioned table (but not the
/// partition metadata itself, which is out of scope for now).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// The table OID.
    pub oid: Oid,
    /// The table name.
    pub name: TableName,
    /// The kind of table (regular or partitioned).
    pub kind: TableKind,
    /// The columns in this table, ordered by position.
    pub columns: Vec<Column>,
    /// The constraints on this table.
    pub constraints: Vec<Constraint>,
    /// The indexes on this table.
    ///
    /// Note: Indexes that back constraints (primary key, unique) are included here
    /// with `is_constraint_index = true`. They should not generate separate
    /// CREATE INDEX statements.
    pub indexes: Vec<Index>,
    /// Comment on this table, if any.
    pub comment: Option<Comment>,
}

/// The kind of table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TableKind {
    /// A regular table (`'r'`).
    #[default]
    Regular,
    /// A partitioned table (`'p'`).
    Partitioned,
}

impl TableKind {
    /// Returns the single-character code used by PostgreSQL in `pg_class.relkind`.
    #[must_use]
    pub const fn as_char(&self) -> char {
        match self {
            Self::Regular => 'r',
            Self::Partitioned => 'p',
        }
    }
}

/// Error returned when parsing an invalid table kind character.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid table kind: '{0}'")]
pub struct InvalidTableKind(pub char);

impl TryFrom<char> for TableKind {
    type Error = InvalidTableKind;

    fn try_from(c: char) -> Result<Self, Self::Error> {
        match c {
            'r' => Ok(Self::Regular),
            'p' => Ok(Self::Partitioned),
            _ => Err(InvalidTableKind(c)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_kind_roundtrip() {
        for kind in [TableKind::Regular, TableKind::Partitioned] {
            let c = kind.as_char();
            let parsed = TableKind::try_from(c).unwrap();
            assert_eq!(kind, parsed);
        }
    }

    #[test]
    fn table_kind_invalid() {
        assert!(TableKind::try_from('v').is_err()); // 'v' is for views
    }

    #[test]
    fn empty_table() {
        let table = Table {
            oid: Oid::new(12345),
            name: TableName::try_new("empty_table".to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        };

        assert_eq!(table.columns.len(), 0);
        assert_eq!(table.kind, TableKind::Regular);
    }
}
