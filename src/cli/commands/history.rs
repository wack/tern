//! History command for listing migration history.
//!
//! This command displays the migration history from the state backend.

use miette::IntoDiagnostic;
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::state::StateBackend;

/// History output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryOutput {
    /// Total number of migrations.
    pub total: usize,
    /// Migrations displayed (may be limited).
    pub displayed: usize,
    /// Migration entries.
    pub migrations: Vec<MigrationEntry>,
}

/// Entry for a single migration in history.
#[derive(Debug, Clone, Serialize)]
pub struct MigrationEntry {
    /// Sequence number (1-indexed).
    pub number: usize,
    /// Migration ID (short hex).
    pub id: String,
    /// Full migration ID (for JSON).
    pub full_id: String,
    /// Migration description.
    pub description: String,
    /// When the migration was created.
    pub created_at: String,
    /// Number of operations.
    pub operation_count: usize,
    /// Whether it has breaking changes.
    pub has_breaking_changes: bool,
    /// Whether it's a baseline migration.
    pub is_baseline: bool,
    /// Whether it's a checkpoint (includes full state).
    pub is_checkpoint: bool,
}

impl std::fmt::Display for HistoryOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.migrations.is_empty() {
            writeln!(f, "No migrations recorded.")?;
            return Ok(());
        }

        writeln!(f, "Migration History")?;
        writeln!(f, "=================")?;
        writeln!(f)?;

        // Header
        writeln!(
            f,
            "{:>4}  {:16}  {:19}  {:6}  Description",
            "#", "ID", "Created", "Ops"
        )?;
        writeln!(f, "{}", "-".repeat(80))?;

        for entry in &self.migrations {
            let flags = if entry.is_baseline {
                "[baseline]"
            } else if entry.has_breaking_changes {
                "[breaking]"
            } else {
                ""
            };

            // Truncate description if too long
            let desc = if entry.description.len() > 35 {
                format!("{}...", &entry.description[..32])
            } else {
                entry.description.clone()
            };

            writeln!(
                f,
                "{:>4}  {:16}  {:19}  {:>6}  {} {}",
                entry.number,
                entry.id,
                format_timestamp(&entry.created_at),
                entry.operation_count,
                desc,
                flags
            )?;
        }

        if self.displayed < self.total {
            writeln!(f)?;
            writeln!(
                f,
                "Showing {} of {} migrations. Use --limit to see more.",
                self.displayed, self.total
            )?;
        }

        Ok(())
    }
}

/// Format a timestamp for display.
fn format_timestamp(ts: &str) -> String {
    // Try to parse and format nicely, fallback to original
    if let Some(idx) = ts.find('T') {
        let date = &ts[..idx];
        let time = ts.get(idx + 1..idx + 9).unwrap_or("");
        format!("{} {}", date, time)
    } else {
        ts.to_string()
    }
}

/// Runs the history command.
///
/// Lists the migration history from the state backend.
///
/// # Arguments
///
/// * `format` - Output format (text or json)
/// * `limit` - Maximum number of migrations to display
/// * `state_path` - Optional path to the state directory
pub async fn run_history(
    format: OutputFormat,
    limit: Option<usize>,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Get all migrations
    let all_migrations = backend.get_all_migrations().await.into_diagnostic()?;

    let total = all_migrations.len();

    // Apply limit if specified
    let migrations_to_show = match limit {
        Some(n) => &all_migrations[all_migrations.len().saturating_sub(n)..],
        None => &all_migrations[..],
    };

    // Build output entries
    let migrations: Vec<MigrationEntry> = migrations_to_show
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            let number = match limit {
                Some(n) => total.saturating_sub(n) + idx + 1,
                None => idx + 1,
            };
            MigrationEntry {
                number,
                id: m.id.to_short_hex(),
                full_id: m.id.to_hex(),
                description: m.description.clone(),
                created_at: m.created_at.to_string(),
                operation_count: m.operation_count(),
                has_breaking_changes: m.has_breaking_changes(),
                is_baseline: m.is_baseline(),
                is_checkpoint: m.is_checkpoint(),
            }
        })
        .collect();

    let output = HistoryOutput {
        total,
        displayed: migrations.len(),
        migrations,
    };

    match format {
        OutputFormat::Text | OutputFormat::Sql => println!("{}", output),
        OutputFormat::Json => print_json(&output),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::Namespace;
    use crate::db::state::{Migration, init_empty};
    use tempfile::TempDir;

    #[tokio::test]
    async fn history_empty() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize with empty state
        init_empty(&backend, "public").await.unwrap();

        // History should show the baseline
        run_history(OutputFormat::Text, None, Some(temp_dir.path()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn history_with_migrations() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::at_path(temp_dir.path());

        // Initialize
        init_empty(&backend, "public").await.unwrap();

        // Add another migration
        let current_hash = backend.get_current_state_hash().await.unwrap();
        let _ns = Namespace::empty("public");
        let m = Migration::new(
            "Second migration",
            vec![],
            current_hash,
            crate::db::state::StateHash::from_bytes([1u8; 32]),
            vec![],
        );
        backend.save_migration(&m).await.unwrap();

        // History should show both
        run_history(OutputFormat::Text, None, Some(temp_dir.path()))
            .await
            .unwrap();
    }

    #[test]
    fn history_output_display() {
        let output = HistoryOutput {
            total: 3,
            displayed: 3,
            migrations: vec![
                MigrationEntry {
                    number: 1,
                    id: "abc123".to_string(),
                    full_id: "abc123".repeat(10),
                    description: "Baseline migration".to_string(),
                    created_at: "2024-01-15T10:00:00Z".to_string(),
                    operation_count: 0,
                    has_breaking_changes: false,
                    is_baseline: true,
                    is_checkpoint: true,
                },
                MigrationEntry {
                    number: 2,
                    id: "def456".to_string(),
                    full_id: "def456".repeat(10),
                    description: "Add users table".to_string(),
                    created_at: "2024-01-16T10:00:00Z".to_string(),
                    operation_count: 1,
                    has_breaking_changes: false,
                    is_baseline: false,
                    is_checkpoint: false,
                },
            ],
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration History"));
        assert!(display.contains("abc123"));
        assert!(display.contains("Baseline migration"));
        assert!(display.contains("[baseline]"));
    }

    #[test]
    fn format_timestamp_works() {
        assert_eq!(
            format_timestamp("2024-01-15T10:30:45.123Z"),
            "2024-01-15 10:30:45"
        );
        assert_eq!(format_timestamp("invalid"), "invalid");
    }
}
