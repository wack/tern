//! PostgreSQL schema types for catalog introspection.
//!
//! This module provides type-safe wrappers around PostgreSQL catalog concepts,
//! ensuring that OIDs, names, and catalog codes are used correctly throughout
//! the codebase.

use nutype::nutype;
use serde::{Deserialize, Serialize};

// =============================================================================
// Test Macros for Enum Roundtrip Conversions
// =============================================================================

/// Tests that an enum variant can be converted to a character and parsed back.
///
/// # Example
///
/// ```ignore
/// assert_enum_char_roundtrip!(RelationKind, [
///     RelationKind::Table,
///     RelationKind::Index,
///     RelationKind::Sequence,
/// ]);
/// ```
#[macro_export]
macro_rules! assert_enum_char_roundtrip {
    ($enum_type:ty, [$($variant:expr),+ $(,)?]) => {
        {
            $(
                let c = $variant.as_char();
                let parsed = <$enum_type>::try_from(c).unwrap();
                assert_eq!($variant, parsed, "Roundtrip failed for {:?}", $variant);
            )+
        }
    };
}

// =============================================================================
// OID (Object Identifier)
// =============================================================================

/// PostgreSQL Object Identifier.
///
/// OIDs are used throughout the PostgreSQL catalog to uniquely identify
/// database objects (tables, columns, types, etc.). They are unsigned 32-bit
/// integers internally.
#[nutype(
    default = 0,
    derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Default,
        Serialize,
        Deserialize,
        AsRef,
        Into
    )
)]
pub struct Oid(u32);

// =============================================================================
// Name Types
// =============================================================================

/// A PostgreSQL schema (namespace) name.
///
/// Corresponds to `pg_namespace.nspname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct SchemaName(String);

/// A PostgreSQL table (relation) name.
///
/// Corresponds to `pg_class.relname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct TableName(String);

/// A PostgreSQL column (attribute) name.
///
/// Corresponds to `pg_attribute.attname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct ColumnName(String);

/// A PostgreSQL constraint name.
///
/// Corresponds to `pg_constraint.conname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct ConstraintName(String);

/// A PostgreSQL index name.
///
/// Corresponds to the `relname` of an index in `pg_class`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct IndexName(String);

/// A PostgreSQL type name.
///
/// Corresponds to `pg_type.typname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct TypeName(String);

/// A PostgreSQL sequence name.
///
/// Corresponds to the `relname` of a sequence in `pg_class`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct SequenceName(String);

/// A PostgreSQL collation name.
///
/// Corresponds to `pg_collation.collname`.
#[nutype(
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        PartialOrd,
        Ord,
        Serialize,
        Deserialize,
        AsRef,
        Deref,
        Into
    )
)]
pub struct CollationName(String);

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

    mod oid_tests {
        use super::*;

        #[test]
        fn oid_creation_and_access() {
            let oid = Oid::new(12345);
            assert_eq!(*oid.as_ref(), 12345);
        }

        #[test]
        fn oid_equality() {
            let oid1 = Oid::new(100);
            let oid2 = Oid::new(100);
            let oid3 = Oid::new(200);
            assert_eq!(oid1, oid2);
            assert_ne!(oid1, oid3);
        }

        #[test]
        fn oid_ordering() {
            let oid1 = Oid::new(100);
            let oid2 = Oid::new(200);
            assert!(oid1 < oid2);
        }
    }

    mod name_tests {
        use super::*;

        #[test]
        fn schema_name_valid() {
            let name = SchemaName::try_new("public".to_string());
            assert!(name.is_ok());
            assert_eq!(name.unwrap().as_ref(), "public");
        }

        #[test]
        fn schema_name_empty_rejected() {
            let name = SchemaName::try_new(String::new());
            assert!(name.is_err());
        }

        #[test]
        fn table_name_valid() {
            let name = TableName::try_new("users".to_string());
            assert!(name.is_ok());
            assert_eq!(name.unwrap().as_ref(), "users");
        }

        #[test]
        fn column_name_valid() {
            let name = ColumnName::try_new("id".to_string());
            assert!(name.is_ok());
            assert_eq!(name.unwrap().as_ref(), "id");
        }

        #[test]
        fn constraint_name_valid() {
            let name = ConstraintName::try_new("users_pkey".to_string());
            assert!(name.is_ok());
        }

        #[test]
        fn index_name_valid() {
            let name = IndexName::try_new("users_email_idx".to_string());
            assert!(name.is_ok());
        }

        #[test]
        fn type_name_valid() {
            let name = TypeName::try_new("int4".to_string());
            assert!(name.is_ok());
            assert_eq!(name.unwrap().as_ref(), "int4");
        }

        #[test]
        fn sequence_name_valid() {
            let name = SequenceName::try_new("users_id_seq".to_string());
            assert!(name.is_ok());
        }

        #[test]
        fn collation_name_valid() {
            let name = CollationName::try_new("en_US".to_string());
            assert!(name.is_ok());
        }
    }

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
