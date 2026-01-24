//! Print migrations command.
//!
//! This command connects to a database and generates the SQL DDL statements
//! that would recreate all objects in the specified schema from scratch.

use clap::Args;
use miette::{Context, IntoDiagnostic};

use crate::db::diff::breaking::analyze_breaking_changes;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::query::{PostgresCatalog, diff_from_empty};
use crate::db::{self};
use crate::{newline, output, warn};

/// Arguments for the print-migrations command.
#[derive(Debug, Clone, Args)]
pub struct PrintMigrations {
    /// PostgreSQL connection string (e.g., postgres://user:pass@localhost/db)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    /// The schema to generate migrations for
    #[arg(long, default_value = "public")]
    pub schema: String,
}

impl PrintMigrations {
    /// Dispatch the print-migrations command.
    ///
    /// Connects to the database and prints migration SQL for the specified schema.
    pub async fn dispatch(&self) -> miette::Result<()> {
        warn!("'print-migrations' is deprecated.");
        // Connect to the database
        let client = db::connect(&self.database_url)
            .await
            .into_diagnostic()
            .wrap_err("Failed to connect to database")?;

        // Create catalog adapter
        let catalog = PostgresCatalog::new(&client);

        // Get diff from empty schema to current state
        let diff = diff_from_empty(&catalog, &self.schema)
            .await
            .into_diagnostic()
            .wrap_err_with(|| format!("Failed to load schema '{}'", self.schema))?;

        // Analyze for breaking changes
        let analysis = analyze_breaking_changes(&diff);
        if !analysis.is_safe() {
            warn!("{} breaking change(s) detected:", analysis.len());
            for change in analysis.iter() {
                warn!("  [{}] {}", change.mitigation.as_str(), change.description);
            }
            newline!();
        }

        // Create migration plan
        let plan = MigrationPlan::from_diff(&diff);

        // Render to SQL
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let script = plan.render(&renderer);

        // Print the SQL
        output!("{}", script.to_sql());

        Ok(())
    }
}
