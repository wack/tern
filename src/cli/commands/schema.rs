//! Schema-related CLI commands.
//!
//! This module implements commands for working with the schema DDL file,
//! which is the foundation of the model-first migration workflow.

use std::path::PathBuf;

use miette::IntoDiagnostic;

use super::{OutputFormat, ensure_backend_initialized, load_backend};
use crate::db::state::{SchemaExporter, StateBackend};

/// Export the current schema as SQL DDL.
///
/// This command generates a `.tern/schema.sql` file containing SQL DDL
/// statements that would recreate the current schema from scratch. The
/// exported schema is useful for:
///
/// - Viewing the current schema in a human-readable format
/// - Model-first migration workflows (editing schema.sql to define changes)
/// - Documentation and code review
/// - Understanding the complete database structure
///
/// # Arguments
///
/// * `output` - Optional path to write the schema (default: `.tern/schema.sql`)
/// * `state_path` - Optional path to the state directory (default: `.tern/`)
/// * `format` - Output format (text or json for metadata)
///
/// # Example
///
/// ```bash
/// # Export to default location (.tern/schema.sql)
/// tern schema export
///
/// # Export to a custom location
/// tern schema export --output schema.sql
///
/// # Export from a specific state directory
/// tern schema export --path /path/to/project
/// ```
pub async fn run_schema_export(
    output: Option<PathBuf>,
    state_path: Option<&std::path::Path>,
    format: OutputFormat,
) -> miette::Result<()> {
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Load the current schema state
    let namespace = backend.get_current_state().await.into_diagnostic()?;

    // Generate the SQL DDL
    let sql = SchemaExporter::export(&namespace);

    // Determine output location
    let output_path = output.unwrap_or_else(|| backend.schema_path());

    // Write to file or stdout
    if output_path == std::path::Path::new("-") {
        // Write to stdout
        match format {
            OutputFormat::Sql | OutputFormat::Text => {
                println!("{sql}");
            }
            OutputFormat::Json => {
                // For JSON format, wrap in a structured output
                let output = SchemaExportOutput {
                    schema: namespace.name.as_ref().to_string(),
                    sql: sql.clone(),
                    path: None,
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).into_diagnostic()?
                );
            }
        }
    } else {
        // Write to file
        std::fs::write(&output_path, &sql)
            .into_diagnostic()
            .map_err(|e| miette::miette!("Failed to write schema file: {}", e))?;

        match format {
            OutputFormat::Text | OutputFormat::Sql => {
                println!("Schema exported to: {}", output_path.display());
                println!();
                println!("Schema: {}", namespace.name.as_ref());
                println!("Tables: {}", namespace.tables.len());
                println!("Views: {}", namespace.views.len());
                println!("Sequences: {}", namespace.sequences.len());
                println!("Enums: {}", namespace.enums.len());
            }
            OutputFormat::Json => {
                let output = SchemaExportOutput {
                    schema: namespace.name.as_ref().to_string(),
                    sql,
                    path: Some(output_path.to_string_lossy().to_string()),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&output).into_diagnostic()?
                );
            }
        }
    }

    Ok(())
}

/// Output structure for JSON format.
#[derive(Debug, serde::Serialize)]
struct SchemaExportOutput {
    /// The schema name (e.g., "public").
    schema: String,
    /// The generated SQL DDL.
    sql: String,
    /// The path where the schema was written (None if stdout).
    path: Option<String>,
}
