//! PostgreSQL schema types for catalog introspection.
//!
//! This module provides type-safe wrappers around PostgreSQL catalog concepts,
//! ensuring that OIDs, names, and catalog codes are used correctly throughout
//! the codebase.

use nutype::nutype;

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
}
