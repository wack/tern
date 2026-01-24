//! Generate migration command.
//!
//! This command generates a migration from changes made to `.tern/schema.sql`.

use std::io::{self, Write};
use std::path::PathBuf;

use anstream::{eprintln, println};
use clap::Args;
use miette::IntoDiagnostic;
use serde::Serialize;

use crate::cli::{OutputFormat, ensure_backend_initialized, load_backend, print_json};
use crate::db::diff::breaking::{BreakingChange, MitigationStrategy, analyze_breaking_changes};
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{
    MigrationPlan, PostgresRenderer, RenderConfig, compute_inverse_operations,
};
use crate::db::pglite::SchemaLoader;
use crate::db::state::{Migration, StateBackend, StateHash};

/// Generate migration from schema changes.
///
/// Compares the current migration state to the edited schema.sql file
/// and generates a migration that would transform the schema.
#[derive(Debug, Clone, Args)]
pub struct Generate {
    /// Migration description (required)
    #[arg(short, long)]
    pub description: String,

    /// Path to the schema file
    #[arg(short, long)]
    pub schema: Option<PathBuf>,

    /// Path to the state directory
    #[arg(long)]
    pub path: Option<PathBuf>,

    /// Preview without recording the migration
    #[arg(long)]
    pub dry_run: bool,

    /// Skip confirmation prompts and integrity verification (dangerous)
    #[arg(long)]
    pub force: bool,

    /// Output format
    #[arg(long, default_value = "text")]
    pub format: OutputFormat,
}

/// Output structure for generate JSON format.
#[derive(Debug, Serialize)]
pub struct GenerateOutput {
    /// Migration ID (hex).
    pub migration_id: String,
    /// Migration description.
    pub description: String,
    /// Number of operations in the migration.
    pub operation_count: usize,
    /// Whether there are breaking changes.
    pub has_breaking_changes: bool,
    /// Whether there are destructive changes.
    pub has_destructive_changes: bool,
    /// Source state hash.
    pub source_state_hash: String,
    /// Target state hash.
    pub target_state_hash: String,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Breaking changes with details.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub breaking_changes: Vec<BreakingChangeOutput>,
    /// Whether the migration can be reversed with `down`.
    pub is_reversible: bool,
}

/// Breaking change output for JSON serialization.
#[derive(Debug, Serialize)]
pub struct BreakingChangeOutput {
    /// Description of the breaking change.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: String,
}

impl From<&BreakingChange> for BreakingChangeOutput {
    fn from(bc: &BreakingChange) -> Self {
        Self {
            description: bc.description.clone(),
            mitigation: bc.mitigation.as_str().to_string(),
        }
    }
}

impl std::fmt::Display for GenerateOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.dry_run {
            writeln!(f, "Migration Preview (Dry Run)")?;
            writeln!(f, "===========================")?;
        } else {
            writeln!(f, "Migration Created")?;
            writeln!(f, "=================")?;
        }
        writeln!(f)?;
        writeln!(
            f,
            "  ID:          {}",
            &self.migration_id[..16.min(self.migration_id.len())]
        )?;
        writeln!(f, "  Description: {}", self.description)?;
        writeln!(f, "  Operations:  {}", self.operation_count)?;
        writeln!(
            f,
            "  From state:  {}",
            &self.source_state_hash[..16.min(self.source_state_hash.len())]
        )?;
        writeln!(
            f,
            "  To state:    {}",
            &self.target_state_hash[..16.min(self.target_state_hash.len())]
        )?;
        writeln!(
            f,
            "  Reversible:  {}",
            if self.is_reversible { "yes" } else { "no" }
        )?;

        if self.has_breaking_changes {
            writeln!(f)?;
            writeln!(f, "WARNING: Breaking Changes Detected")?;
            writeln!(f, "-----------------------------------")?;
            for bc in &self.breaking_changes {
                writeln!(f, "  [{}] {}", bc.mitigation, bc.description)?;
            }
        }

        if !self.is_reversible {
            writeln!(f)?;
            writeln!(f, "NOTE: This migration contains irreversible operations.")?;
            writeln!(f, "      Running 'tern down' will not be possible.")?;
        }

        if self.dry_run {
            writeln!(f)?;
            writeln!(f, "This was a dry run. No migration was recorded.")?;
            writeln!(f, "Run without --dry-run to create the migration.")?;
        } else {
            writeln!(f)?;
            writeln!(f, "Run 'tern up <DATABASE_URL>' to apply this migration.")?;
        }

        Ok(())
    }
}

