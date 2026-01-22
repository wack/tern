//! PGLite runtime management.
//!
//! This module provides [`PgLiteRuntime`] which manages the lifecycle of an
//! embedded PostgreSQL instance running via PGLite (PostgreSQL compiled to
//! WebAssembly).
//!
//! # Overview
//!
//! PGLite allows running a real PostgreSQL database entirely in-memory without
//! any external dependencies. This is perfect for:
//!
//! - Schema validation without a running database
//! - Testing DDL changes in isolation
//! - Model-first migration workflows
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                         PgLiteRuntime                                │
//! │                                                                      │
//! │   ┌────────────────┐     ┌────────────────┐     ┌───────────────┐  │
//! │   │  pglite-oxide  │────▶│  Unix Socket   │────▶│ tokio-postgres│  │
//! │   │   (WASM PG)    │     │  Proxy Server  │     │    Client     │  │
//! │   └────────────────┘     └────────────────┘     └───────────────┘  │
//! │                                                                      │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use tern::db::pglite::PgLiteRuntime;
//!
//! // Create and start a new PGLite instance
//! let mut runtime = PgLiteRuntime::new()?;
//! runtime.start().await?;
//!
//! // Get a PostgreSQL client connection
//! let client = runtime.client().await?;
//!
//! // Execute SQL
//! client.batch_execute("CREATE TABLE users (id SERIAL PRIMARY KEY)").await?;
//!
//! // The runtime is automatically cleaned up when dropped
//! ```

use std::path::PathBuf;

use tokio_postgres::Client;

use super::error::PgLiteError;

/// Manages an embedded PostgreSQL instance via PGLite.
///
/// The runtime handles:
/// - Installing the PGLite WASM runtime (if not already present)
/// - Initializing a fresh database cluster
/// - Starting a proxy server that exposes a PostgreSQL-compatible socket
/// - Providing tokio-postgres client connections
/// - Cleanup when dropped
///
/// # Thread Safety
///
/// The runtime is thread-safe and can be shared across async tasks.
/// However, only one client connection should be active at a time for
/// a single-user embedded database.
///
/// # Temporary Storage
///
/// By default, the runtime uses a temporary directory for the database
/// cluster, which is automatically cleaned up when the runtime is dropped.
pub struct PgLiteRuntime {
    /// Paths to the PGLite installation.
    paths: Option<pglite_oxide::PglitePaths>,

    /// Temporary directory for this runtime instance.
    temp_dir: Option<tempfile::TempDir>,

    /// Whether the proxy has been started.
    started: bool,

    /// Background task handle for the proxy server.
    proxy_handle: Option<tokio::task::JoinHandle<()>>,
}

impl PgLiteRuntime {
    /// Creates a new PGLite runtime instance.
    ///
    /// This does not start the database; call [`start()`](Self::start) to
    /// initialize and start the instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the PGLite runtime cannot be installed or
    /// the temporary directory cannot be created.
    pub fn new() -> Result<Self, PgLiteError> {
        // Create a temporary directory for this instance
        let temp_dir = tempfile::TempDir::new().map_err(|e| PgLiteError::RuntimeInit {
            message: format!("failed to create temporary directory: {}", e),
        })?;

        Ok(Self {
            paths: None,
            temp_dir: Some(temp_dir),
            started: false,
            proxy_handle: None,
        })
    }

    /// Creates a new PGLite runtime with a custom data directory.
    ///
    /// This is useful for persistent databases or for placing the runtime
    /// in a specific location.
    ///
    /// # Arguments
    ///
    /// * `data_dir` - Directory to store the database cluster
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or accessed.
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Result<Self, PgLiteError> {
        let data_dir = data_dir.into();

        // Ensure directory exists
        std::fs::create_dir_all(&data_dir).map_err(|e| PgLiteError::RuntimeInit {
            message: format!(
                "failed to create data directory {}: {}",
                data_dir.display(),
                e
            ),
        })?;

        Ok(Self {
            paths: None,
            temp_dir: None,
            started: false,
            proxy_handle: None,
        })
    }

