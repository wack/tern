//! Migration executor.
//!
//! This module provides the `MigrationExecutor` which coordinates the
//! execution of pending migrations against a live database.

use tokio_postgres::Client;

use crate::db::checksum::compute_schema_checksum;
use crate::db::migrate::{MigrationPlan, PostgresRenderer, RenderConfig};
use crate::db::query::{PostgresCatalog, load_namespace};
use crate::db::state::{LocalFileBackend, StateBackend};
use crate::db::state::{Migration, MigrationId};

use super::error::{ExecutionError, ExecutionResult, MigrationResult};
use super::tracker::MigrationTracker;

/// Result of integrity verification checks.
#[derive(Debug, Clone, Default)]
pub struct VerificationStatus {
    /// Whether schema checksum verification passed (or was not applicable).
    pub schema_ok: bool,
    /// Details about schema verification failure, if any.
    pub schema_mismatch: Option<SchemaMismatch>,
    /// Whether migration history verification passed.
    pub history_ok: bool,
    /// Details about history verification failure, if any.
    pub history_diverged: Option<HistoryDivergence>,
}

/// Details about a schema checksum mismatch.
#[derive(Debug, Clone)]
pub struct SchemaMismatch {
    /// The migration ID that was verified against.
    pub migration_id: String,
    /// The expected checksum from the database.
    pub expected: String,
    /// The actual checksum computed from the live schema.
    pub actual: String,
}

/// Details about migration history divergence.
#[derive(Debug, Clone)]
pub struct HistoryDivergence {
    /// The first migration that diverged.
    pub migration_id: String,
    /// The expected hash from the database.
    pub expected_hash: String,
    /// The actual hash computed from the local file.
    pub actual_hash: String,
}

impl VerificationStatus {
    /// Returns true if all checks passed.
    pub fn all_ok(&self) -> bool {
        self.schema_ok && self.history_ok
    }
}

/// Executes migrations against a live PostgreSQL database.
///
/// The executor:
/// 1. Ensures tracking tables exist
/// 2. Verifies migration history integrity
/// 3. Identifies pending migrations
/// 4. Executes each migration in a transaction
/// 5. Records the migration in the tracking tables
pub struct MigrationExecutor<'a> {
    client: &'a Client,
    backend: &'a LocalFileBackend,
    tracker: MigrationTracker<'a>,
}

impl<'a> MigrationExecutor<'a> {
    /// Creates a new migration executor.
    ///
    /// # Arguments
    ///
    /// * `client` - PostgreSQL client connection
    /// * `backend` - Local file backend containing migrations
    /// * `target_schema` - The schema where migrations are applied (e.g., "public")
    pub fn new(client: &'a Client, backend: &'a LocalFileBackend, target_schema: &str) -> Self {
        Self {
            client,
            backend,
            tracker: MigrationTracker::new(client, target_schema),
        }
    }

    /// Returns the target schema name.
    pub fn target_schema(&self) -> &str {
        self.tracker.target_schema()
    }

    /// Checks integrity of the migration state without executing anything.
    ///
    /// This method verifies:
    /// 1. Schema checksum - the live database matches the expected state
    /// 2. History integrity - migration files match what was applied
    ///
    /// Returns a `VerificationStatus` indicating what passed or failed.
    pub async fn check_integrity(&self) -> Result<VerificationStatus, ExecutionError> {
        // Ensure tracking infrastructure exists
        self.tracker.ensure_schema().await?;

        let mut status = VerificationStatus {
            schema_ok: true,
            schema_mismatch: None,
            history_ok: true,
            history_diverged: None,
        };

        // Get current state
        let current_id = self.tracker.get_current_migration_id().await?;

        // Check schema integrity
        if let Some(ref id) = current_id
            && let Some(expected_hash) = self.tracker.get_schema_hash(id).await?
            && let Err(ExecutionError::SchemaDrift {
                expected, actual, ..
            }) = self
                .tracker
                .verify_schema_checksum(id, &expected_hash)
                .await
        {
            status.schema_ok = false;
            status.schema_mismatch = Some(SchemaMismatch {
                migration_id: id.clone(),
                expected,
                actual,
            });
        }

        // Load all local migrations for history check
        let all_migrations = self
            .backend
            .get_all_migrations()
            .await
            .map_err(|e| ExecutionError::InvalidState(e.to_string()))?;

        // Check history integrity
        let local_hashes: Vec<(MigrationId, String)> = all_migrations
            .iter()
            .map(|m| (m.id, compute_migration_hash(m)))
            .collect();

        let diverged = self.tracker.verify_history(&local_hashes).await?;

        if let Some((id, expected, actual)) = diverged.first() {
            status.history_ok = false;
            status.history_diverged = Some(HistoryDivergence {
                migration_id: id.clone(),
                expected_hash: expected.clone(),
                actual_hash: actual.clone(),
            });
        }

        Ok(status)
    }

