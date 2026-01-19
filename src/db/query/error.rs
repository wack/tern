//! Error types for catalog query operations.

use crate::db::model::column::InvalidGeneratedStorage;
use crate::db::model::table::InvalidTableKind;
use crate::db::model::types::InvalidForeignKeyAction;
use crate::db::model::types::InvalidIndexMethod;
use crate::db::schema::{
    CollationNameError, ColumnNameError, ConstraintNameError, IndexNameError, SchemaNameError,
    SequenceNameError, TableNameError, TypeNameError,
};

/// Error returned when loading schema from the database.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum QueryError {
    /// Database connection or query error.
    #[error("database error: {0}")]
    Database(#[from] tokio_postgres::Error),

    /// No namespace found with the given name.
    #[error("namespace not found: {0}")]
    NamespaceNotFound(String),

    /// Error parsing a schema name from the database.
    #[error("invalid schema name: {0}")]
    InvalidSchemaName(#[from] SchemaNameError),

    /// Error parsing a table name from the database.
    #[error("invalid table name: {0}")]
    InvalidTableName(#[from] TableNameError),

    /// Error parsing a column name from the database.
    #[error("invalid column name: {0}")]
    InvalidColumnName(#[from] ColumnNameError),

    /// Error parsing a constraint name from the database.
    #[error("invalid constraint name: {0}")]
    InvalidConstraintName(#[from] ConstraintNameError),

    /// Error parsing an index name from the database.
    #[error("invalid index name: {0}")]
    InvalidIndexName(#[from] IndexNameError),

    /// Error parsing a type name from the database.
    #[error("invalid type name: {0}")]
    InvalidTypeName(#[from] TypeNameError),

    /// Error parsing a sequence name from the database.
    #[error("invalid sequence name: {0}")]
    InvalidSequenceName(#[from] SequenceNameError),

    /// Error parsing a collation name from the database.
    #[error("invalid collation name: {0}")]
    InvalidCollationName(#[from] CollationNameError),

    /// Error parsing a table kind character.
    #[error("invalid table kind: {0}")]
    InvalidTableKind(#[from] InvalidTableKind),

    /// Error parsing a foreign key action character.
    #[error("invalid foreign key action: {0}")]
    InvalidForeignKeyAction(#[from] InvalidForeignKeyAction),

    /// Error parsing an index method.
    #[error("invalid index method: {0}")]
    InvalidIndexMethod(#[from] InvalidIndexMethod),

    /// Error parsing a generated storage type.
    #[error("invalid generated storage: {0}")]
    InvalidGeneratedStorage(#[from] InvalidGeneratedStorage),

    /// Missing required data in query result.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// Unexpected null value in query result.
    #[error("unexpected null value for field: {0}")]
    UnexpectedNull(&'static str),
}
