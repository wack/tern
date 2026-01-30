//! Rust SeaORM code generation module.
//!
//! This module provides code generation for Rust SeaORM entities from PostgreSQL
//! table definitions. It supports:
//!
//! - Idiomatic SeaORM entities with proper type mappings
//! - Primary keys (single and composite), foreign keys, unique constraints, and indexes
//! - Both compact (DeriveEntityModel) and expanded entity formats
//! - Proper handling of Rust reserved words and invalid identifiers
//! - Relationship generation for foreign keys
//! - ActiveEnum generation for PostgreSQL enums
//!
//! # Example
//!
//! ```ignore
//! use tern_codegen::{Codegen, rust::RustSeaOrmCodegen};
//!
//! let codegen = RustSeaOrmCodegen::new(RustSeaOrmCodegenConfig::default());
//! let output = codegen.generate(tables);
//! // output contains generated SeaORM entity files
//! ```

mod column;
mod entity;
mod generator;
mod imports;
mod naming;
mod primary_key;
mod relation;
mod type_mapping;

#[cfg(test)]
mod tests;

pub use generator::RustSeaOrmCodegen;

use std::fmt;

/// Configuration for Rust SeaORM code generation.
#[derive(Debug, Clone)]
pub struct RustSeaOrmCodegenConfig {
    /// Whether to generate compact format (DeriveEntityModel) or expanded format.
    /// Compact is recommended and is the default.
    pub entity_format: EntityFormat,

    /// Whether to generate relationship attributes and Related trait implementations.
    pub generate_relations: bool,

    /// Whether to include doc comments from table/column comments.
    pub include_doc_comments: bool,

    /// How to handle Rust reserved words in identifiers.
    pub reserved_word_strategy: ReservedWordStrategy,

    /// Module name for the generated entities (used for cross-module references).
    pub module_name: Option<String>,

    /// Whether to generate ActiveEnum types for PostgreSQL enums.
    /// Note: Requires enum definitions to be passed separately or inferred.
    pub generate_active_enums: bool,

    /// The schema name to use (if not public).
    pub schema_name: Option<String>,

    /// Output mode: single file or multiple files.
    pub output_mode: OutputMode,
}

impl Default for RustSeaOrmCodegenConfig {
    fn default() -> Self {
        Self {
            entity_format: EntityFormat::default(),
            generate_relations: false,
            include_doc_comments: true,
            reserved_word_strategy: ReservedWordStrategy::default(),
            module_name: None,
            generate_active_enums: false,
            schema_name: None,
            output_mode: OutputMode::default(),
        }
    }
}

/// Entity format for generated code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EntityFormat {
    /// Uses DeriveEntityModel macro - recommended, less boilerplate.
    #[default]
    Compact,
    /// Generates explicit Column enum, PrimaryKey enum, and trait implementations.
    Expanded,
}

impl fmt::Display for EntityFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compact => write!(f, "compact"),
            Self::Expanded => write!(f, "expanded"),
        }
    }
}

/// Strategy for handling Rust reserved words in identifiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReservedWordStrategy {
    /// Append an underscore: `type` -> `type_` with column_name attribute.
    #[default]
    AppendUnderscore,
    /// Use raw identifier: `type` -> `r#type`.
    RawIdentifier,
    /// Prepend with custom prefix: `type` -> `field_type`.
    PrependPrefix(String),
}

impl fmt::Display for ReservedWordStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppendUnderscore => write!(f, "append_underscore"),
            Self::RawIdentifier => write!(f, "raw_identifier"),
            Self::PrependPrefix(prefix) => write!(f, "prepend_prefix({prefix})"),
        }
    }
}

/// Output mode for generated code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Generate all entities in a single `entities.rs` file.
    SingleFile,
    /// Generate separate files for each entity with a `mod.rs`.
    #[default]
    MultiFile,
}

impl fmt::Display for OutputMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SingleFile => write!(f, "single_file"),
            Self::MultiFile => write!(f, "multi_file"),
        }
    }
}

/// Errors that can occur during Rust SeaORM code generation.
#[derive(Debug, thiserror::Error)]
pub enum RustSeaOrmCodegenError {
    /// An unsupported PostgreSQL type was encountered.
    #[error("unsupported PostgreSQL type: {type_name} (formatted: {formatted})")]
    UnsupportedType {
        type_name: String,
        formatted: String,
    },

    /// A table has no columns.
    #[error("table '{table_name}' has no columns")]
    EmptyTable { table_name: String },

    /// A table has no primary key.
    #[error("table '{table_name}' has no primary key")]
    NoPrimaryKey { table_name: String },

    /// An identifier is invalid after sanitization.
    #[error("invalid identifier after sanitization: '{original}' -> '{sanitized}'")]
    InvalidIdentifier { original: String, sanitized: String },

    /// Code generation failed.
    #[error("code generation failed: {message}")]
    GenerationError { message: String },
}

/// Warnings that don't prevent generation but should be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustSeaOrmCodegenWarning {
    /// An unsupported constraint was encountered.
    UnsupportedConstraint {
        table: String,
        constraint: String,
        kind: String,
    },
    /// A generated column was ignored.
    GeneratedColumnIgnored { table: String, column: String },
    /// A feature flag is required for the generated code.
    FeatureFlagRequired { feature: String, reason: String },
    /// A table has no primary key.
    TableWithoutPrimaryKey { table: String },
}

impl fmt::Display for RustSeaOrmCodegenWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedConstraint {
                table,
                constraint,
                kind,
            } => {
                write!(
                    f,
                    "Table '{table}': Unsupported {kind} constraint '{constraint}'"
                )
            }
            Self::GeneratedColumnIgnored { table, column } => {
                write!(
                    f,
                    "Table '{table}': Generated column '{column}' ignored (not supported by SeaORM)"
                )
            }
            Self::FeatureFlagRequired { feature, reason } => {
                write!(f, "Required SeaORM feature: '{feature}' ({reason})")
            }
            Self::TableWithoutPrimaryKey { table } => {
                write!(
                    f,
                    "Table '{table}': No primary key defined (SeaORM requires a primary key)"
                )
            }
        }
    }
}

/// Definition of a PostgreSQL enum type for ActiveEnum generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDefinition {
    /// The enum type name.
    pub name: String,
    /// The schema containing the enum (if not public).
    pub schema: Option<String>,
    /// The enum values in order.
    pub values: Vec<String>,
}
