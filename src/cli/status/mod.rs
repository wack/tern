//! Status command for viewing state backend status.
//!
//! This command displays the current state of the migration backend,
//! including migration count, current state hash, and last migration info.

use std::path::PathBuf;

use clap::Args;
use miette::{IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, print_json};
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::output;

/// Show state backend status
///
/// Displays information about the current state backend, including
/// migration count, state hash, and schema summary.
#[derive(Debug, Clone, Args)]
pub struct Status {
    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,
}

/// Status output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct StatusOutput {
    /// Whether the state backend is initialized.
    pub initialized: bool,
    /// Path to the state directory.
    pub state_directory: String,
    /// Number of migrations applied.
    pub migration_count: usize,
    /// Current state hash (hex).
    pub current_state_hash: String,
    /// Last migration info, if any.
    pub last_migration: Option<LastMigrationInfo>,
    /// Current schema summary.
    pub schema_summary: Option<SchemaSummary>,
}

/// Information about the last migration.
#[derive(Debug, Clone, Serialize)]
pub struct LastMigrationInfo {
    /// Migration ID (short hex).
    pub id: String,
    /// Migration description.
    pub description: String,
    /// When the migration was created.
    pub created_at: String,
    /// Whether it has breaking changes.
    pub has_breaking_changes: bool,
}

/// Summary of the current schema state.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaSummary {
    /// Schema name.
    pub name: String,
    /// Number of tables.
    pub table_count: usize,
    /// Number of views.
    pub view_count: usize,
    /// Number of enums.
    pub enum_count: usize,
    /// Number of sequences.
    pub sequence_count: usize,
}

impl std::fmt::Display for StatusOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Tern State Backend")?;
        writeln!(f, "==================")?;
        writeln!(f)?;
        writeln!(
            f,
            "  Status:          {}",
            if self.initialized {
                "Initialized"
            } else {
                "Not initialized"
            }
        )?;
        writeln!(f, "  State directory: {}", self.state_directory)?;
        writeln!(f, "  Migrations:      {}", self.migration_count)?;
        writeln!(f, "  State hash:      {}", self.current_state_hash)?;

        if let Some(ref last) = self.last_migration {
            writeln!(f)?;
            writeln!(f, "Last Migration")?;
            writeln!(f, "--------------")?;
            writeln!(f, "  ID:          {}", last.id)?;
            writeln!(f, "  Description: {}", last.description)?;
            writeln!(f, "  Created:     {}", last.created_at)?;
            if last.has_breaking_changes {
                writeln!(f, "  Warning:     Has breaking changes")?;
            }
        }

        if let Some(ref schema) = self.schema_summary {
            writeln!(f)?;
            writeln!(f, "Current Schema ({})", schema.name)?;
            writeln!(f, "--------------")?;
            writeln!(f, "  Tables:    {}", schema.table_count)?;
            writeln!(f, "  Views:     {}", schema.view_count)?;
            writeln!(f, "  Enums:     {}", schema.enum_count)?;
            writeln!(f, "  Sequences: {}", schema.sequence_count)?;
        }

        Ok(())
    }
}

impl Status {
    /// Dispatch the status command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Determine backend location
        let backend = match self.path.as_deref() {
            Some(p) => LocalFileBackend::at_path(p),
            None => LocalFileBackend::default_location(),
        };

        // Check if initialized
        let initialized = backend.is_initialized().await.into_diagnostic()?;

        if !initialized {
            return Err(miette!(
                "State backend not initialized at {}\n\nRun 'tern init' to initialize a new project.",
                backend.root().display()
            ));
        }

        // Get migration index
        let index = backend.get_migration_index().await.into_diagnostic()?;

        // Get current state hash
        let state_hash = backend.get_current_state_hash().await.into_diagnostic()?;

        // Get last migration info if there are migrations
        let last_migration = if !index.is_empty() {
            let last_id = index.last().unwrap();
            let migration = backend.get_migration(last_id).await.into_diagnostic()?;
            Some(LastMigrationInfo {
                id: migration.id.to_short_hex(),
                description: migration.description.clone(),
                created_at: migration.created_at.to_string(),
                has_breaking_changes: migration.has_breaking_changes(),
            })
        } else {
            None
        };

        // Get schema summary if state exists
        let schema_summary = match backend.get_current_state().await {
            Ok(state) => Some(SchemaSummary {
                name: state.name.to_string(),
                table_count: state.tables.len(),
                view_count: state.views.len(),
                enum_count: state.enums.len(),
                sequence_count: state.sequences.len(),
            }),
            Err(_) => None,
        };

        let status_output = StatusOutput {
            initialized,
            state_directory: backend.root().display().to_string(),
            migration_count: index.len(),
            current_state_hash: if state_hash.is_zero() {
                "(empty)".to_string()
            } else {
                state_hash.to_short_hex()
            },
            last_migration,
            schema_summary,
        };

        match self.format {
            OutputFormat::Text | OutputFormat::Sql => output!("{}", status_output),
            OutputFormat::Json => print_json(&status_output),
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::state::init_empty;
    use tempfile::TempDir;

    #[tokio::test]
    async fn status_fails_if_not_initialized() {
        let temp_dir = TempDir::new().unwrap();

        let status = Status {
            format: OutputFormat::Text,
            path: Some(temp_dir.path().to_path_buf()),
        };
        let result = status.dispatch().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn status_succeeds_after_init() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize
        init_empty(&backend, "public").await.unwrap();

        // Status should succeed
        let status = Status {
            format: OutputFormat::Text,
            path: Some(temp_dir.path().to_path_buf()),
        };
        status.dispatch().await.unwrap();
    }

    #[test]
    fn status_output_display() {
        let output = StatusOutput {
            initialized: true,
            state_directory: ".tern".to_string(),
            migration_count: 5,
            current_state_hash: "abc123".to_string(),
            last_migration: Some(LastMigrationInfo {
                id: "def456".to_string(),
                description: "Add users table".to_string(),
                created_at: "2024-01-15T10:00:00Z".to_string(),
                has_breaking_changes: false,
            }),
            schema_summary: Some(SchemaSummary {
                name: "public".to_string(),
                table_count: 3,
                view_count: 1,
                enum_count: 2,
                sequence_count: 1,
            }),
        };

        let display = format!("{}", output);
        assert!(display.contains("Initialized"));
        assert!(display.contains("5"));
        assert!(display.contains("abc123"));
        assert!(display.contains("Add users table"));
    }

    #[test]
    fn status_output_json_serializes() {
        let output = StatusOutput {
            initialized: true,
            state_directory: ".tern".to_string(),
            migration_count: 1,
            current_state_hash: "abc123".to_string(),
            last_migration: None,
            schema_summary: None,
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"initialized\":true"));
    }
}
