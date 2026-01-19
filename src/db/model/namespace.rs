//! Schema (namespace) and related types.

use serde::{Deserialize, Serialize};

use crate::db::schema::{Oid, SchemaName, SequenceName, TableName, TypeName};

use super::table::Table;
use super::types::{Comment, SqlExpr, TypeInfo};

/// A database schema (namespace) containing tables and other objects.
///
/// This is the top-level container for all objects within a PostgreSQL schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    /// The schema OID.
    pub oid: Oid,
    /// The schema name.
    pub name: SchemaName,
    /// Tables in this schema.
    pub tables: Vec<Table>,
    /// Views in this schema (both regular and materialized).
    pub views: Vec<View>,
    /// Sequences in this schema.
    pub sequences: Vec<Sequence>,
    /// Enum types defined in this schema.
    pub enums: Vec<EnumType>,
    /// Comment on this schema, if any.
    pub comment: Option<Comment>,
}

/// A view definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// The view OID.
    pub oid: Oid,
    /// The view name (views use `pg_class.relname`).
    pub name: TableName,
    /// The view definition (SELECT statement).
    pub definition: SqlExpr,
    /// Whether this is a materialized view.
    pub is_materialized: bool,
    /// Comment on this view, if any.
    pub comment: Option<Comment>,
}

/// A sequence definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sequence {
    /// The sequence OID.
    pub oid: Oid,
    /// The sequence name.
    pub name: SequenceName,
    /// The data type of the sequence (smallint, integer, or bigint).
    pub data_type: TypeInfo,
    /// The starting value.
    pub start_value: i64,
    /// The increment between values.
    pub increment: i64,
    /// The minimum value.
    pub min_value: i64,
    /// The maximum value.
    pub max_value: i64,
    /// How many values to cache.
    pub cache_size: i64,
    /// Whether the sequence wraps around (CYCLE).
    pub is_cyclic: bool,
    /// Comment on this sequence, if any.
    pub comment: Option<Comment>,
}

/// An enum type definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumType {
    /// The enum type OID.
    pub oid: Oid,
    /// The enum type name.
    pub name: TypeName,
    /// The enum values in order.
    pub values: Vec<String>,
    /// Comment on this enum type, if any.
    pub comment: Option<Comment>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_namespace() {
        let ns = Namespace {
            oid: Oid::new(12345),
            name: SchemaName::try_new("public".to_string()).unwrap(),
            tables: vec![],
            views: vec![],
            sequences: vec![],
            enums: vec![],
            comment: None,
        };

        assert_eq!(ns.name.as_ref(), "public");
        assert!(ns.tables.is_empty());
    }

    #[test]
    fn view_definition() {
        let view = View {
            oid: Oid::new(12346),
            name: TableName::try_new("active_users".to_string()).unwrap(),
            definition: SqlExpr::new("SELECT * FROM users WHERE active = true".to_string()),
            is_materialized: false,
            comment: None,
        };

        assert!(!view.is_materialized);
        assert!(view.definition.as_ref().contains("SELECT"));
    }

    #[test]
    fn materialized_view() {
        let view = View {
            oid: Oid::new(12347),
            name: TableName::try_new("user_stats".to_string()).unwrap(),
            definition: SqlExpr::new(
                "SELECT user_id, COUNT(*) FROM orders GROUP BY user_id".to_string(),
            ),
            is_materialized: true,
            comment: None,
        };

        assert!(view.is_materialized);
    }

    #[test]
    fn sequence_definition() {
        let seq = Sequence {
            oid: Oid::new(12348),
            name: SequenceName::try_new("users_id_seq".to_string()).unwrap(),
            data_type: TypeInfo {
                name: TypeName::try_new("int8".to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: "bigint".to_string(),
                is_array: false,
            },
            start_value: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cache_size: 1,
            is_cyclic: false,
            comment: None,
        };

        assert_eq!(seq.increment, 1);
        assert!(!seq.is_cyclic);
    }

    #[test]
    fn enum_type_definition() {
        let enum_type = EnumType {
            oid: Oid::new(12349),
            name: TypeName::try_new("status".to_string()).unwrap(),
            values: vec![
                "pending".to_string(),
                "active".to_string(),
                "completed".to_string(),
            ],
            comment: None,
        };

        assert_eq!(enum_type.values.len(), 3);
        assert_eq!(enum_type.values[0], "pending");
    }
}
