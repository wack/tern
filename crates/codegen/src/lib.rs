//! Code generation library for Tern.

use std::collections::HashMap;

use tern_ddl::Table;

pub mod drizzle;
pub mod python;
pub mod rust;

/// Trait for code generation from database schema definitions.
pub trait Codegen {
    /// Generates code from a list of table definitions.
    ///
    /// Returns a map of file names to their generated content.
    fn generate(&self, tables: Vec<Table>) -> HashMap<String, String>;
}