impl Generate {
    /// Dispatch the generate command.
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // Verify migration chain integrity (unless --force)
        if !self.force {
            self.verify_chain_integrity(&backend).await?;
        }

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        let source_hash = StateHash::from_namespace(&source);

        // Determine schema file path
        let schema_file = self.schema.unwrap_or_else(|| backend.schema_path());

        // Verify the schema file exists
        if !schema_file.exists() {
            return Err(miette::miette!(
                "Schema file not found: {}\n\nRun 'tern schema export' to generate the schema file first.",
                schema_file.display()
            ));
        }

        // Load the target state (edited schema file)
        let target = SchemaLoader::load_file(&schema_file)
            .await
            .map_err(|e| miette::miette!("Failed to load schema file: {}", e))?;
        let target_hash = StateHash::from_namespace(&target);

        // Generate diff
        let diff = diff_namespaces(&source, &target);

        // Analyze for breaking changes
        let analysis = analyze_breaking_changes(&diff);

        // Generate migration plan
        let plan = MigrationPlan::from_diff(&diff);

        // Check if there are any changes
        if plan.is_empty() {
            match self.format {
                OutputFormat::Text => {
                    println!("No changes detected.");
                    println!();
                    println!("The schema file matches the current state.");
                }
                OutputFormat::Json => {
                    let output = GenerateOutput {
                        migration_id: String::new(),
                        description: self.description.clone(),
                        operation_count: 0,
                        has_breaking_changes: false,
                        has_destructive_changes: false,
                        source_state_hash: source_hash.to_hex(),
                        target_state_hash: target_hash.to_hex(),
                        dry_run: self.dry_run,
                        breaking_changes: vec![],
                        is_reversible: true, // Empty migration is trivially reversible
                    };
                    print_json(&output);
                }
                OutputFormat::Sql => {
                    println!("-- No changes detected.");
                }
            }
            return Ok(());
        }

        // Handle destructive changes
        let destructive_count = analysis.count_by_mitigation(MitigationStrategy::Destructive);
        if destructive_count > 0 && !self.force && !self.dry_run {
            // Show warning
            eprintln!();
            eprintln!(
                "WARNING: {} destructive change(s) detected:",
                destructive_count
            );
            eprintln!();
            for change in analysis.by_mitigation(MitigationStrategy::Destructive) {
                eprintln!("  - {}", change.description);
            }
            eprintln!();
            eprintln!("These changes will result in DATA LOSS and cannot be undone.");
            eprintln!();

            // Prompt for confirmation
            if !confirm_destructive_changes()? {
                eprintln!("Migration cancelled.");
                return Ok(());
            }
        }

        // Get the breaking changes for the migration
        let breaking_changes = analysis.into_changes();

        // Compute inverse operations for the down migration
        let inverse_result = compute_inverse_operations(&plan.operations);
        let down_operations = inverse_result.operations;

        // Create the migration
        let migration = Migration::new(
            &self.description,
            plan.operations.clone(),
            down_operations,
            source_hash,
            target_hash,
            breaking_changes.clone(),
        );

        // Build output
        let breaking_change_outputs: Vec<BreakingChangeOutput> = breaking_changes
            .iter()
            .map(BreakingChangeOutput::from)
            .collect();

        let output = GenerateOutput {
            migration_id: migration.id.to_hex(),
            description: self.description.clone(),
            operation_count: migration.up_operations.len(),
            has_breaking_changes: !breaking_changes.is_empty(),
            has_destructive_changes: destructive_count > 0,
            source_state_hash: source_hash.to_hex(),
            target_state_hash: target_hash.to_hex(),
            dry_run: self.dry_run,
            breaking_changes: breaking_change_outputs,
            is_reversible: migration.is_reversible(),
        };

        // Record the migration (unless dry run)
        if !self.dry_run {
            backend
                .record_migration(&migration, &target)
                .await
                .into_diagnostic()?;
        }

        // Output based on format
        match self.format {
            OutputFormat::Text => println!("{}", output),
            OutputFormat::Json => print_json(&output),
            OutputFormat::Sql => {
                let renderer = PostgresRenderer::new(RenderConfig::default());
                let script = plan.render(&renderer);
                println!("{}", script.to_sql());
            }
        }

