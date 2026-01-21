//! Initialize state backend command.
//!
//! This command initializes a new Tern project with state backend, either
//! from an existing database or with an empty schema.

use miette::{Context, IntoDiagnostic, miette};

use crate::db::query::PostgresCatalog;
use crate::db::state::{LocalFileBackend, StateBackend, init_empty, init_from_database};
use crate::db::{self};

/// Runs the init command.
///
/// Initializes a new Tern project with a state backend. If a database URL
/// is provided, the current schema is captured as the baseline. Otherwise,
/// an empty schema is created.
///
/// # Arguments
///
/// * `from` - Optional database URL to initialize from
/// * `schema` - Database schema name (default: "public")
/// * `path` - Optional path for the state directory (default: current directory)
pub async fn run_init(
    from: Option<String>,
    schema: &str,
    path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Determine backend location
    let backend = match path {
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
    let baseline = match from {
        Some(database_url) => {
            println!("Connecting to database...");

            // Connect to the database
            let client = db::connect(&database_url)
                .await
                .into_diagnostic()
                .wrap_err("Failed to connect to database")?;

            let catalog = PostgresCatalog::new(&client);

            println!("Capturing schema '{}'...", schema);

            // Initialize from database
            let baseline = init_from_database(&backend, &catalog, schema)
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
            init_empty(&backend, schema)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_empty_succeeds() {
        let temp_dir = TempDir::new().unwrap();

        run_init(None, "public", Some(temp_dir.path()))
            .await
            .unwrap();

        // Verify state directory was created
        assert!(temp_dir.path().join(".tern").exists());
        assert!(temp_dir.path().join(".tern/migrations").exists());
        assert!(temp_dir.path().join(".tern/migrations/index.json").exists());
    }

    #[tokio::test]
    async fn init_fails_if_already_initialized() {
        let temp_dir = TempDir::new().unwrap();

        // First init should succeed
        run_init(None, "public", Some(temp_dir.path()))
            .await
            .unwrap();

        // Second init should fail
        let result = run_init(None, "public", Some(temp_dir.path())).await;
        assert!(result.is_err());
    }
}
