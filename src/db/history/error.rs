//! Error types for schema history operations.
//!
//! Note: This module has `#[allow(unused_assignments)]` because thiserror's proc macros
//! generate code that references struct fields in error message formatting, which the
//! compiler cannot detect, leading to false positive "unused_assignments" warnings.

#![allow(unused_assignments)]

use miette::Diagnostic;
use thiserror::Error;

/// Errors that can occur when applying operations to a namespace.
///
/// Note: The fields in these variants are used by thiserror's generated Display and Diagnostic
/// implementations to format error messages.
#[derive(Debug, Error, Diagnostic)]
pub enum ApplyError {
    /// The target table was not found in the namespace.
    #[error("table '{name}' not found in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::table_not_found),
        help("Ensure the table exists before applying operations that reference it")
    )]
    TableNotFound { schema: String, name: String },

    /// The target column was not found in the table.
    #[error("column '{column}' not found in table '{schema}.{table}'")]
    #[diagnostic(
        code(tern::history::column_not_found),
        help("Ensure the column exists before applying operations that reference it")
    )]
    ColumnNotFound {
        schema: String,
        table: String,
        column: String,
    },

    /// The target constraint was not found in the table.
    #[error("constraint '{constraint}' not found in table '{schema}.{table}'")]
    #[diagnostic(
        code(tern::history::constraint_not_found),
        help("Ensure the constraint exists before applying operations that reference it")
    )]
    ConstraintNotFound {
        schema: String,
        table: String,
        constraint: String,
    },

    /// The target index was not found in the namespace.
    #[error("index '{name}' not found in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::index_not_found),
        help("Ensure the index exists before applying operations that reference it")
    )]
    IndexNotFound { schema: String, name: String },

    /// The target view was not found in the namespace.
    #[error("view '{name}' not found in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::view_not_found),
        help("Ensure the view exists before applying operations that reference it")
    )]
    ViewNotFound { schema: String, name: String },

    /// The target sequence was not found in the namespace.
    #[error("sequence '{name}' not found in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::sequence_not_found),
        help("Ensure the sequence exists before applying operations that reference it")
    )]
    SequenceNotFound { schema: String, name: String },

    /// The target enum type was not found in the namespace.
    #[error("enum type '{name}' not found in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::enum_not_found),
        help("Ensure the enum type exists before applying operations that reference it")
    )]
    EnumNotFound { schema: String, name: String },

    /// An object with the same name already exists.
    #[error("{kind} '{name}' already exists in schema '{schema}'")]
    #[diagnostic(
        code(tern::history::already_exists),
        help("Cannot create an object that already exists")
    )]
    AlreadyExists {
        schema: String,
        name: String,
        kind: &'static str,
    },

    /// The operation targets a different schema than this namespace.
    #[error("operation targets schema '{target}' but namespace is '{actual}'")]
    #[diagnostic(
        code(tern::history::schema_mismatch),
        help("Operations must target the same schema as the namespace being modified")
    )]
    SchemaMismatch { target: String, actual: String },

    /// The enum value position reference was not found.
    #[error("enum value '{reference}' not found in enum '{schema}.{enum_name}'")]
    #[diagnostic(
        code(tern::history::enum_value_not_found),
        help("The BEFORE or AFTER reference must exist in the enum")
    )]
    EnumValueNotFound {
        schema: String,
        enum_name: String,
        reference: String,
    },
}
