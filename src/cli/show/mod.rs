//! Show command for displaying migration details.
//!
//! This command displays detailed information about a specific migration.

use std::path::PathBuf;

use clap::Args;
use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::state::{LocalFileBackend, MigrationId, StateBackend};
use crate::{newline, output};

/// Show details of a specific migration
///
/// Displays detailed information about a migration, including
/// its operations, state hashes, and breaking changes.
#[derive(Debug, Clone, Args)]
pub struct Show {
    /// Migration ID (full hex or prefix)
    pub migration_id: String,

    /// Output format (text, json, or sql)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,
}

/// Show output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct ShowOutput {
    /// Migration ID (full hex).
    pub id: String,
    /// Short ID for display.
    pub short_id: String,
    /// Migration description.
    pub description: String,
    /// When the migration was created.
    pub created_at: String,
    /// Parent state hash.
    pub parent_state_hash: String,
    /// Resulting state hash.
    pub resulting_state_hash: String,
    /// Number of operations.
    pub operation_count: usize,
    /// Whether it has breaking changes.
    pub has_breaking_changes: bool,
    /// Whether it's a baseline migration.
    pub is_baseline: bool,
    /// Whether it's a checkpoint (includes full state).
    pub is_checkpoint: bool,
    /// Breaking changes, if any.
    pub breaking_changes: Vec<BreakingChangeInfo>,
    /// SQL statements (for SQL format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_statements: Option<Vec<String>>,
}

/// Breaking change information.
#[derive(Debug, Clone, Serialize)]
pub struct BreakingChangeInfo {
    /// Description of the breaking change.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: String,
}

impl std::fmt::Display for ShowOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration Details")?;
        writeln!(f, "=================")?;
        writeln!(f)?;
        writeln!(f, "  ID:             {}", self.id)?;
        writeln!(f, "  Description:    {}", self.description)?;
        writeln!(f, "  Created:        {}", self.created_at)?;
        writeln!(f, "  Operations:     {}", self.operation_count)?;
        writeln!(f)?;
        writeln!(f, "State Transition")?;
        writeln!(f, "----------------")?;
        writeln!(f, "  From: {}", &self.parent_state_hash[..16])?;
        writeln!(f, "  To:   {}", &self.resulting_state_hash[..16])?;
        writeln!(f)?;
        writeln!(f, "Flags")?;
        writeln!(f, "-----")?;
        writeln!(
            f,
            "  Baseline:         {}",
            if self.is_baseline { "Yes" } else { "No" }
        )?;
        writeln!(
            f,
            "  Checkpoint:       {}",
            if self.is_checkpoint { "Yes" } else { "No" }
        )?;
        writeln!(
            f,
            "  Breaking changes: {}",
            if self.has_breaking_changes {
                "Yes"
            } else {
                "No"
            }
        )?;

        if !self.breaking_changes.is_empty() {
            writeln!(f)?;
            writeln!(f, "Breaking Changes")?;
            writeln!(f, "----------------")?;
            for bc in &self.breaking_changes {
                writeln!(f, "  [{}] {}", bc.mitigation, bc.description)?;
            }
        }

        Ok(())
    }
}

