//! DDL types for PostgreSQL schema representation.
//!
//! This crate provides types for representing PostgreSQL schema objects in memory,
//! designed to be:
//!
//! - **Serializable**: Can be saved/loaded for migration state tracking
//! - **Comparable**: Can diff two schemas to detect changes
//! - **Complete**: Captures enough detail to regenerate DDL
//!
//! # Module Structure
//!
//! - [`schema`]: Core schema primitives (Oid, name types)
//! - [`types`]: Primitive types used across the model (expressions, type info, etc.)
//! - [`column`]: Column definitions with type, default, identity, and generation info
//! - [`constraint`]: Constraint definitions (primary key, foreign key, unique, check, exclusion)
//! - [`index`]: Index definitions with columns, sort order, and predicates
//! - [`table`]: Table definitions aggregating columns, constraints, and indexes

pub mod column;
pub mod constraint;
pub mod index;
pub mod schema;
pub mod table;
pub mod types;

// Re-export main types for convenience
pub use column::{Column, GeneratedColumn, GeneratedStorage, IdentityKind};
pub use constraint::{
    CheckConstraint, Constraint, ConstraintKind, ExclusionConstraint, ExclusionElement,
    ForeignKeyConstraint, PrimaryKeyConstraint, UniqueConstraint,
};
pub use index::{Index, IndexColumn, NullsOrder, SortOrder};
pub use schema::{
    CollationName, ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName, TableName,
    TypeName,
};
pub use table::{Table, TableKind};
pub use types::{
    Comment, ForeignKeyAction, IndexMethod, QualifiedCollationName, QualifiedName,
    QualifiedTableName, QualifiedTypeName, SqlExpr, TypeInfo,
};

// =============================================================================
// Macros for Reducing Code Duplication
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

/// Implements `as_char()` method and `TryFrom<char>` trait for enums that map to single characters.
///
/// This macro generates:
/// - `as_char(&self) -> char` method returning the character for each variant
/// - `TryFrom<char>` implementation parsing characters back to variants
/// - Error handling with the specified error type
///
/// The error type must be a tuple struct that can be constructed with a single `char` argument.
///
/// # Example
///
/// ```ignore
/// pub enum MyEnum {
///     VariantA,
///     VariantB,
/// }
///
/// impl_char_enum!(MyEnum, MyError, [
///     VariantA => 'a',
///     VariantB => 'b',
/// ]);
/// ```
#[macro_export]
macro_rules! impl_char_enum {
    (
        $enum_type:ty,
        $error_type:path,
        [
            $($variant:ident => $char:expr),+ $(,)?
        ]
    ) => {
        impl $enum_type {
            /// Returns the single-character code used by PostgreSQL.
            #[must_use]
            pub const fn as_char(&self) -> char {
                match self {
                    $(Self::$variant => $char),+
                }
            }
        }

        impl TryFrom<char> for $enum_type {
            type Error = $error_type;

            fn try_from(c: char) -> Result<Self, Self::Error> {
                match c {
                    $($char => Ok(Self::$variant)),+,
                    _ => Err($error_type(c)),
                }
            }
        }
    };
}

/// Implements `as_sql()` method that returns SQL keywords for enum variants.
///
/// This macro generates:
/// - `as_sql(&self) -> &'static str` method
/// - Returns the SQL keyword for each variant
///
/// # Example
///
/// ```ignore
/// impl_sql_enum!(MyEnum, [
///     Cascade => "CASCADE",
///     Restrict => "RESTRICT",
/// ]);
/// ```
#[macro_export]
macro_rules! impl_sql_enum {
    (
        $enum_type:ty,
        [
            $($variant:ident => $sql:expr),+ $(,)?
        ]
    ) => {
        impl $enum_type {
            /// Returns the SQL keyword for this variant.
            #[must_use]
            pub const fn as_sql(&self) -> &'static str {
                match self {
                    $(Self::$variant => $sql),+
                }
            }
        }
    };
}

/// Implements `as_str()` method for enums with string representations.
///
/// This macro generates:
/// - `as_str(&self) -> &'static str` method
/// - Returns the string representation for each variant
///
/// # Example
///
/// ```ignore
/// impl_str_enum!(MyEnum, [
///     BTree => "btree",
///     Hash => "hash",
/// ]);
/// ```
#[macro_export]
macro_rules! impl_str_enum {
    (
        $enum_type:ty,
        [
            $($variant:ident => $str:expr),+ $(,)?
        ]
    ) => {
        impl $enum_type {
            /// Returns the string representation for this variant.
            #[must_use]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => $str),+
                }
            }
        }
    };
}

/// Implements `TryFrom<&str>` trait for enums that parse from strings.
///
/// This macro generates:
/// - `TryFrom<&str>` implementation parsing strings to enum variants
/// - Error handling with automatic string conversion for the error type
///
/// The error type must accept a String in its constructor: `Error(String)`.
///
/// # Example
///
/// ```ignore
/// impl_str_from_enum!(MyEnum, MyError, [
///     "btree" => BTree,
///     "hash" => Hash,
/// ]);
/// ```
#[macro_export]
macro_rules! impl_str_from_enum {
    (
        $enum_type:ty,
        $error_type:path,
        [
            $($str_val:expr => $variant:ident),+ $(,)?
        ]
    ) => {
        impl TryFrom<&str> for $enum_type {
            type Error = $error_type;

            fn try_from(s: &str) -> Result<Self, Self::Error> {
                match s {
                    $($str_val => Ok(Self::$variant)),+,
                    _ => Err($error_type(s.to_string())),
                }
            }
        }
    };
}
