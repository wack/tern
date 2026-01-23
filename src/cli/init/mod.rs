//! Initialize state backend command.
//!
//! This command initializes a new Tern project with state backend, either
//! from an existing database or with an empty schema.

use std::path::PathBuf;

use anstream::println;
use clap::Args;
use miette::{Context, IntoDiagnostic, miette};

use crate::db::query::PostgresCatalog;
use crate::db::state::{LocalFileBackend, StateBackend, init_empty, init_from_database};
use crate::db::{self};

/// Initialize a new Tern project with state backend
///
/// Creates the .tern/ directory structure and optionally captures
/// the current database schema as the baseline migration.
#[derive(Debug, Clone, Args)]
pub struct Init {
    /// PostgreSQL connection string to initialize from (captures current schema)
    #[arg(long, env = "DATABASE_URL")]
    pub from: Option<String>,

    /// The database schema to capture
    #[arg(long, default_value = "public")]
    pub schema: String,

    /// Path to create the state directory (default: current directory)
    #[arg(long)]
    pub path: Option<PathBuf>,
}

impl Init {
    /// Dispatch the init command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Determine backend location
        let backend = match self.path.as_deref() {
            Some(p) => LocalFileBackend::at_path(p),
            None => LocalFileBackend::default_location(),
        };

        // Check if already initialized
        if backend.is_initialized().await.into_diagnostic()? {
            return Err(miette!(
                "State backend already initialized at {}",
                backend.root().display()
            ));
        }

        // Initialize based on whether we have a database URL
        let baseline = match self.from {
            Some(database_url) => {
                println!("Connecting to database...");

                // Connect to the database
                let client = db::connect(&database_url)
                    .await
                    .into_diagnostic()
                    .wrap_err("Failed to connect to database")?;

                let catalog = PostgresCatalog::new(&client);

                println!("Capturing schema '{}'...", self.schema);

                // Initialize from database
                let baseline = init_from_database(&backend, &catalog, &self.schema)
                    .await
                    .into_diagnostic()
                    .wrap_err("Failed to initialize from database")?;

                let state = baseline.checkpoint_state.as_ref().unwrap();
                println!(
                    "Captured {} table(s), {} view(s), {} enum(s), {} sequence(s)",
                    state.tables.len(),
                    state.views.len(),
                    state.enums.len(),
                    state.sequences.len()
                );

                baseline
            }
            None => {
                println!("Initializing empty state backend...");

                // Initialize with empty schema
                init_empty(&backend, &self.schema)
                    .await
                    .into_diagnostic()
                    .wrap_err("Failed to initialize empty state")?
            }
        };

        println!();
        println!("Tern initialized successfully!");
        println!();
        println!("  State directory: {}", backend.root().display());
        println!("  Baseline migration: {}", baseline.id.to_short_hex());
        println!(
            "  State hash: {}",
            baseline.resulting_state_hash.to_short_hex()
        );
        println!();
        println!(
            "Run 'tern status' to view the current state or 'tern compile' to create a migration."
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_empty_succeeds() {
        let temp_dir = TempDir::new().unwrap();

        let init = Init {
            from: None,
            schema: "public".to_string(),
            path: Some(temp_dir.path().to_path_buf()),
        };
        init.dispatch().await.unwrap();

        // Verify state directory was created
        assert!(temp_dir.path().join(".tern").exists());
        assert!(temp_dir.path().join(".tern/migrations").exists());
        assert!(temp_dir.path().join(".tern/migrations/index.json").exists());
    }

    #[tokio::test]
    async fn init_fails_if_already_initialized() {
        let temp_dir = TempDir::new().unwrap();

        // First init should succeed
        let init = Init {
            from: None,
            schema: "public".to_string(),
            path: Some(temp_dir.path().to_path_buf()),
        };
        init.dispatch().await.unwrap();

        // Second init should fail
        let init2 = Init {
            from: None,
            schema: "public".to_string(),
            path: Some(temp_dir.path().to_path_buf()),
        };
        let result = init2.dispatch().await;
        assert!(result.is_err());
    }
}