    /// Starts the PGLite instance.
    ///
    /// This method:
    /// 1. Installs the PGLite WASM runtime (if needed)
    /// 2. Initializes a fresh database cluster
    /// 3. Starts the proxy server for client connections
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The PGLite runtime cannot be installed
    /// - Database initialization fails
    /// - The proxy server cannot start
    pub async fn start(&mut self) -> Result<(), PgLiteError> {
        if self.started {
            return Ok(());
        }

        tracing::debug!("Starting PGLite runtime");

        // Determine the installation path
        let install_path = if let Some(ref temp_dir) = self.temp_dir {
            temp_dir.path().to_path_buf()
        } else {
            // Use a default location if no temp dir
            PathBuf::from(".tern/pglite")
        };

        // Install and initialize PGLite
        let install_path_clone = install_path.clone();
        // IMPORTANT: Must use spawn_blocking here because pglite_oxide::install_and_init_in()
        // internally uses wasmtime to compile WebAssembly, and wasmtime attempts to create
        // its own async runtime. Since we're already inside a tokio runtime (from #[tokio::main]),
        // this would cause a "Cannot start a runtime from within a runtime" panic.
        // spawn_blocking runs this on a dedicated thread pool for blocking operations,
        // preventing the nested runtime error.
        let paths = tokio::task::spawn_blocking(move || {
            pglite_oxide::install_and_init_in(&install_path_clone)
        })
        .await
        .map_err(|e| PgLiteError::RuntimeInit {
            message: format!("failed to spawn blocking task: {}", e),
        })?
        .map_err(|e| PgLiteError::RuntimeInit {
            message: format!("failed to install PGLite: {}", e),
        })?;

        self.paths = Some(paths);

        // Start the proxy server in a background task
        // The proxy exposes a Unix socket at /tmp/.s.PGSQL.5432 (or similar)
        let proxy_handle = tokio::spawn(async move {
            if let Err(e) = pglite_oxide::interactive::start_proxy(false).await {
                tracing::error!("PGLite proxy error: {}", e);
            }
        });

        self.proxy_handle = Some(proxy_handle);

        // Give the proxy a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        self.started = true;

        tracing::debug!("PGLite runtime started successfully");

        Ok(())
    }

    /// Gets a PostgreSQL client connection to the PGLite instance.
    ///
    /// Each call creates a new connection to the PGLite proxy server.
    /// The connection uses the proxy server started by [`start()`](Self::start).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The runtime hasn't been started
    /// - The connection fails
    pub async fn client(&self) -> Result<Client, PgLiteError> {
        if !self.started {
            return Err(PgLiteError::RuntimeStart {
                message: "runtime not started; call start() first".to_string(),
            });
        }

        // Create a new connection
        // PGLite proxy listens on a Unix socket
        let connection_string = "host=/tmp dbname=postgres user=postgres";

        let (client, connection) =
            tokio_postgres::connect(connection_string, tokio_postgres::NoTls)
                .await
                .map_err(|e| PgLiteError::Connection { source: e })?;

        // Spawn the connection handler
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("PGLite connection error: {}", e);
            }
        });

        Ok(client)
    }

    /// Resets the database to a clean state.
    ///
    /// This drops all user-created objects in the public schema while
    /// preserving the database structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the reset operation fails.
    pub async fn reset(&self) -> Result<(), PgLiteError> {
        let client = self.client().await?;

        // Drop and recreate the public schema
        client
            .batch_execute(
                "DROP SCHEMA IF EXISTS public CASCADE; \
                 CREATE SCHEMA public; \
                 GRANT ALL ON SCHEMA public TO postgres; \
                 GRANT ALL ON SCHEMA public TO public;",
            )
            .await
            .map_err(|e| PgLiteError::ExecuteSql { source: e })?;

        Ok(())
    }

    /// Checks if the runtime has been started.
    #[must_use]
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// Returns the path to the PGLite data directory.
    #[must_use]
    pub fn data_path(&self) -> Option<PathBuf> {
        self.paths.as_ref().map(|p| p.pgdata.clone())
    }
}

impl Drop for PgLiteRuntime {
    fn drop(&mut self) {
        // Abort the proxy task if it's running
        if let Some(handle) = self.proxy_handle.take() {
            handle.abort();
        }

        tracing::debug!("PGLite runtime dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_creation() {
        // This test just verifies the struct can be created
        // Full integration tests require the pglite feature
        let runtime = PgLiteRuntime::new();
        assert!(runtime.is_ok());

        let rt = runtime.unwrap();
        assert!(!rt.is_started());
    }
}
