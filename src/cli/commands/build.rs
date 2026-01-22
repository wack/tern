//! Build migration command.
//!
//! This command builds a migration executable or OCI image from the compiled
//! migration state.

use std::path::PathBuf;

use jiff::Zoned;
use miette::{Context, IntoDiagnostic, miette};
use serde::Serialize;

use super::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::compile::{
    BreakingChangeData, CompileOptions, ExecutableBuilder, MigrationData, MitigationStrategy,
    PackageFormat, PackagedBuildResult, StatementData, Target, compile_migration,
};
use crate::db::query::PostgresCatalog;
use crate::db::state::StateBackend;
use crate::db::{self};

/// Build output for JSON format.
#[derive(Debug, Clone, Serialize)]
pub struct BuildOutput {
    /// Migration ID (hex).
    pub migration_id: String,
    /// Migration description.
    pub description: String,
    /// Number of SQL statements.
    pub statement_count: usize,
    /// Output format used.
    pub output_format: String,
    /// Output path.
    pub output_path: String,
    /// Target platform.
    pub target: String,
    /// OCI manifest digest (only for OCI format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_digest: Option<String>,
}

impl std::fmt::Display for BuildOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Migration Built")?;
        writeln!(f, "===============")?;
        writeln!(f)?;
        writeln!(f, "  ID:          {}", self.migration_id)?;
        writeln!(f, "  Description: {}", self.description)?;
        writeln!(f, "  Statements:  {}", self.statement_count)?;
        writeln!(f, "  Target:      {}", self.target)?;
        writeln!(f, "  Format:      {}", self.output_format)?;
        writeln!(f)?;
        writeln!(f, "Output written to: {}", self.output_path)?;

        if let Some(ref digest) = self.manifest_digest {
            writeln!(f)?;
            writeln!(f, "OCI Manifest: {}", digest)?;
        }

        Ok(())
    }
}

