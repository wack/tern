//! Migration-specific error types.

use miette::Diagnostic;
use thiserror::Error;

/// Errors that can occur during migration planning or execution.
///
/// Note: The `unused` warnings are false positives. These fields are used by
/// thiserror's generated Display implementation to format error messages.
#[derive(Debug, Error, Diagnostic)]
#[allow(clippy::enum_variant_names)]
pub enum MigrationError {
    /// A circular dependency was detected between tables.
    #[error("circular dependency detected involving table {schema}.{table}")]
    #[diagnostic(
        code(tern::migrate::circular_dependency),
        help("Check for circular foreign key references between tables")
    )]
    CircularDependency { schema: String, table: String },

    /// An enum type cannot be dropped because it is still in use.
    #[error("cannot drop enum {schema}.{name} because it is still in use")]
    #[diagnostic(
        code(tern::migrate::enum_in_use),
        help("Drop or alter the columns using this enum first")
    )]
    EnumInUse { schema: String, name: String },

    /// Rollback SQL cannot be generated for this operation.
    #[error("cannot generate rollback for operation: {operation}")]
    #[diagnostic(
        code(tern::migrate::no_rollback),
        help("Some operations like DROP COLUMN cannot be automatically reversed")
    )]
    NoRollback { operation: String },

    /// The requested operation is not supported.
    #[error("unsupported operation: {description}")]
    #[diagnostic(code(tern::migrate::unsupported))]
    Unsupported { description: String },

    /// An enum value cannot be removed (PostgreSQL limitation).
    #[error("cannot remove enum value '{value}' from {schema}.{enum_name}")]
    #[diagnostic(
        code(tern::migrate::enum_value_removal),
        help(
            "PostgreSQL does not support removing enum values. Consider creating a new enum type and migrating data."
        )
    )]
    EnumValueRemoval {
        schema: String,
        enum_name: String,
        value: String,
    },

    /// Enum values cannot be reordered (PostgreSQL limitation).
    #[error("cannot reorder enum values in {schema}.{enum_name}")]
    #[diagnostic(
        code(tern::migrate::enum_reorder),
        help(
            "PostgreSQL does not support reordering enum values. Consider creating a new enum type and migrating data."
        )
    )]
    EnumValueReorder { schema: String, enum_name: String },
}
