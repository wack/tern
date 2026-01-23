//! Record command for manually recording migrations.
//!
//! This command allows recording a migration as applied to the state backend
//! without actually executing it. This is useful for:
//! - Recording externally applied migrations
//! - Synchronizing state backends across environments
//! - Manual state management
//!
//! # Schema Drift Warning
//!
//! Before using this command, ensure your database is in sync with the state
//! backend by running `tern verify`. Recording migrations when schema drift
//! exists may create inconsistencies between the recorded state and the actual
//! database schema.

use std::path::PathBuf;

use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::state::{Migration, MigrationId, StateBackend, StateHash};

/// Record output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct RecordOutput {
    /// Whether the operation was successful.
    pub success: bool,
    /// Migration ID that was recorded.
    pub migration_id: String,
    /// Description of the recorded migration.
    pub description: String,
    /// New state hash after recording.
    pub new_state_hash: String,
    /// Message describing what was done.
    pub message: String,
}

impl std::fmt::Display for RecordOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration Recorded")?;
        writeln!(f, "==================")?;
        writeln!(f)?;
        writeln!(f, "  ID:          {}", &self.migration_id[..16])?;
        writeln!(f, "  Description: {}", self.description)?;
        writeln!(f, "  State hash:  {}", &self.new_state_hash[..16])?;
        writeln!(f)?;
        writeln!(f, "{}", self.message)?;
        Ok(())
    }
}

/// Runs the record command.
///
/// Records a migration as applied to the state backend. The migration can be
/// specified either by ID (if it already exists in the backend) or by loading
/// from a file.
///
/// # Arguments
///
/// * `migration_id` - Optional migration ID to mark as applied
/// * `migration_file` - Optional path to a migration JSON file
/// * `format` - Output format
/// * `state_path` - Optional path to the state directory
pub async fn run_record(
    migration_id: Option<&str>,
    migration_file: Option<PathBuf>,
    format: OutputFormat,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Must provide one of migration_id or migration_file
    if migration_id.is_none() && migration_file.is_none() {
        return Err(miette!(
            "Must provide either --migration-id or --migration-file"
        ));
    }

    if migration_id.is_some() && migration_file.is_some() {
        return Err(miette!(
            "Cannot provide both --migration-id and --migration-file"
        ));
    }

    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Load the migration
    let (migration, new_state) = if let Some(file_path) = migration_file {
        // Load migration from file
        let content = std::fs::read_to_string(&file_path)
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to read migration file: {}", file_path.display()))?;

        let migration: Migration = serde_json::from_str(&content)
            .into_diagnostic()
            .wrap_err("Failed to parse migration JSON")?;

        // Verify the migration chain
        let current_hash = backend.get_current_state_hash().await.into_diagnostic()?;
        if migration.parent_state_hash != current_hash {
            return Err(miette!(
                "Migration parent hash ({}) does not match current state ({})",
                migration.parent_state_hash.to_short_hex(),
                current_hash.to_short_hex()
            ));
        }

        // If the migration has a checkpoint state, use it; otherwise reconstruct
        let new_state = if let Some(ref state) = migration.checkpoint_state {
            state.clone()
        } else {
            // We need to apply the operations to the current state
            let current_state = backend.get_current_state().await.into_diagnostic()?;
            current_state
                .apply(&migration.operations)
                .into_diagnostic()
                .wrap_err("Failed to apply migration operations")?
        };

        (migration, new_state)
    } else if let Some(id_str) = migration_id {
        // Look up existing migration by ID
        let id = parse_migration_id(id_str)?;

        // Check if migration exists in backend
        let migration = match backend.get_migration(&id).await {
            Ok(m) => m,
            Err(_) => {
                return Err(miette!(
                    "Migration {} not found in state backend.\n\nTo record an external migration, use --migration-file.",
                    id_str
                ));
            }
        };

        // Get the state at this migration
        let new_state = backend.get_state_at(&id).await.into_diagnostic()?;

        (migration, new_state)
    } else {
        unreachable!("Validated above");
    };

    // Record the migration
    backend
        .record_migration(&migration, &new_state)
        .await
        .into_diagnostic()
        .wrap_err("Failed to record migration")?;

    let output = RecordOutput {
        success: true,
        migration_id: migration.id.to_hex(),
        description: migration.description.clone(),
        new_state_hash: migration.resulting_state_hash.to_hex(),
        message: "Migration has been recorded to the state backend.".to_string(),
    };

    match format {
        OutputFormat::Text | OutputFormat::Sql => println!("{}", output),
        OutputFormat::Json => print_json(&output),
    }

    Ok(())
}

