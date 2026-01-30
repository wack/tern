//! PostgreSQL schema types for catalog introspection.
//!
//! This module provides type-safe wrappers around PostgreSQL catalog concepts,
//! ensuring that OIDs, names, and catalog codes are used correctly throughout
//! the codebase.
//!
//! Basic schema types (Oid, name types) are re-exported from `tern_ddl`.
//! Additional catalog introspection types (RelationKind, ConstraintType) are
//! defined here.

use serde::{Deserialize, Serialize};

// Re-export basic schema types from tern-ddl
pub use tern_ddl::schema::{
    CollationName, CollationNameError, ColumnName, ColumnNameError, ConstraintName,
    ConstraintNameError, IndexName, IndexNameError, Oid, SchemaName, SchemaNameError, SequenceName,
    SequenceNameError, TableName, TableNameError, TypeName, TypeNameError,
};

// =============================================================================
// Relation Kind (pg_class.relkind)
// =============================================================================

/// The kind of relation stored in `pg_class`.
///
/// PostgreSQL stores this as a single character in `pg_class.relkind`.
/// This enum provides type-safe access to the most common relation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Ordinary table (`'r'`)
    Table,
    /// Index (`'i'`)
    Index,
    /// Sequence (`'S'`)
    Sequence,
    /// TOAST table (`'t'`)
    Toast,
    /// View (`'v'`)
    View,
    /// Materialized view (`'m'`)
    MaterializedView,
    /// Composite type (`'c'`)
    CompositeType,
    /// Foreign table (`'f'`)
    ForeignTable,
    /// Partitioned table (`'p'`)
    PartitionedTable,
    /// Partitioned index (`'I'`)
    PartitionedIndex,
}

/// Error returned when parsing an invalid relation kind character.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid relation kind: '{0}'")]
pub struct InvalidRelationKind(pub char);

crate::impl_char_enum!(RelationKind, InvalidRelationKind, [
    Table => 'r',
    Index => 'i',
    Sequence => 'S',
    Toast => 't',
    View => 'v',
    MaterializedView => 'm',
    CompositeType => 'c',
    ForeignTable => 'f',
    PartitionedTable => 'p',
    PartitionedIndex => 'I',
]);

// =============================================================================
// Constraint Type (pg_constraint.contype)
// =============================================================================

/// The type of constraint stored in `pg_constraint`.
///
/// PostgreSQL stores this as a single character in `pg_constraint.contype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintType {
    /// Check constraint (`'c'`)
    Check,
    /// Foreign key constraint (`'f'`)
    ForeignKey,
    /// Primary key constraint (`'p'`)
    PrimaryKey,
    /// Unique constraint (`'u'`)
    Unique,
    /// Exclusion constraint (`'x'`)
    Exclusion,
}

/// Error returned when parsing an invalid constraint type character.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid constraint type: '{0}'")]
pub struct InvalidConstraintType(pub char);

crate::impl_char_enum!(ConstraintType, InvalidConstraintType, [
    Check => 'c',
    ForeignKey => 'f',
    PrimaryKey => 'p',
    Unique => 'u',
    Exclusion => 'x',
]);

#[cfg(test)]
mod tests {
    use super::*;

    mod relation_kind_tests {
        use super::*;

        #[test]
        fn relation_kind_from_char() {
            assert_eq!(RelationKind::try_from('r'), Ok(RelationKind::Table));
            assert_eq!(RelationKind::try_from('i'), Ok(RelationKind::Index));
            assert_eq!(RelationKind::try_from('S'), Ok(RelationKind::Sequence));
            assert_eq!(RelationKind::try_from('t'), Ok(RelationKind::Toast));
            assert_eq!(RelationKind::try_from('v'), Ok(RelationKind::View));
            assert_eq!(
                RelationKind::try_from('m'),
                Ok(RelationKind::MaterializedView)
            );
            assert_eq!(RelationKind::try_from('c'), Ok(RelationKind::CompositeType));
            assert_eq!(RelationKind::try_from('f'), Ok(RelationKind::ForeignTable));
            assert_eq!(
                RelationKind::try_from('p'),
                Ok(RelationKind::PartitionedTable)
            );
            assert_eq!(
                RelationKind::try_from('I'),
                Ok(RelationKind::PartitionedIndex)
            );
        }

        #[test]
        fn relation_kind_to_char() {
            assert_eq!(RelationKind::Table.as_char(), 'r');
            assert_eq!(RelationKind::Index.as_char(), 'i');
            assert_eq!(RelationKind::Sequence.as_char(), 'S');
            assert_eq!(RelationKind::PartitionedTable.as_char(), 'p');
        }

        #[test]
        fn relation_kind_invalid_char() {
            let result = RelationKind::try_from('z');
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), InvalidRelationKind('z'));
        }

        #[test]
        fn relation_kind_roundtrip() {
            crate::assert_enum_char_roundtrip!(
                RelationKind,
                [
                    RelationKind::Table,
                    RelationKind::Index,
                    RelationKind::Sequence,
                    RelationKind::Toast,
                    RelationKind::View,
                    RelationKind::MaterializedView,
                    RelationKind::CompositeType,
                    RelationKind::ForeignTable,
                    RelationKind::PartitionedTable,
                    RelationKind::PartitionedIndex,
                ]
            );
        }
    }

    mod constraint_type_tests {
        use super::*;

        #[test]
        fn constraint_type_from_char() {
            assert_eq!(ConstraintType::try_from('c'), Ok(ConstraintType::Check));
            assert_eq!(
                ConstraintType::try_from('f'),
                Ok(ConstraintType::ForeignKey)
            );
            assert_eq!(
                ConstraintType::try_from('p'),
                Ok(ConstraintType::PrimaryKey)
            );
            assert_eq!(ConstraintType::try_from('u'), Ok(ConstraintType::Unique));
            assert_eq!(ConstraintType::try_from('x'), Ok(ConstraintType::Exclusion));
        }

        #[test]
        fn constraint_type_to_char() {
            assert_eq!(ConstraintType::Check.as_char(), 'c');
            assert_eq!(ConstraintType::ForeignKey.as_char(), 'f');
            assert_eq!(ConstraintType::PrimaryKey.as_char(), 'p');
            assert_eq!(ConstraintType::Unique.as_char(), 'u');
            assert_eq!(ConstraintType::Exclusion.as_char(), 'x');
        }

        #[test]
        fn constraint_type_invalid_char() {
            let result = ConstraintType::try_from('z');
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), InvalidConstraintType('z'));
        }

        #[test]
        fn constraint_type_roundtrip() {
            crate::assert_enum_char_roundtrip!(
                ConstraintType,
                [
                    ConstraintType::Check,
                    ConstraintType::ForeignKey,
                    ConstraintType::PrimaryKey,
                    ConstraintType::Unique,
                    ConstraintType::Exclusion,
                ]
            );
        }
    }
}
