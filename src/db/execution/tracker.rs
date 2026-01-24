//! Migration tracker for live database execution.
//!
//! This module manages the `tern.migrations` and `tern.current` tables that
//! track which migrations have been applied to a database.

use tokio_postgres::Client;

use crate::db::state::MigrationId;

use super::error::ExecutionError;

/// SQL to create the tern schema and tracking tables.
const CREATE_SCHEMA_SQL: &str = r#"
-- Create tern schema for migration tracking
CREATE SCHEMA IF NOT EXISTS tern;

-- Table storing all applied migrations
CREATE TABLE IF NOT EXISTS tern.migrations (
    id              TEXT PRIMARY KEY,
    sequence        INTEGER NOT NULL UNIQUE,
    description     TEXT NOT NULL,
    migration_hash  TEXT NOT NULL,
    schema_hash     TEXT NOT NULL,
    applied_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Single-row table pointing to current migration
CREATE TABLE IF NOT EXISTS tern.current (
    singleton       BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton = true),
    migration_id    TEXT NOT NULL REFERENCES tern.migrations(id),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
"#;

/// Record of a migration stored in the database.
#[derive(Debug, Clone)]
pub struct MigrationRecord {
    /// The migration ID.
    pub id: String,
    /// The sequence number (1, 2, 3, ...).
    pub sequence: i32,
    /// The migration description.
    pub description: String,
    /// BLAKE3 hash of the migration operations.
    pub migration_hash: String,
    /// xxhash3 checksum of the resulting schema.
    pub schema_hash: String,
}

/// Tracks migration state in a live PostgreSQL database.
///
/// The tracker manages two tables in the `tern` schema:
///
/// - `tern.migrations`: Records all applied migrations with their hashes
/// - `tern.current`: Single-row table pointing to the current migration
pub struct MigrationTracker<'a> {
    client: &'a Client,
    target_schema: String,
}

impl<'a> MigrationTracker<'a> {
    /// Creates a new migration tracker.
    ///
    /// # Arguments
    ///
    /// * `client` - PostgreSQL client connection
    /// * `target_schema` - The schema where migrations are applied (e.g., "public")
    pub fn new(client: &'a Client, target_schema: &str) -> Self {
        Self {
            client,
            target_schema: target_schema.to_string(),
        }
    }

    /// Returns the target schema name.
    pub fn target_schema(&self) -> &str {
        &self.target_schema
    }

    /// Ensures the tern tracking schema and tables exist.
    ///
    /// This is idempotent - it can be called multiple times safely.
    pub async fn ensure_schema(&self) -> Result<(), ExecutionError> {
        self.client
            .batch_execute(CREATE_SCHEMA_SQL)
            .await
            .map_err(|e| ExecutionError::CreateSchema(e.to_string()))?;
        Ok(())
    }

    /// Gets the current migration ID, if any.
    ///
    /// Returns `None` if no migrations have been applied (fresh database).
    pub async fn get_current_migration_id(&self) -> Result<Option<String>, ExecutionError> {
        let row = self
            .client
            .query_opt("SELECT migration_id FROM tern.current LIMIT 1", &[])
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(row.map(|r| r.get(0)))
    }

