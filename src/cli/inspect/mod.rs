//! Inspect command for examining compiled migration artifacts.
//!
//! This command inspects migration source files or JSON migration files
//! to display their contents and metadata.

use std::path::PathBuf;

use clap::Args;
use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, print_json};
use crate::db::state::Migration;
use crate::{newline, output, warn};

/// Inspect a compiled migration file
///
/// Examines a migration source file or JSON file and displays
/// its contents and metadata.
#[derive(Debug, Clone, Args)]
pub struct Inspect {
    /// Path to the file to inspect (.json or .rs)
    pub path: PathBuf,

    /// Output format (text, json, or sql)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

impl Inspect {
    /// Dispatch the inspect command.
    pub async fn dispatch(self) -> miette::Result<()> {
        warn!("'inspect' is deprecated.");

        if !self.path.exists() {
            return Err(miette!("File not found: {}", self.path.display()));
        }

        // Determine file type and inspect accordingly
        let extension = self.path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let inspect_output = match extension.to_lowercase().as_str() {
            "json" => inspect_json_migration(&self.path)?,
            "rs" => inspect_rust_source(&self.path)?,
            _ => {
                return Err(miette!(
                    "Unsupported file type: '{}'. Expected .json or .rs",
                    extension
                ));
            }
        };

        match self.format {
            OutputFormat::Text => output!("{}", inspect_output),
            OutputFormat::Json => print_json(&inspect_output),
            OutputFormat::Sql => {
                if let Some(ref statements) = inspect_output.sql_statements {
                    output!(
                        "-- Migration: {} ({})",
                        inspect_output.migration_id.as_deref().unwrap_or("unknown"),
                        inspect_output.description.as_deref().unwrap_or("unknown")
                    );
                    newline!();
                    for stmt in statements {
                        output!("{};", stmt);
                        newline!();
                    }
                } else {
                    output!("-- No SQL statements available");
                }
            }
        }

        Ok(())
    }
}

/// Inspect output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct InspectOutput {
    /// Path to the inspected file.
    pub path: String,
    /// File type detected.
    pub file_type: String,
    /// Migration ID (if available).
    pub migration_id: Option<String>,
    /// Migration description (if available).
    pub description: Option<String>,
    /// Created timestamp (if available).
    pub created_at: Option<String>,
    /// Number of operations (if available).
    pub operation_count: Option<usize>,
    /// Whether it has breaking changes.
    pub has_breaking_changes: Option<bool>,
    /// Source state hash (if available).
    pub source_state_hash: Option<String>,
    /// Target state hash (if available).
    pub target_state_hash: Option<String>,
    /// SQL statements (for source files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_statements: Option<Vec<String>>,
    /// Breaking changes details.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breaking_changes: Vec<BreakingChangeDetail>,
}

/// Breaking change detail.
#[derive(Debug, Clone, Serialize)]
pub struct BreakingChangeDetail {
    /// Description of the breaking change.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: String,
}

impl std::fmt::Display for InspectOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration Inspection")?;
        writeln!(f, "====================")?;
        writeln!(f)?;
        writeln!(f, "  File:      {}", self.path)?;
        writeln!(f, "  Type:      {}", self.file_type)?;

        if let Some(ref id) = self.migration_id {
            writeln!(f, "  ID:        {}", &id[..std::cmp::min(16, id.len())])?;
        }

        if let Some(ref desc) = self.description {
            writeln!(f, "  Desc:      {}", desc)?;
        }

        if let Some(ref created) = self.created_at {
            writeln!(f, "  Created:   {}", created)?;
        }

        if let Some(count) = self.operation_count {
            writeln!(f, "  Operations: {}", count)?;
        }

        if let Some(ref src) = self.source_state_hash {
            writeln!(f, "  From state: {}", &src[..std::cmp::min(16, src.len())])?;
        }

        if let Some(ref tgt) = self.target_state_hash {
            writeln!(f, "  To state:   {}", &tgt[..std::cmp::min(16, tgt.len())])?;
        }

        if let Some(has_bc) = self.has_breaking_changes
            && has_bc
        {
            writeln!(f)?;
            writeln!(f, "WARNING: Breaking Changes")?;
            writeln!(f, "-------------------------")?;
            for bc in &self.breaking_changes {
                writeln!(f, "  [{}] {}", bc.mitigation, bc.description)?;
            }
        }

        if let Some(ref statements) = self.sql_statements {
            writeln!(f)?;
            writeln!(f, "SQL Statements ({})", statements.len())?;
            writeln!(f, "--------------")?;
            for (i, stmt) in statements.iter().enumerate() {
                let preview = if stmt.len() > 60 {
                    format!("{}...", &stmt[..60])
                } else {
                    stmt.clone()
                };
                writeln!(f, "  {}. {}", i + 1, preview)?;
            }
        }

        Ok(())
    }
}

