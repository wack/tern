//! PGLite integration for embedded PostgreSQL execution.
//!
//! This module provides functionality for running PostgreSQL DDL in an embedded
//! PostgreSQL environment using PGLite (PostgreSQL compiled to WebAssembly).
//!
//! # Overview
//!
//! The module consists of several components:
//!
//! - [`PgLiteRuntime`]: Manages the lifecycle of an embedded PGLite instance
//! - [`WorklistExecutor`]: Executes SQL files with dependency resolution
//! - [`SchemaLoader`]: Loads schemas from SQL files into `Namespace` objects
//!
//! # Feature Flag
//!
//! The `pglite` feature must be enabled to use this module. Without it, only
//! the `WorklistExecutor` is available (which can work with any PostgreSQL
//! connection).
//!
//! # Example
//!
//! ```ignore
//! use tern::db::pglite::{SchemaLoader, SchemaLoadConfig};
//!
//! // Load schema from a single file
//! let namespace = SchemaLoader::load_file(".tern/schema.sql").await?;
//!
//! // Load schema from multiple files
//! let namespace = SchemaLoader::load_directory(".tern/schema/").await?;
//! ```

mod error;
#[cfg(feature = "pglite")]
mod runtime;
mod worklist;

pub use error::{PgLiteError, WorklistError};
#[cfg(feature = "pglite")]
pub use runtime::PgLiteRuntime;
pub use worklist::{ExecutionResult, WorklistExecutor};

use std::path::Path;

use crate::db::model::Namespace;
use crate::db::query::{Catalog, load_namespace};

/// Configuration for schema loading operations.
#[derive(Debug, Clone)]
pub struct SchemaLoadConfig {
    /// The PostgreSQL schema (namespace) to introspect after loading.
    ///
    /// Default: "public"
    pub schema_name: String,

    /// Maximum number of retry attempts when encountering dependency errors.
    ///
    /// Default: 100 (sufficient for most schema sizes)
    pub max_retries: usize,
}

impl Default for SchemaLoadConfig {
    fn default() -> Self {
        Self {
            schema_name: "public".to_string(),
            max_retries: 100,
        }
    }
}

/// Loads database schemas from SQL files.
///
/// The `SchemaLoader` executes SQL DDL in an embedded PostgreSQL environment
/// (PGLite) and introspects the resulting schema to produce a `Namespace`.
/// This is the core component of the model-first migration workflow.
///
/// # Single File Mode
///
/// When loading from a single file (`.tern/schema.sql`), all DDL statements
/// are executed in sequence.
///
/// # Multi-File Mode
///
/// When loading from a directory (`.tern/schema/*.sql`), files are executed
/// using a worklist algorithm that automatically resolves dependencies:
///
/// 1. All `.sql` files are queued for execution
/// 2. Each file is attempted in turn
/// 3. If a file fails due to a missing dependency, it's moved to the back
/// 4. The process continues until all files succeed or a circular dependency
///    is detected
///
/// # Example
///
/// ```ignore
/// use tern::db::pglite::SchemaLoader;
///
/// // Load from single file
/// let ns = SchemaLoader::load_file(".tern/schema.sql").await?;
///
/// // Load from directory
/// let ns = SchemaLoader::load_directory(".tern/schema/").await?;
///
/// // Load with custom configuration
/// let config = SchemaLoadConfig {
///     schema_name: "my_schema".to_string(),
///     ..Default::default()
/// };
/// let ns = SchemaLoader::load_file_with_config(".tern/schema.sql", config).await?;
/// ```
pub struct SchemaLoader;

impl SchemaLoader {
    /// Loads a schema from a single SQL file.
    ///
    /// The file should contain valid PostgreSQL DDL that can be executed
    /// against an empty database to create the complete schema.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the SQL file
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The file cannot be read
    /// - The SQL contains syntax errors
    /// - PGLite execution fails
    #[cfg(feature = "pglite")]
    pub async fn load_file(path: impl AsRef<Path>) -> Result<Namespace, PgLiteError> {
        Self::load_file_with_config(path, SchemaLoadConfig::default()).await
    }

