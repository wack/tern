//! Schema export command.
//!
//! This command exports the current schema state as SQL DDL.

use std::path::PathBuf;

use clap::Args;
use miette::IntoDiagnostic;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend};
use crate::db::state::{SchemaExporter, StateBackend};
use crate::{info, newline, output};

/// Export the current schema as SQL DDL
///
/// Generates a schema.sql file containing SQL DDL statements that
/// would recreate the current schema from scratch. This file is
/// useful for viewing the schema, documentation, and the model-first
/// migration workflow.
#[derive(Debug, Clone, Args)]
pub struct Export {
    /// Output path for the schema file (default: .tern/schema.sql, use "-" for stdout)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Output format (text shows summary, sql shows DDL, json includes metadata)
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,
}

impl Export {
    /// Dispatch the schema export command.
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Load the current schema state
        let namespace = backend.get_current_state().await.into_diagnostic()?;

        // Generate the SQL DDL
        let sql = SchemaExporter::export(&namespace);

        // Determine output location
        let output_path = self.output.unwrap_or_else(|| backend.schema_path());

        // Write to file or stdout
        if output_path == std::path::Path::new("-") {
            // Write to stdout
            match self.format {
                OutputFormat::Sql | OutputFormat::Text => {
                    output!("{sql}");
                }
                OutputFormat::Json => {
                    // For JSON format, wrap in a structured output
                    let export_output = SchemaExportOutput {
                        schema: namespace.name.as_ref().to_string(),
                        sql: sql.clone(),
                        path: None,
                    };
                    output!(
                        "{}",
                        serde_json::to_string_pretty(&export_output).into_diagnostic()?
                    );
                }
            }
        } else {
            // Write to file
            std::fs::write(&output_path, &sql)
                .into_diagnostic()
                .map_err(|e| miette::miette!("Failed to write schema file: {}", e))?;

            match self.format {
                OutputFormat::Text | OutputFormat::Sql => {
                    info!("Schema exported to: {}", output_path.display());
                    newline!();
                    info!("Schema: {}", namespace.name.as_ref());
                    info!("Tables: {}", namespace.tables.len());
                    info!("Views: {}", namespace.views.len());
                    info!("Sequences: {}", namespace.sequences.len());
                    info!("Enums: {}", namespace.enums.len());
                }
                OutputFormat::Json => {
                    let export_output = SchemaExportOutput {
                        schema: namespace.name.as_ref().to_string(),
                        sql,
                        path: Some(output_path.to_string_lossy().to_string()),
                    };
                    output!(
                        "{}",
                        serde_json::to_string_pretty(&export_output).into_diagnostic()?
                    );
                }
            }
        }

        Ok(())
    }
}

/// Output structure for JSON format.
#[derive(Debug, serde::Serialize)]
pub struct SchemaExportOutput {
    /// The schema name (e.g., "public").
    pub schema: String,
    /// The generated SQL DDL.
    pub sql: String,
    /// The path where the schema was written (None if stdout).
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_export_output_serializes() {
        let output = SchemaExportOutput {
            schema: "public".to_string(),
            sql: "CREATE TABLE foo (id int);".to_string(),
            path: Some(".tern/schema.sql".to_string()),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"schema\":\"public\""));
        assert!(json.contains("\"sql\":\"CREATE TABLE foo"));
    }

    #[test]
    fn schema_export_output_path_optional() {
        let output = SchemaExportOutput {
            schema: "public".to_string(),
            sql: "".to_string(),
            path: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"path\":null"));
    }
}
