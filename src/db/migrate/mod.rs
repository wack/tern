//! Migration planning and execution.
//!
//! This module provides the pipeline for converting schema diffs into
//! executable SQL migrations.
//!
//! # Architecture
//!
//! The migration system uses a three-layer pipeline:
//!
//! 1. **Collection**: `OperationCollector` extracts semantic operations from diffs
//! 2. **Ordering**: Operations are topologically sorted by dependencies
//! 3. **Rendering**: `PostgresRenderer` converts operations to SQL
//!
//! # Example
//!
//! ```ignore
//! use tern::db::diff::diff_namespaces;
//! use tern::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
//!
//! // Compare schemas
//! let diff = diff_namespaces(&source, &target);
//!
//! // Create migration plan
//! let plan = MigrationPlan::from_diff(&diff);
//!
//! // Render to SQL
//! let renderer = PostgresRenderer::new(RenderConfig::default());
//! let script = plan.render(&renderer);
//!
//! // Get the SQL
//! println!("{}", script.to_sql());
//! ```

mod collector;
mod error;
mod operation;
mod render;

pub use collector::{CollectorConfig, OperationCollector};
pub use error::MigrationError;
pub use operation::{
    ColumnChanges, CommentTarget, DefaultChange, EnumValuePosition, GeneratedChange,
    IdentityChange, ObjectKind, Operation, OperationId, SequenceChanges, SetColumnType,
};
pub use render::{IdentifierQuoting, PostgresRenderer, RenderConfig, RenderedOperation, Renderer};
