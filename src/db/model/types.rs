//! Primitive types used across the domain model.

use nutype::nutype;
use serde::{Deserialize, Serialize};

use crate::db::schema::{CollationName, SchemaName, TableName, TypeName};

// =============================================================================
// SQL Expression
// =============================================================================

/// A decompiled SQL expression from `pg_get_expr()`.
///
/// Used for column defaults, check constraint expressions, generated column
/// expressions, and partial index predicates. The expression is stored as
/// a raw SQL string that can be directly embedded in DDL statements.
#[nutype(derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    AsRef,
    Deref,
    Into
))]
pub struct SqlExpr(String);

// =============================================================================
// Qualified Names
// =============================================================================

/// A schema-qualified name for a database object.
///
/// Used for referencing tables in foreign keys, types, collations, etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QualifiedName<T> {
    pub schema: SchemaName,
    pub name: T,
}

impl<T> QualifiedName<T> {
    /// Creates a new qualified name.
    pub fn new(schema: SchemaName, name: T) -> Self {
        Self { schema, name }
    }
}

impl<T: AsRef<str>> std::fmt::Display for QualifiedName<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.schema.as_ref(), self.name.as_ref())
    }
}

/// A schema-qualified table name.
pub type QualifiedTableName = QualifiedName<TableName>;

/// A schema-qualified type name.
pub type QualifiedTypeName = QualifiedName<TypeName>;

/// A schema-qualified collation name.
pub type QualifiedCollationName = QualifiedName<CollationName>;

// =============================================================================
// Type Information
// =============================================================================

/// PostgreSQL type information as stored in `pg_type`.
///
/// This captures both the raw type identity (OID, name, schema) and the
/// formatted representation with any type modifiers (e.g., `varchar(255)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeInfo {
    /// The type's name (e.g., "int4", "varchar", "numeric").
    pub name: TypeName,
    /// The schema containing this type (e.g., "pg_catalog").
    pub schema: SchemaName,
    /// The formatted type with modifiers, from `format_type(atttypid, atttypmod)`.
    /// Examples: "integer", "character varying(255)", "numeric(10,2)".
    pub formatted: String,
    /// True if this is an array type (e.g., `integer[]`).
    pub is_array: bool,
}

// =============================================================================
// Foreign Key Actions
// =============================================================================

/// Foreign key referential actions for `ON DELETE` and `ON UPDATE` clauses.
///
/// These correspond to the single-character codes in `pg_constraint.confdeltype`
/// and `pg_constraint.confupdtype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ForeignKeyAction {
    /// No action (`'a'`). The default.
    #[default]
    NoAction,
    /// Restrict (`'r'`).
    Restrict,
    /// Cascade (`'c'`).
    Cascade,
    /// Set null (`'n'`).
    SetNull,
    /// Set default (`'d'`).
    SetDefault,
}

/// Error returned when parsing an invalid foreign key action character.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid foreign key action: '{0}'")]
pub struct InvalidForeignKeyAction(pub char);

crate::impl_char_enum!(ForeignKeyAction, InvalidForeignKeyAction, [
    NoAction => 'a',
    Restrict => 'r',
    Cascade => 'c',
    SetNull => 'n',
    SetDefault => 'd',
]);

crate::impl_sql_enum!(ForeignKeyAction, [
    NoAction => "NO ACTION",
    Restrict => "RESTRICT",
    Cascade => "CASCADE",
    SetNull => "SET NULL",
    SetDefault => "SET DEFAULT",
]);

// =============================================================================
// Index Method
// =============================================================================

/// Index access method.
///
/// Corresponds to `pg_am.amname` for the index's access method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IndexMethod {
    /// B-tree index (default, most common).
    #[default]
    BTree,
    /// Hash index.
    Hash,
    /// GiST (Generalized Search Tree) index.
    Gist,
    /// GIN (Generalized Inverted Index).
    Gin,
    /// BRIN (Block Range Index).
    Brin,
    /// SP-GiST (Space-Partitioned GiST).
    SpGist,
}

/// Error returned when parsing an invalid index method name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid index method: '{0}'")]
pub struct InvalidIndexMethod(pub String);

crate::impl_str_enum!(IndexMethod, [
    BTree => "btree",
    Hash => "hash",
    Gist => "gist",
    Gin => "gin",
    Brin => "brin",
    SpGist => "spgist",
]);

crate::impl_str_from_enum!(IndexMethod, InvalidIndexMethod, [
    "btree" => BTree,
    "hash" => Hash,
    "gist" => Gist,
    "gin" => Gin,
    "brin" => Brin,
    "spgist" => SpGist,
]);

// =============================================================================
// Comment
// =============================================================================

/// A comment on a database object (from `COMMENT ON ...`).
#[nutype(derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    AsRef,
    Deref,
    Into
))]
pub struct Comment(String);

#[cfg(test)]
mod tests {
    use super::*;

    mod foreign_key_action_tests {
        use super::*;

        #[test]
        fn from_char() {
            assert_eq!(
                ForeignKeyAction::try_from('a'),
                Ok(ForeignKeyAction::NoAction)
            );
            assert_eq!(
                ForeignKeyAction::try_from('r'),
                Ok(ForeignKeyAction::Restrict)
            );
            assert_eq!(
                ForeignKeyAction::try_from('c'),
                Ok(ForeignKeyAction::Cascade)
            );
            assert_eq!(
                ForeignKeyAction::try_from('n'),
                Ok(ForeignKeyAction::SetNull)
            );
            assert_eq!(
                ForeignKeyAction::try_from('d'),
                Ok(ForeignKeyAction::SetDefault)
            );
        }

        #[test]
        fn to_char() {
            assert_eq!(ForeignKeyAction::NoAction.as_char(), 'a');
            assert_eq!(ForeignKeyAction::Cascade.as_char(), 'c');
        }

        #[test]
        fn to_sql() {
            assert_eq!(ForeignKeyAction::NoAction.as_sql(), "NO ACTION");
            assert_eq!(ForeignKeyAction::Cascade.as_sql(), "CASCADE");
            assert_eq!(ForeignKeyAction::SetNull.as_sql(), "SET NULL");
        }

        #[test]
        fn invalid_char() {
            assert!(ForeignKeyAction::try_from('x').is_err());
        }
    }

    mod index_method_tests {
        use super::*;

        #[test]
        fn from_str() {
            assert_eq!(IndexMethod::try_from("btree"), Ok(IndexMethod::BTree));
            assert_eq!(IndexMethod::try_from("hash"), Ok(IndexMethod::Hash));
            assert_eq!(IndexMethod::try_from("gin"), Ok(IndexMethod::Gin));
        }

        #[test]
        fn to_str() {
            assert_eq!(IndexMethod::BTree.as_str(), "btree");
            assert_eq!(IndexMethod::Gin.as_str(), "gin");
        }

        #[test]
        fn invalid_str() {
            assert!(IndexMethod::try_from("invalid").is_err());
        }
    }

    mod qualified_name_tests {
        use super::*;

        #[test]
        fn display() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let table = TableName::try_new("users".to_string()).unwrap();
            let qualified = QualifiedTableName::new(schema, table);
            assert_eq!(qualified.to_string(), "public.users");
        }
    }
}