        Ok(())
    }

    /// Verifies the integrity of the local migration chain.
    ///
    /// Checks that each migration's parent_state_hash matches the previous
    /// migration's resulting_state_hash.
    async fn verify_chain_integrity<B: StateBackend>(&self, backend: &B) -> miette::Result<()> {
        use crate::db::state::StateError;

        match backend.verify_chain().await {
            Ok(()) => Ok(()),
            Err(StateError::BrokenChain {
                id,
                parent,
                expected,
            }) => Err(miette::miette!(
                "Migration chain integrity check failed\n\n\
                 Migration {} has an invalid parent hash.\n\n\
                 Expected parent: {}...\n\
                 Actual parent:   {}...\n\n\
                 This indicates the migration file was modified or corrupted.\n\n\
                 To resolve:\n  \
                 Option 1: Restore the migration from version control\n  \
                 Option 2: Run 'tern verify chain' for detailed diagnostics\n  \
                 Option 3: Use --force to skip verification (dangerous)",
                &id.to_hex()[..16.min(id.to_hex().len())],
                &expected.to_hex()[..16.min(expected.to_hex().len())],
                &parent.to_hex()[..16.min(parent.to_hex().len())],
            )),
            Err(e) => Err(miette::miette!("Failed to verify migration chain: {}", e)),
        }
    }
}

/// Prompts the user to confirm destructive changes.
///
/// Returns `true` if the user confirms, `false` otherwise.
fn confirm_destructive_changes() -> miette::Result<bool> {
    eprint!("Continue with destructive changes? [y/N] ");
    io::stderr().flush().into_diagnostic()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input).into_diagnostic()?;

    let input = input.trim().to_lowercase();
    Ok(input == "y" || input == "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_output_display() {
        let output = GenerateOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            operation_count: 3,
            has_breaking_changes: false,
            has_destructive_changes: false,
            is_reversible: true,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            dry_run: false,
            breaking_changes: vec![],
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration Created"));
        assert!(display.contains("Add users table"));
        assert!(display.contains("Operations:  3"));
        assert!(!display.contains("dry run"));
    }

    #[test]
    fn generate_dry_run_display() {
        let output = GenerateOutput {
            migration_id: "abc123def456".repeat(5),
            description: "Add users table".to_string(),
            operation_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            is_reversible: true,
            source_state_hash: "0".repeat(64),
            target_state_hash: "1".repeat(64),
            dry_run: true,
            breaking_changes: vec![],
        };
        let display = format!("{}", output);
        assert!(display.contains("Migration Preview (Dry Run)"));
        assert!(display.contains("This was a dry run"));
    }

    #[test]
    fn generate_output_serializes() {
        let output = GenerateOutput {
            migration_id: "abc".to_string(),
            description: "Test".to_string(),
            operation_count: 1,
            has_breaking_changes: false,
            has_destructive_changes: false,
            is_reversible: true,
            source_state_hash: "src".to_string(),
            target_state_hash: "tgt".to_string(),
            dry_run: false,
            breaking_changes: vec![],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"migration_id\":\"abc\""));
        assert!(json.contains("\"description\":\"Test\""));
    }

    mod chain_verification_tests {
        use crate::db::model::Namespace;
        use crate::db::state::{InMemoryBackend, Migration, StateBackend, StateHash};

        use super::Generate;

        #[tokio::test]
        async fn verify_chain_integrity_passes_for_valid_chain() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                vec![],
                m1.resulting_state_hash,
                StateHash::from_bytes([22u8; 32]),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            let generate = Generate {
                description: "Test".to_string(),
                schema: None,
                path: None,
                dry_run: false,
                force: false,
                format: crate::cli::OutputFormat::Text,
            };

            // Should pass without error
            let result = generate.verify_chain_integrity(&backend).await;
            assert!(result.is_ok());
        }

        #[tokio::test]
        async fn verify_chain_integrity_fails_for_broken_chain() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            // Create a migration with wrong parent hash
            let m2 = Migration::new(
                "Second",
                vec![],
                vec![],
                StateHash::from_bytes([99u8; 32]), // Wrong parent hash
                StateHash::from_bytes([22u8; 32]),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            let generate = Generate {
                description: "Test".to_string(),
                schema: None,
                path: None,
                dry_run: false,
                force: false,
                format: crate::cli::OutputFormat::Text,
            };

            // Should fail with error message
            let result = generate.verify_chain_integrity(&backend).await;
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Migration chain integrity check failed"));
            assert!(err.contains("invalid parent hash"));
        }

        #[tokio::test]
        async fn verify_chain_integrity_passes_for_empty_history() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            let generate = Generate {
                description: "Test".to_string(),
                schema: None,
                path: None,
                dry_run: false,
                force: false,
                format: crate::cli::OutputFormat::Text,
            };

            // Empty history should pass (nothing to verify)
            let result = generate.verify_chain_integrity(&backend).await;
            assert!(result.is_ok());
        }
    }
}