    /// Executes all pending migrations.
    ///
    /// # Algorithm
    ///
    /// 1. Ensure tern schema and tracking tables exist
    /// 2. Get the current migration ID from the database
    /// 3. Verify schema integrity (unless force=true)
    /// 4. Load local migrations and verify history integrity
    /// 5. Identify pending migrations (those after current)
    /// 6. For each pending migration:
    ///    a. Begin transaction
    ///    b. Render and execute SQL
    ///    c. Compute schema checksum
    ///    d. Record migration in tracking tables
    ///    e. Commit transaction
    /// 7. Return execution result
    ///
    /// # Arguments
    ///
    /// * `force` - If true, skip integrity verification checks
    pub async fn execute_pending(&self, force: bool) -> Result<ExecutionResult, ExecutionError> {
        // Ensure tracking infrastructure exists
        self.tracker.ensure_schema().await?;

        // Get current state
        let current_id = self.tracker.get_current_migration_id().await?;

        // Verify schema integrity before proceeding (unless force=true)
        if !force
            && let Some(ref id) = current_id
            && let Some(expected_hash) = self.tracker.get_schema_hash(id).await?
        {
            self.tracker
                .verify_schema_checksum(id, &expected_hash)
                .await?;
        }

        // Load all local migrations
        let all_migrations = self
            .backend
            .get_all_migrations()
            .await
            .map_err(|e| ExecutionError::InvalidState(e.to_string()))?;

        // Verify history integrity (unless force=true)
        if !force {
            self.verify_history(&all_migrations).await?;
        }

        // Find pending migrations
        let pending = self.find_pending_migrations(&all_migrations, current_id.as_deref())?;

        if pending.is_empty() {
            return Ok(ExecutionResult::success(vec![]));
        }

        // Execute each pending migration
        let mut applied = Vec::new();

        for migration in pending {
            match self.execute_migration(&migration).await {
                Ok(result) => {
                    applied.push(result);
                }
                Err(e) => {
                    return Ok(ExecutionResult::failure(applied, e.to_string()));
                }
            }
        }

        Ok(ExecutionResult::success(applied))
    }

    /// Gets the list of pending migrations without executing them.
    ///
    /// Useful for dry-run mode.
    ///
    /// # Arguments
    ///
    /// * `force` - If true, skip integrity verification checks
    pub async fn get_pending(&self, force: bool) -> Result<Vec<Migration>, ExecutionError> {
        // Ensure tracking infrastructure exists
        self.tracker.ensure_schema().await?;

        // Get current state
        let current_id = self.tracker.get_current_migration_id().await?;

        // Verify schema integrity before proceeding (unless force=true)
        if !force
            && let Some(ref id) = current_id
            && let Some(expected_hash) = self.tracker.get_schema_hash(id).await?
        {
            self.tracker
                .verify_schema_checksum(id, &expected_hash)
                .await?;
        }

        // Load all local migrations
        let all_migrations = self
            .backend
            .get_all_migrations()
            .await
            .map_err(|e| ExecutionError::InvalidState(e.to_string()))?;

        // Verify history integrity (unless force=true)
        if !force {
            self.verify_history(&all_migrations).await?;
        }

        // Find pending migrations
        self.find_pending_migrations(&all_migrations, current_id.as_deref())
    }

    /// Verifies that recorded migrations match local migration hashes.
    async fn verify_history(&self, local_migrations: &[Migration]) -> Result<(), ExecutionError> {
        let local_hashes: Vec<(MigrationId, String)> = local_migrations
            .iter()
            .map(|m| (m.id, compute_migration_hash(m)))
            .collect();

        let diverged = self.tracker.verify_history(&local_hashes).await?;

        if let Some((id, expected, actual)) = diverged.first() {
            return Err(ExecutionError::HistoryDiverged {
                migration_id: id.clone(),
                expected_hash: expected.clone(),
                actual_hash: actual.clone(),
            });
        }

        Ok(())
    }