    /// Loads a schema from a single SQL file with custom configuration.
    #[cfg(feature = "pglite")]
    pub async fn load_file_with_config(
        path: impl AsRef<Path>,
        config: SchemaLoadConfig,
    ) -> Result<Namespace, PgLiteError> {
        let path = path.as_ref();

        // Read the SQL file
        let sql = std::fs::read_to_string(path).map_err(|e| PgLiteError::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;

        Self::load_sql_with_config(&sql, config).await
    }

    /// Loads a schema from a directory containing multiple SQL files.
    ///
    /// All `.sql` files in the directory are loaded using the worklist
    /// algorithm, which automatically resolves dependencies between files.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the directory containing SQL files
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The directory cannot be read
    /// - Any SQL file contains syntax errors
    /// - Circular dependencies are detected between files
    #[cfg(feature = "pglite")]
    pub async fn load_directory(path: impl AsRef<Path>) -> Result<Namespace, PgLiteError> {
        Self::load_directory_with_config(path, SchemaLoadConfig::default()).await
    }

    /// Loads a schema from a directory with custom configuration.
    #[cfg(feature = "pglite")]
    pub async fn load_directory_with_config(
        path: impl AsRef<Path>,
        config: SchemaLoadConfig,
    ) -> Result<Namespace, PgLiteError> {
        let path = path.as_ref();

        // Collect all SQL files
        let pattern = path.join("*.sql");
        let pattern_str = pattern.to_string_lossy();

        let files: Vec<_> = glob::glob(&pattern_str)
            .map_err(|e| PgLiteError::InvalidGlobPattern {
                pattern: pattern_str.to_string(),
                source: e,
            })?
            .filter_map(|entry| entry.ok())
            .collect();

        if files.is_empty() {
            return Err(PgLiteError::NoSqlFiles {
                path: path.to_path_buf(),
            });
        }

        // Start PGLite runtime
        let mut runtime = PgLiteRuntime::new()?;
        runtime.start().await?;

        // Get a client connection
        let client = runtime.client().await?;

        // Execute files using worklist executor
        let executor = WorklistExecutor::new(config.max_retries);
        executor.execute_files(&client, &files).await?;

        // Introspect the resulting schema
        let catalog = crate::db::query::PostgresCatalog::new(&client);
        let namespace = load_namespace(&catalog, &config.schema_name).await?;

        Ok(namespace)
    }

    /// Loads a schema from raw SQL string.
    ///
    /// This is useful for testing or when the SQL is already in memory.
    #[cfg(feature = "pglite")]
    pub async fn load_sql(sql: &str) -> Result<Namespace, PgLiteError> {
        Self::load_sql_with_config(sql, SchemaLoadConfig::default()).await
    }

    /// Loads a schema from raw SQL string with custom configuration.
    #[cfg(feature = "pglite")]
    pub async fn load_sql_with_config(
        sql: &str,
        config: SchemaLoadConfig,
    ) -> Result<Namespace, PgLiteError> {
        // Start PGLite runtime
        let mut runtime = PgLiteRuntime::new()?;
        runtime.start().await?;

        // Get a client connection
        let client = runtime.client().await?;

        // Execute the SQL
        client
            .batch_execute(sql)
            .await
            .map_err(|e| PgLiteError::ExecuteSql { source: e })?;

        // Introspect the resulting schema
        let catalog = crate::db::query::PostgresCatalog::new(&client);
        let namespace = load_namespace(&catalog, &config.schema_name).await?;

        Ok(namespace)
    }

    /// Loads a schema using an existing PostgreSQL client connection.
    ///
    /// This method allows using a real PostgreSQL database instead of PGLite,
    /// which is useful for:
    /// - Testing without PGLite
    /// - Scenarios where PGLite doesn't support required features
    /// - Debugging schema issues
    ///
    /// # Arguments
    ///
    /// * `client` - An existing tokio-postgres client
    /// * `sql` - The SQL to execute
    /// * `config` - Loading configuration
    ///
    /// # Warning
    ///
    /// This executes DDL against the provided database. Use with caution
    /// to avoid modifying production data.
    pub async fn load_sql_with_client<C: Catalog>(
        catalog: &C,
        schema_name: &str,
    ) -> Result<Namespace, PgLiteError> {
        let namespace = load_namespace(catalog, schema_name).await?;
        Ok(namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = SchemaLoadConfig::default();
        assert_eq!(config.schema_name, "public");
        assert_eq!(config.max_retries, 100);
    }
}