/// Inspects a JSON migration file.
fn inspect_json_migration(path: &PathBuf) -> miette::Result<InspectOutput> {
    let content = std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read file: {}", path.display()))?;

    let migration: Migration = serde_json::from_str(&content)
        .into_diagnostic()
        .wrap_err("Failed to parse migration JSON")?;

    // Try to generate SQL from up_operations
    let sql_statements = if !migration.up_operations.is_empty() {
        use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
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

    Ok(InspectOutput {
        path: path.display().to_string(),
        file_type: "Migration JSON".to_string(),
        migration_id: Some(migration.id.to_hex()),
        description: Some(migration.description.clone()),
        created_at: Some(migration.created_at.to_string()),
        operation_count: Some(migration.operation_count()),
        has_breaking_changes: Some(migration.has_breaking_changes()),
        source_state_hash: Some(migration.parent_state_hash.to_hex()),
        target_state_hash: Some(migration.resulting_state_hash.to_hex()),
        sql_statements,
        breaking_changes: migration
            .breaking_changes
            .iter()
            .map(|bc| BreakingChangeDetail {
                description: bc.description.clone(),
                mitigation: bc.mitigation.as_str().to_string(),
            })
            .collect(),
    })
}

/// Inspects a Rust source file containing a migration.
fn inspect_rust_source(path: &PathBuf) -> miette::Result<InspectOutput> {
    let content = std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to read file: {}", path.display()))?;

    // Parse the define_migration! macro to extract information
    let mut migration_id = None;
    let mut description = None;
    let mut source_state_hash = None;
    let mut target_state_hash = None;
    let mut sql_statements = Vec::new();
    let mut breaking_changes = Vec::new();

    // Simple regex-like parsing for the macro content
    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("id:") {
            migration_id = extract_string_value(trimmed, "id:");
        } else if trimmed.starts_with("description:") {
            description = extract_string_value(trimmed, "description:");
        } else if trimmed.starts_with("source_state_hash:") {
            source_state_hash = extract_string_value(trimmed, "source_state_hash:");
        } else if trimmed.starts_with("target_state_hash:") {
            target_state_hash = extract_string_value(trimmed, "target_state_hash:");
        } else if trimmed.starts_with("(\"") && trimmed.contains("\", \"") {
            // This is a statement tuple: ("description", "SQL")
            if let Some(sql) = extract_statement_sql(trimmed) {
                sql_statements.push(sql);
            }
        } else if trimmed.contains("mitigation:") && trimmed.contains("description:") {
            // Breaking change entry
            if let Some(bc) = extract_breaking_change(trimmed) {
                breaking_changes.push(bc);
            }
        }
    }

    Ok(InspectOutput {
        path: path.display().to_string(),
        file_type: "Rust Source".to_string(),
        migration_id,
        description,
        created_at: None,
        operation_count: Some(sql_statements.len()),
        has_breaking_changes: Some(!breaking_changes.is_empty()),
        source_state_hash,
        target_state_hash,
        sql_statements: if sql_statements.is_empty() {
            None
        } else {
            Some(sql_statements)
        },
        breaking_changes,
    })
}

/// Extracts a string value from a line like 'id: "value",'
fn extract_string_value(line: &str, prefix: &str) -> Option<String> {
    let after_prefix = line.strip_prefix(prefix)?.trim();
    let without_quotes = after_prefix.trim_start_matches('"');
    let end_quote = without_quotes.find('"')?;
    Some(without_quotes[..end_quote].to_string())
}

/// Extracts SQL from a statement tuple like '("desc", "SQL"),'
fn extract_statement_sql(line: &str) -> Option<String> {
    // Find the second quoted string (the SQL)
    let mut in_quotes = false;
    let mut string_count = 0;
    let mut start = 0;

    for (i, c) in line.char_indices() {
        if c == '"' && (i == 0 || !line[..i].ends_with('\\')) {
            if !in_quotes {
                // Opening quote
                in_quotes = true;
                string_count += 1;
                if string_count == 2 {
                    // Second string = SQL
                    start = i + 1;
                }
            } else {
                // Closing quote
                in_quotes = false;
                if string_count >= 2 {
                    // End of SQL string
                    return Some(unescape_string(&line[start..i]));
                }
            }
        }
    }
    None
}