    /// Finds migrations that haven't been applied yet.
    fn find_pending_migrations(
        &self,
        all_migrations: &[Migration],
        current_id: Option<&str>,
    ) -> Result<Vec<Migration>, ExecutionError> {
        match current_id {
            None => {
                // Fresh database - all migrations are pending
                Ok(all_migrations.to_vec())
            }
            Some(current) => {
                // Find position of current migration
                let current_pos = all_migrations.iter().position(|m| m.id.to_hex() == current);

                match current_pos {
                    Some(pos) => {
                        // Return everything after the current position
                        Ok(all_migrations[pos + 1..].to_vec())
                    }
                    None => {
                        // Current migration not found in local migrations
                        Err(ExecutionError::MigrationNotFound(current.to_string()))
                    }
                }
            }
        }
    }

    /// Executes a single migration within a transaction.
    async fn execute_migration(
        &self,
        migration: &Migration,
    ) -> Result<MigrationResult, ExecutionError> {
        // Render migration to SQL
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let plan = MigrationPlan::from_operations(migration.up_operations.clone());
        let script = plan.render(&renderer);
        let sql = script.to_sql();
        let statement_count = script.all_statements().len();

        // Begin transaction
        self.client
            .execute("BEGIN", &[])
            .await
            .map_err(|e| ExecutionError::Transaction(format!("failed to begin: {}", e)))?;

        // Set search path to target schema
        let set_search_path = format!("SET search_path TO {}", self.target_schema());
        self.client
            .batch_execute(&set_search_path)
            .await
            .map_err(|e| ExecutionError::MigrationFailed {
                migration_id: migration.id.to_short_hex(),
                message: format!("failed to set search_path: {}", e),
            })?;

        // Execute migration SQL
        if !sql.is_empty()
            && let Err(e) = self.client.batch_execute(&sql).await
        {
            // Rollback on failure
            let _ = self.client.execute("ROLLBACK", &[]).await;
            return Err(ExecutionError::MigrationFailed {
                migration_id: migration.id.to_short_hex(),
                message: e.to_string(),
            });
        }

        // Compute schema checksum
        let catalog = PostgresCatalog::new(self.client);
        let namespace = load_namespace(&catalog, self.target_schema())
            .await
            .map_err(|e| ExecutionError::ChecksumFailed(e.to_string()))?;
        let schema_hash = compute_schema_checksum(&namespace);

        // Compute migration hash
        let migration_hash = compute_migration_hash(migration);

        // Record in tracking tables
        self.tracker
            .record_migration(
                &migration.id,
                &migration.description,
                &migration_hash,
                &schema_hash,
            )
            .await?;

        // Commit transaction
        self.client
            .execute("COMMIT", &[])
            .await
            .map_err(|e| ExecutionError::Transaction(format!("failed to commit: {}", e)))?;

        Ok(MigrationResult {
            migration_id: migration.id,
            description: migration.description.clone(),
            statement_count,
            schema_hash,
        })
    }
}

/// Computes the BLAKE3 hash of a migration's up_operations.
///
/// This hash is used to detect if a migration has been modified since it was
/// applied. Only the up_operations array is hashed, not the description or
/// timestamps, allowing descriptions to be updated without triggering
/// divergence errors.
pub fn compute_migration_hash(migration: &Migration) -> String {
    let mut hasher = blake3::Hasher::new();
    let ops_json =
        serde_json::to_vec(&migration.up_operations).expect("operations should be serializable");
    hasher.update(&ops_json);
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::state::StateHash;

    #[test]
    fn compute_migration_hash_is_deterministic() {
        let migration1 = Migration::new(
            "Test migration",
            vec![], // up_operations
            vec![], // down_operations
            StateHash::zero(),
            StateHash::zero(),
            vec![],
        );
        let migration2 = Migration::new(
            "Different description", // Different description
            vec![],                  // Same up_operations
            vec![],                  // down_operations
            StateHash::zero(),
            StateHash::zero(),
            vec![],
        );

        let hash1 = compute_migration_hash(&migration1);
        let hash2 = compute_migration_hash(&migration2);

        // Hashes should be the same since only up_operations matter
        assert_eq!(hash1, hash2);
    }
}