/// Runs the build command.
///
/// Builds a migration executable or OCI image by comparing the current state
/// to the live database.
///
/// # Arguments
///
/// * `database_url` - PostgreSQL connection string
/// * `schema` - Database schema name
/// * `output` - Output path for the built artifact
/// * `description` - Migration description
/// * `target` - Target platform for compilation
/// * `package_format` - Output format (binary or oci)
/// * `record` - Whether to record the migration to the state backend
/// * `format` - CLI output format
/// * `state_path` - Optional path to the state directory
#[allow(clippy::too_many_arguments)]
pub async fn run_build(
    database_url: &str,
    schema: &str,
    output: PathBuf,
    description: &str,
    target: &str,
    package_format: &str,
    record: bool,
    format: OutputFormat,
    state_path: Option<&std::path::Path>,
) -> miette::Result<()> {
    // Parse target
    let target = Target::from_str_name(target).ok_or_else(|| {
        miette!(
            "Invalid target: '{}'. Valid targets: native, x86_64-linux-gnu, x86_64-linux-musl, x86_64-macos, aarch64-macos, x86_64-windows",
            target
        )
    })?;

    // Parse package format
    let package_format = PackageFormat::from_str_name(package_format).ok_or_else(|| {
        miette!(
            "Invalid format: '{}'. Valid formats: binary, oci",
            package_format
        )
    })?;

    // Load the state backend
    let backend = load_backend(state_path);
    ensure_backend_initialized(&backend).await?;

    // Get current state from backend
    let source_state = backend
        .get_current_state()
        .await
        .into_diagnostic()
        .wrap_err("Failed to load current state from backend")?;

    // Connect to database and load target state
    println!("Connecting to database...");

    let client = db::connect(database_url)
        .await
        .into_diagnostic()
        .wrap_err("Failed to connect to database")?;

    let catalog = PostgresCatalog::new(&client);

    println!("Loading schema '{}'...", schema);

    let target_state = crate::db::query::load_namespace(&catalog, schema)
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to load schema '{}'", schema))?;

    // Compile the migration
    println!("Compiling migration...");

    let options = CompileOptions::new(description).with_target(target);
    let result = compile_migration(&source_state, &target_state, options).into_diagnostic()?;

    // Create migration data for the executable builder
    let now = Zoned::now();
    let migration_data = MigrationData {
        id: result.migration_id(),
        description: description.to_string(),
        source_state_hash: result.source_hash.to_hex(),
        target_state_hash: result.target_hash.to_hex(),
        compiled_at: now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
        statements: result
            .compilation
            .statements
            .iter()
            .enumerate()
            .map(|(i, s)| StatementData {
                description: s.description.clone(),
                sql: s.sql.clone(),
                sequence: (i + 1) as u32,
            })
            .collect(),
        breaking_changes: result
            .compilation
            .breaking_changes
            .iter()
            .map(|bc| BreakingChangeData {
                description: bc.description.clone(),
                mitigation: match bc.mitigation {
                    crate::db::diff::breaking::MitigationStrategy::DualWrite => {
                        MitigationStrategy::DualWrite
                    }
                    crate::db::diff::breaking::MitigationStrategy::Backfill => {
                        MitigationStrategy::Backfill
                    }
                    crate::db::diff::breaking::MitigationStrategy::Ratchet => {
                        MitigationStrategy::Ratchet
                    }
                    crate::db::diff::breaking::MitigationStrategy::Destructive => {
                        MitigationStrategy::Destructive
                    }
                },
                affected_sql: bc.affected_sql.clone(),
            })
            .collect(),
    };

    // Build the executable or OCI image
    println!(
        "Building {} for {}...",
        match package_format {
            PackageFormat::Binary => "executable",
            PackageFormat::Oci => "OCI image",
        },
        target
    );

    let builder = ExecutableBuilder::new();
    let build_result = builder
        .build_from_data_with_format(&migration_data, &output, target, package_format)
        .into_diagnostic()
        .wrap_err("Failed to build migration")?;

    // Record migration if requested
    if record {
        backend
            .record_migration(&result.migration, &target_state)
            .await
            .into_diagnostic()
            .wrap_err("Failed to record migration")?;

        println!("Migration recorded to state backend.");
    }

    // Build output
    let manifest_digest = match &build_result {
        PackagedBuildResult::Oci(r) => Some(r.manifest_digest.clone()),
        PackagedBuildResult::Binary(_) => None,
    };

    let build_output = BuildOutput {
        migration_id: result.migration_id(),
        description: description.to_string(),
        statement_count: result.statement_count(),
        output_format: package_format.to_string(),
        output_path: build_result.output_path().display().to_string(),
        target: target.to_string(),
        manifest_digest,
    };

    // Output results
    match format {
        OutputFormat::Text => println!("{}", build_output),
        OutputFormat::Json => print_json(&build_output),
        OutputFormat::Sql => {
            for stmt in &result.compilation.statements {
                println!("{};", stmt.sql);
                println!();
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_display() {
        let output = BuildOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            statement_count: 3,
            output_format: "binary".to_string(),
            output_path: "./migrations/add_users".to_string(),
            target: "x86_64-linux-musl".to_string(),
            manifest_digest: None,
        };

        let display = format!("{}", output);
        assert!(display.contains("Migration Built"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("binary"));
        assert!(display.contains("x86_64-linux-musl"));
    }

    #[test]
    fn build_output_display_with_oci() {
        let output = BuildOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            statement_count: 3,
            output_format: "oci".to_string(),
            output_path: "./migrations/add_users.tar".to_string(),
            target: "x86_64-linux-musl".to_string(),
            manifest_digest: Some("sha256:abc123...".to_string()),
        };

        let display = format!("{}", output);
        assert!(display.contains("OCI Manifest"));
        assert!(display.contains("sha256:abc123"));
    }

    #[test]
    fn build_output_json_serializes() {
        let output = BuildOutput {
            migration_id: "abc".to_string(),
            description: "Test".to_string(),
            statement_count: 1,
            output_format: "oci".to_string(),
            output_path: "./test.tar".to_string(),
            target: "native".to_string(),
            manifest_digest: Some("sha256:test".to_string()),
        };

        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"migration_id\":\"abc\""));
        assert!(json.contains("\"manifest_digest\":\"sha256:test\""));
    }
}