/// Parse a migration ID from a string (full hex or prefix).
fn parse_migration_id(s: &str) -> miette::Result<MigrationId> {
    // Try full hex first
    if let Some(id) = MigrationId::from_hex(s) {
        return Ok(id);
    }

    // Try hex prefix (for convenience)
    if let Some(id) = MigrationId::from_hex_prefix(s) {
        return Ok(id);
    }

    Err(miette!(
        "Invalid migration ID: '{}'. Expected a 64-character hex string.",
        s
    ))
}

/// Create a simple migration for recording (used when syncing state).
#[allow(dead_code)]
pub fn create_sync_migration(
    description: &str,
    parent_hash: StateHash,
    resulting_hash: StateHash,
) -> Migration {
    Migration::new(description, vec![], parent_hash, resulting_hash, vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::Namespace;
    use crate::db::state::{LocalFileBackend, StateBackend, init_empty};
    use tempfile::TempDir;

    #[tokio::test]
    async fn record_requires_id_or_file() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());
        init_empty(&backend, "public").await.unwrap();

        let result = run_record(None, None, OutputFormat::Text, Some(temp_dir.path())).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn record_rejects_both_id_and_file() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());
        init_empty(&backend, "public").await.unwrap();

        let result = run_record(
            Some("abc123"),
            Some(PathBuf::from("test.json")),
            OutputFormat::Text,
            Some(temp_dir.path()),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn record_from_file() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());
        init_empty(&backend, "public").await.unwrap();

        // Get current state hash
        let current_hash = backend.get_current_state_hash().await.unwrap();

        // Create a migration file
        let ns = Namespace::empty("public");
        let migration = Migration::new(
            "Test migration",
            vec![],
            current_hash,
            StateHash::from_bytes([1u8; 32]),
            vec![],
        )
        .with_checkpoint(ns);

        let migration_file = temp_dir.path().join("migration.json");
        std::fs::write(
            &migration_file,
            serde_json::to_string_pretty(&migration).unwrap(),
        )
        .unwrap();

        // Record should succeed
        run_record(
            None,
            Some(migration_file),
            OutputFormat::Text,
            Some(temp_dir.path()),
        )
        .await
        .unwrap();

        // Verify migration was recorded
        let index = backend.get_migration_index().await.unwrap();
        assert_eq!(index.len(), 2); // baseline + new migration
    }

    #[test]
    fn parse_migration_id_full() {
        let id_str = "a".repeat(64);
        let id = parse_migration_id(&id_str).unwrap();
        assert_eq!(id.to_hex(), id_str);
    }

    #[test]
    fn parse_migration_id_prefix() {
        let id_str = "abcd";
        let id = parse_migration_id(id_str).unwrap();
        assert!(id.to_hex().starts_with("abcd"));
    }

    #[test]
    fn parse_migration_id_invalid() {
        let result = parse_migration_id("not-hex");
        assert!(result.is_err());
    }

    #[test]
    fn record_output_display() {
        let output = RecordOutput {
            success: true,
            migration_id: "a".repeat(64),
            description: "Test migration".to_string(),
            new_state_hash: "b".repeat(64),
            message: "Migration recorded.".to_string(),
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration Recorded"));
        assert!(display.contains("Test migration"));
    }
}
