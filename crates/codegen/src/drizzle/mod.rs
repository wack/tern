//! Drizzle ORM code generation module.
//!
//! This module provides code generation for TypeScript Drizzle ORM schema
//! definitions from PostgreSQL table definitions. It supports:
//!
//! - Idiomatic Drizzle `pgTable` definitions with proper type mappings
//! - Primary keys (single and composite), foreign keys, unique constraints
//! - Optional relation generation via `relations()` calls
//! - camelCase or snake_case column naming with explicit DB column names
//! - Non-public schema support via `pgSchema`
//!
//! # Example
//!
//! ```ignore
//! use tern_codegen::{Codegen, drizzle::DrizzleCodegen};
//!
//! let codegen = DrizzleCodegen::new(DrizzleCodegenConfig::default());
//! let output = codegen.generate(tables);
//! // output contains "schema.ts" with Drizzle table definitions
//! ```

mod column;
mod generator;
mod imports;
mod naming;
mod table;
mod type_mapping;

#[cfg(test)]
mod tests;

pub use generator::DrizzleCodegen;

use std::fmt;

/// Configuration for Drizzle ORM code generation.
#[derive(Debug, Clone)]
pub struct DrizzleCodegenConfig {
    /// Output mode: single file or multiple files.
    pub output_mode: OutputMode,

    /// Whether to generate Drizzle `relations()` calls for foreign keys.
    pub generate_relations: bool,

    /// Whether to use camelCase for column variable names.
    ///
    /// When true (default), column names like `user_id` become `userId` in the
    /// generated code, with the original DB name passed to the builder:
    /// `integer("user_id")`.
    ///
    /// When false, column names are kept as-is: `user_id: integer("user_id")`.
    pub camel_case_columns: bool,

    /// Optional schema name for non-public schemas.
    ///
    /// When set, uses `pgSchema` instead of `pgTable`:
    /// ```ignore
    /// const mySchema = pgSchema("myschema");
    /// export const users = mySchema.table("users", { ... });
    /// ```
    pub schema_name: Option<String>,
}

impl Default for DrizzleCodegenConfig {
    fn default() -> Self {
        Self {
            output_mode: OutputMode::default(),
            generate_relations: false,
            camel_case_columns: true,
            schema_name: None,
        }
    }
}

/// Output mode for generated code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutputMode {
    /// Generate all table definitions in a single `schema.ts` file.
    #[default]
    SingleFile,
    /// Generate separate files per table with an `index.ts` barrel export.
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

/// Errors that can occur during Drizzle code generation.
#[derive(Debug, thiserror::Error)]
pub enum DrizzleCodegenError {
    /// An unsupported PostgreSQL type was encountered.
    #[error("unsupported PostgreSQL type: {0}")]
    UnsupportedType(String),

    /// A table has no columns.
    #[error("table has no columns: {0}")]
    EmptyTable(String),

    /// Code generation failed.
    #[error("code generation failed: {0}")]
    GenerationError(String),
}