/// Extracts a breaking change from a line.
fn extract_breaking_change(line: &str) -> Option<BreakingChangeDetail> {
    // This is a simplified parser - in practice, breaking changes
    // might span multiple lines
    let desc_start = line.find("description: \"")?;
    let desc_content = &line[desc_start + 14..];
    let desc_end = desc_content.find('"')?;
    let description = desc_content[..desc_end].to_string();

    let mit_start = line.find("mitigation: ")?;
    let mit_content = &line[mit_start + 12..];
    let mit_end = mit_content.find([',', '}']).unwrap_or(mit_content.len());
    let mitigation = mit_content[..mit_end].trim().to_string();

    Some(BreakingChangeDetail {
        description,
        mitigation,
    })
}

/// Unescape a Rust string literal.
fn unescape_string(s: &str) -> String {
    s.replace("\\\"", "\"")
        .replace("\\\\", "\\")
        .replace("\\n", "\n")
        .replace("\\t", "\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::Namespace;
    use crate::db::state::Migration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn inspect_json_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("migration.json");

        // Create a test migration
        let ns = Namespace::empty("public");
        let migration = Migration::baseline(ns);
        std::fs::write(
            &file_path,
            serde_json::to_string_pretty(&migration).unwrap(),
        )
        .unwrap();

        // Inspect should succeed
        let inspect = Inspect {
            path: file_path,
            format: OutputFormat::Text,
        };
        inspect.dispatch().await.unwrap();
    }

    #[tokio::test]
    async fn inspect_rust_source() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("migration.rs");

        // Create a test Rust source file
        let source = r#"
use tern_migration_guest::define_migration;

define_migration! {
    id: "abc123",
    description: "Add users table",
    source_state_hash: "000000",
    target_state_hash: "111111",
    compiled_at: "2024-01-15T10:00:00Z",
    breaking_changes: [],
    statements: [
        ("Create users table", "CREATE TABLE users (id INTEGER)"),
    ]
}
"#;
        std::fs::write(&file_path, source).unwrap();

        // Inspect should succeed
        let inspect = Inspect {
            path: file_path,
            format: OutputFormat::Text,
        };
        inspect.dispatch().await.unwrap();
    }

    #[tokio::test]
    async fn inspect_file_not_found() {
        let inspect = Inspect {
            path: PathBuf::from("/nonexistent/file.json"),
            format: OutputFormat::Text,
        };
        let result = inspect.dispatch().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn inspect_unsupported_type() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("migration.txt");
        std::fs::write(&file_path, "test").unwrap();

        let inspect = Inspect {
            path: file_path,
            format: OutputFormat::Text,
        };
        let result = inspect.dispatch().await;
        assert!(result.is_err());
    }

    #[test]
    fn extract_string_value_works() {
        assert_eq!(
            extract_string_value(r#"id: "abc123","#, "id:"),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_string_value(r#"description: "Test migration","#, "description:"),
            Some("Test migration".to_string())
        );
    }

    #[test]
    fn extract_statement_sql_works() {
        let line = r#"        ("Create table", "CREATE TABLE foo ()"),"#;
        assert_eq!(
            extract_statement_sql(line),
            Some("CREATE TABLE foo ()".to_string())
        );
    }

    #[test]
    fn unescape_string_works() {
        assert_eq!(unescape_string(r#"hello \"world\""#), r#"hello "world""#);
        assert_eq!(unescape_string(r#"line1\nline2"#), "line1\nline2");
    }

    #[test]
    fn inspect_output_display() {
        let output = InspectOutput {
            path: "/path/to/migration.json".to_string(),
            file_type: "Migration JSON".to_string(),
            migration_id: Some("abc123".to_string()),
            description: Some("Test migration".to_string()),
            created_at: Some("2024-01-15".to_string()),
            operation_count: Some(3),
            has_breaking_changes: Some(false),
            source_state_hash: Some("000000".to_string()),
            target_state_hash: Some("111111".to_string()),
            sql_statements: Some(vec!["CREATE TABLE foo ()".to_string()]),
            breaking_changes: vec![],
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration Inspection"));
        assert!(display.contains("Test migration"));
        assert!(display.contains("CREATE TABLE foo"));
    }
}