    /// Gets all recorded migrations in sequence order.
    pub async fn get_all_migrations(&self) -> Result<Vec<MigrationRecord>, ExecutionError> {
        let rows = self
            .client
            .query(
                "SELECT id, sequence, description, migration_hash, schema_hash
                 FROM tern.migrations
                 ORDER BY sequence ASC",
                &[],
            )
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| MigrationRecord {
                id: r.get(0),
                sequence: r.get(1),
                description: r.get(2),
                migration_hash: r.get(3),
                schema_hash: r.get(4),
            })
            .collect())
    }

    /// Gets a specific migration record by ID.
    pub async fn get_migration(&self, id: &str) -> Result<Option<MigrationRecord>, ExecutionError> {
        let row = self
            .client
            .query_opt(
                "SELECT id, sequence, description, migration_hash, schema_hash
                 FROM tern.migrations
                 WHERE id = $1",
                &[&id],
            )
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(row.map(|r| MigrationRecord {
            id: r.get(0),
            sequence: r.get(1),
            description: r.get(2),
            migration_hash: r.get(3),
            schema_hash: r.get(4),
        }))
    }

    /// Gets the next sequence number for a new migration.
    pub async fn next_sequence(&self) -> Result<i32, ExecutionError> {
        let row = self
            .client
            .query_one(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM tern.migrations",
                &[],
            )
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(row.get(0))
    }

    /// Records a migration as applied.
    ///
    /// This should be called within a transaction after successfully
    /// executing the migration SQL.
    ///
    /// # Arguments
    ///
    /// * `id` - The migration ID
    /// * `description` - The migration description
    /// * `migration_hash` - BLAKE3 hash of the migration operations
    /// * `schema_hash` - xxhash3 checksum of the resulting schema
    pub async fn record_migration(
        &self,
        id: &MigrationId,
        description: &str,
        migration_hash: &str,
        schema_hash: &str,
    ) -> Result<i32, ExecutionError> {
        let sequence = self.next_sequence().await?;
        let id_hex = id.to_hex();

        // Insert the migration record
        self.client
            .execute(
                "INSERT INTO tern.migrations (id, sequence, description, migration_hash, schema_hash)
                 VALUES ($1, $2, $3, $4, $5)",
                &[&id_hex, &sequence, &description, &migration_hash, &schema_hash],
            )
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        // Update the current pointer
        self.client
            .execute(
                "INSERT INTO tern.current (singleton, migration_id, updated_at)
                 VALUES (true, $1, now())
                 ON CONFLICT (singleton) DO UPDATE SET
                     migration_id = EXCLUDED.migration_id,
                     updated_at = EXCLUDED.updated_at",
                &[&id_hex],
            )
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(sequence)
    }

    /// Verifies that recorded migrations match local migration hashes.
    ///
    /// Returns the list of migration IDs that have diverged (local hash
    /// doesn't match recorded hash).
    pub async fn verify_history(
        &self,
        local_migrations: &[(MigrationId, String)], // (id, hash)
    ) -> Result<Vec<(String, String, String)>, ExecutionError> {
        let recorded = self.get_all_migrations().await?;
        let mut diverged = Vec::new();

        for record in &recorded {
            // Find the corresponding local migration
            if let Some((_, local_hash)) = local_migrations
                .iter()
                .find(|(id, _): &&(MigrationId, String)| id.to_hex() == record.id)
                && local_hash != &record.migration_hash
            {
                diverged.push((
                    record.id.clone(),
                    record.migration_hash.clone(),
                    local_hash.clone(),
                ));
            }
        }

        Ok(diverged)
    }

    /// Checks if a migration has been applied.
    pub async fn is_applied(&self, id: &MigrationId) -> Result<bool, ExecutionError> {
        let id_hex = id.to_hex();
        let row = self
            .client
            .query_opt("SELECT 1 FROM tern.migrations WHERE id = $1", &[&id_hex])
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(row.is_some())
    }

    /// Gets the count of applied migrations.
    pub async fn migration_count(&self) -> Result<i64, ExecutionError> {
        let row = self
            .client
            .query_one("SELECT COUNT(*) FROM tern.migrations", &[])
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        Ok(row.get(0))
    }

    /// Removes the most recent migration record from the tracking tables.
    ///
    /// This should be called within a transaction after successfully
    /// reverting the migration operations.
    ///
    /// # Returns
    ///
    /// Returns the ID of the removed migration if successful.
    pub async fn unrecord_migration(&self) -> Result<String, ExecutionError> {
        // Get the current (most recent) migration
        let current_id = self
            .get_current_migration_id()
            .await?
            .ok_or(ExecutionError::NoMigrationsToRevert)?;

        // Get the current migration's sequence to find the previous one
        let current_migration = self
            .get_migration(&current_id)
            .await?
            .ok_or(ExecutionError::MigrationNotFound(current_id.clone()))?;

        // Find the previous migration (if any)
        let previous = if current_migration.sequence > 1 {
            let row = self
                .client
                .query_opt(
                    "SELECT id FROM tern.migrations WHERE sequence = $1",
                    &[&(current_migration.sequence - 1)],
                )
                .await
                .map_err(|e| ExecutionError::Query(e.to_string()))?;
            row.map(|r| r.get::<_, String>(0))
        } else {
            None
        };

        // Delete the current migration record
        self.client
            .execute("DELETE FROM tern.migrations WHERE id = $1", &[&current_id])
            .await
            .map_err(|e| ExecutionError::Query(e.to_string()))?;

        // Update the current pointer
        if let Some(prev_id) = previous {
            self.client
                .execute(
                    "UPDATE tern.current SET migration_id = $1, updated_at = now()",
                    &[&prev_id],
                )
                .await
                .map_err(|e| ExecutionError::Query(e.to_string()))?;
        } else {
            // No previous migration - delete the current pointer
            self.client
                .execute("DELETE FROM tern.current", &[])
                .await
                .map_err(|e| ExecutionError::Query(e.to_string()))?;
        }

        Ok(current_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_schema_sql_is_valid() {
        // Basic check that the SQL doesn't have obvious syntax issues
        assert!(CREATE_SCHEMA_SQL.contains("CREATE SCHEMA"));
        assert!(CREATE_SCHEMA_SQL.contains("CREATE TABLE"));
        assert!(CREATE_SCHEMA_SQL.contains("tern.migrations"));
        assert!(CREATE_SCHEMA_SQL.contains("tern.current"));
    }

    #[test]
    fn migration_record_debug() {
        let record = MigrationRecord {
            id: "abc123".to_string(),
            sequence: 1,
            description: "Test migration".to_string(),
            migration_hash: "hash123".to_string(),
            schema_hash: "schema456".to_string(),
        };
        let debug = format!("{:?}", record);
        assert!(debug.contains("abc123"));
        assert!(debug.contains("Test migration"));
    }
}
