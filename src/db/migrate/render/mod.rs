//! SQL rendering for migration operations.
//!
//! This module provides the infrastructure for converting semantic operations
//! into database-specific SQL statements.

mod identifier;
mod postgres;

pub use identifier::IdentifierQuoting;
pub use postgres::PostgresRenderer;

use super::operation::Operation;

/// Rendered SQL for a single operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedOperation {
    /// Forward migration SQL statements.
    pub forward: Vec<String>,
    /// Rollback SQL statements (in reverse order of forward).
    pub rollback: Option<Vec<String>>,
    /// Human-readable description.
    pub description: String,
}

impl RenderedOperation {
    /// Create a new rendered operation with no rollback.
    pub fn forward_only(forward: Vec<String>, description: impl Into<String>) -> Self {
        Self {
            forward,
            rollback: None,
            description: description.into(),
        }
    }

    /// Create a new rendered operation with rollback.
    pub fn with_rollback(
        forward: Vec<String>,
        rollback: Vec<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            forward,
            rollback: Some(rollback),
            description: description.into(),
        }
    }
}

/// Configuration for SQL rendering.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// How to quote identifiers.
    pub quoting: IdentifierQuoting,
    /// Add IF EXISTS to DROP statements.
    pub if_exists: bool,
    /// Add IF NOT EXISTS to CREATE statements.
    pub if_not_exists: bool,
    /// Add CASCADE to DROP statements.
    pub cascade: bool,
    /// Generate rollback SQL.
    pub generate_rollback: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            quoting: IdentifierQuoting::WhenNeeded,
            if_exists: false,
            if_not_exists: false,
            cascade: false,
            generate_rollback: true,
        }
    }
}

/// Trait for rendering operations to SQL.
pub trait Renderer {
    /// Render a single operation to SQL.
    fn render(&self, operation: &Operation) -> RenderedOperation;

    /// Render multiple operations.
    fn render_all(&self, operations: &[Operation]) -> Vec<RenderedOperation> {
        operations.iter().map(|op| self.render(op)).collect()
    }
}