impl Show {
    /// Dispatch the show command.
    pub async fn dispatch(self) -> miette::Result<()> {
        // Load the state backend
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Find the migration
        let migration = find_migration(&backend, &self.migration_id).await?;

        // Generate SQL if needed
        let sql_statements = if matches!(self.format, OutputFormat::Sql) {
            let plan = MigrationPlan::from_operations(migration.up_operations.clone());
            let renderer = PostgresRenderer::new(RenderConfig::default());
            let script = plan.render(&renderer);
            Some(
                script
                    .all_statements()
                    .into_iter()
                    .map(String::from)
                    .collect(),
            )
        } else {
            None
        };

        // Build output
        let show_output = ShowOutput {
            id: migration.id.to_hex(),
            short_id: migration.id.to_short_hex(),
            description: migration.description.clone(),
            created_at: migration.created_at.to_string(),
            parent_state_hash: migration.parent_state_hash.to_hex(),
            resulting_state_hash: migration.resulting_state_hash.to_hex(),
            operation_count: migration.operation_count(),
            has_breaking_changes: migration.has_breaking_changes(),
            is_baseline: migration.is_baseline(),
            is_checkpoint: migration.is_checkpoint(),
            breaking_changes: migration
                .breaking_changes
                .iter()
                .map(|bc| BreakingChangeInfo {
                    description: bc.description.clone(),
                    mitigation: bc.mitigation.as_str().to_string(),
                })
                .collect(),
            sql_statements,
        };

        match self.format {
            OutputFormat::Text => output!("{}", show_output),
            OutputFormat::Json => print_json(&show_output),
            OutputFormat::Sql => {
                if let Some(ref statements) = show_output.sql_statements {
                    if statements.is_empty() {
                        output!("-- No SQL statements (baseline migration)");
                    } else {
                        output!(
                            "-- Migration: {} ({})",
                            show_output.short_id,
                            show_output.description
                        );
                        newline!();
                        for stmt in statements {
                            output!("{};", stmt);
                            newline!();
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// Finds a migration by ID or prefix.
async fn find_migration(
    backend: &LocalFileBackend,
    id_str: &str,
) -> miette::Result<crate::db::state::Migration> {
    // Try to parse as a full ID first
    if let Some(id) = MigrationId::from_hex(id_str) {
        return backend
            .get_migration(&id)
            .await
            .into_diagnostic()
            .wrap_err("Migration not found");
    }

    // Otherwise, search by prefix
    let index = backend.get_migration_index().await.into_diagnostic()?;

    // Find migrations that match the prefix
    let matches: Vec<_> = index
        .migrations
        .iter()
        .filter(|id| id.to_hex().starts_with(id_str) || id.to_short_hex().starts_with(id_str))
        .collect();

    match matches.len() {
        0 => Err(miette!("No migration found matching '{}'", id_str)),
        1 => backend
            .get_migration(matches[0])
            .await
            .into_diagnostic()
            .wrap_err("Failed to load migration"),
        n => Err(miette!(
            "Ambiguous migration ID '{}' matches {} migrations. Please provide more characters.",
            id_str,
            n
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::state::init_empty;
    use tempfile::TempDir;

    #[tokio::test]
    async fn show_baseline_migration() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize
        init_empty(&backend, "public").await.unwrap();

        // Get baseline ID
        let index = backend.get_migration_index().await.unwrap();
        let baseline_id = index.last().unwrap().to_hex();

        // Show should work
        let show = Show {
            migration_id: baseline_id,
            format: OutputFormat::Text,
            path: Some(temp_dir.path().to_path_buf()),
        };
        show.dispatch().await.unwrap();
    }

    #[tokio::test]
    async fn show_by_prefix() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize
        init_empty(&backend, "public").await.unwrap();

        // Get baseline ID prefix
        let index = backend.get_migration_index().await.unwrap();
        let prefix = index.last().unwrap().to_hex()[..8].to_string();

        // Show by prefix should work
        let show = Show {
            migration_id: prefix,
            format: OutputFormat::Text,
            path: Some(temp_dir.path().to_path_buf()),
        };
        show.dispatch().await.unwrap();
    }

    #[tokio::test]
    async fn show_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize
        init_empty(&backend, "public").await.unwrap();

        // Non-existent ID should fail
        let show = Show {
            migration_id: "nonexistent".to_string(),
            format: OutputFormat::Text,
            path: Some(temp_dir.path().to_path_buf()),
        };
        let result = show.dispatch().await;
        assert!(result.is_err());
    }

    #[test]
    fn show_output_display() {
        let output = ShowOutput {
            id: "a".repeat(64),
            short_id: "abcd1234".to_string(),
            description: "Test migration".to_string(),
            created_at: "2024-01-15T10:00:00Z".to_string(),
            parent_state_hash: "0".repeat(64),
            resulting_state_hash: "1".repeat(64),
            operation_count: 3,
            has_breaking_changes: true,
            is_baseline: false,
            is_checkpoint: false,
            breaking_changes: vec![BreakingChangeInfo {
                description: "Dropping column".to_string(),
                mitigation: "Destructive".to_string(),
            }],
            sql_statements: None,
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration Details"));
        assert!(display.contains("Test migration"));
        assert!(display.contains("Breaking Changes"));
        assert!(display.contains("Dropping column"));
    }
}
