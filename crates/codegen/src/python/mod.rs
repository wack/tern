//! Python SQLModel code generation module.
//!
//! This module provides code generation for Python SQLModel models from PostgreSQL
//! table definitions. It supports:
//!
//! - Idiomatic SQLModel models with proper type mappings
//! - Primary keys, foreign keys, unique constraints, and indexes
//! - Optional relationship generation for foreign keys
//! - Generated and identity columns
//! - Proper handling of Python reserved words and invalid identifiers
//!
//! # Example
//!
//! ```ignore
//! use tern_codegen::{Codegen, python::PythonCodegen};
//!
//! let codegen = PythonCodegen::new(PythonCodegenConfig::default());
//! let output = codegen.generate(tables);
//! // output contains "models.py" with SQLModel classes
//! ```

mod field;
mod generator;
mod imports;
mod model;
mod naming;
mod relationship;
mod type_mapping;

#[cfg(test)]
mod tests;

pub use generator::PythonCodegen;

use std::fmt;

/// Configuration for Python SQLModel code generation.
#[derive(Debug, Clone)]
pub struct PythonCodegenConfig {
    /// Whether to generate Pydantic-only base classes for each model.
    /// These are useful for request/response validation without DB coupling.
    pub generate_base_models: bool,

    /// Module name prefix for generated imports (e.g., "app.models").
    pub module_prefix: Option<String>,

    /// Whether to include docstrings from table/column comments.
    pub include_docstrings: bool,

    /// How to handle Python reserved words in identifiers.
    pub reserved_word_strategy: ReservedWordStrategy,

    /// Whether to generate relationship attributes for foreign keys.
    pub generate_relationships: bool,

    /// Output mode: single file or multiple files.
    pub output_mode: OutputMode,
}

impl Default for PythonCodegenConfig {
    fn default() -> Self {
        Self {
            generate_base_models: false,
            module_prefix: None,
            include_docstrings: true,
            reserved_word_strategy: ReservedWordStrategy::default(),
            generate_relationships: false,
            output_mode: OutputMode::default(),
        }
    }
}

/// Strategy for handling Python reserved words in identifiers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ReservedWordStrategy {
    /// Append an underscore: `class` -> `class_`
    #[default]
    AppendUnderscore,
    /// Prepend with prefix: `class` -> `field_class`
    PrependPrefix(String),
}

impl fmt::Display for ReservedWordStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AppendUnderscore => write!(f, "append_underscore"),
            Self::PrependPrefix(prefix) => write!(f, "prepend_prefix({prefix})"),
        }
    }
}

/// Output mode for generated code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Generate all models in a single `models.py` file.
    #[default]
    SingleFile,
    /// Generate separate files for each model with a shared types module.
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

/// Errors that can occur during Python code generation.
#[derive(Debug, thiserror::Error)]
pub enum PythonCodegenError {
    /// An unsupported PostgreSQL type was encountered.
    #[error("unsupported PostgreSQL type: {0}")]
    UnsupportedType(String),

    /// A table has no columns.
    #[error("table has no columns: {0}")]
    EmptyTable(String),

    /// An identifier is invalid after sanitization.
    #[error("invalid identifier after sanitization: {0}")]
    InvalidIdentifier(String),

    /// Code generation failed.
    #[error("code generation failed: {0}")]
    GenerationError(String),
}
