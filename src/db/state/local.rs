//! Local filesystem backend for migration state.
//!
//! This module provides a state backend that stores migration history
//! on the local filesystem in the `.tern/` directory.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::StateBackend;
use super::error::StateError;
use super::types::{Migration, MigrationId, MigrationIndex, StateHash};
use crate::db::model::Namespace;

/// Default directory name for tern state.
pub const DEFAULT_STATE_DIR: &str = ".tern";

/// Migrations subdirectory within the state directory.
const MIGRATIONS_DIR: &str = "migrations";

/// Index file name.
const INDEX_FILE: &str = "index.json";

/// Current state file name.
const STATE_FILE: &str = "state.json";

/// Schema DDL file name.
///
/// This file contains the complete schema as SQL DDL statements.
/// It is automatically regenerated when migrations are recorded.
const SCHEMA_FILE: &str = "schema.sql";

/// A state backend that stores migrations on the local filesystem.
///
/// # Directory Structure
///
/// ```text
/// .tern/
/// ├── state.json        # Current schema state (Namespace)
/// └── migrations/
///     ├── index.json    # Ordered list of migration IDs + metadata
///     ├── 00001.json    # First migration
///     ├── 00002.json    # Second migration
///     └── ...
/// ```
///
/// Migration files are named with sequential 5-digit zero-padded integers
/// (e.g., `00001.json`, `00002.json`). The sequential naming makes migrations
/// easy to browse and naturally sort in filesystem order. The actual migration
/// ID (content hash) is stored within each file, not in the filename.
///
/// The index file contains a `MigrationIndex` with the ordered list of
/// migration IDs, mapping them to their file numbers.
///
/// The state file contains the current `Namespace` representing the schema
/// state after all migrations have been applied.
#[derive(Debug, Clone)]
pub struct LocalFileBackend {
    /// Root directory for tern state (typically `.tern/`).
    root: PathBuf,
}

impl LocalFileBackend {
    /// Creates a new local file backend at the specified root directory.
    ///
    /// This does not create the directory; use `initialize()` to create
    /// the directory structure.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Creates a new local file backend at `.tern/` in the current directory.
    #[must_use]
    pub fn default_location() -> Self {
        Self::new(DEFAULT_STATE_DIR)
    }

    /// Creates a new local file backend at `.tern/` relative to the given path.
    #[must_use]
    pub fn at_path(base: impl AsRef<Path>) -> Self {
        Self::new(base.as_ref().join(DEFAULT_STATE_DIR))
    }

    /// Returns the root directory path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the migrations directory path.
    fn migrations_dir(&self) -> PathBuf {
        self.root.join(MIGRATIONS_DIR)
    }

    /// Returns the path to the migration index file.
    fn index_path(&self) -> PathBuf {
        self.migrations_dir().join(INDEX_FILE)
    }

    /// Returns the path to the current state file.
    fn state_path(&self) -> PathBuf {
        self.root.join(STATE_FILE)
    }

    /// Returns the path to the schema DDL file.
    ///
    /// The schema file (`.tern/schema.sql`) contains the complete database
    /// schema as SQL DDL statements. It is regenerated automatically when
    /// migrations are recorded.
    #[must_use]
    pub fn schema_path(&self) -> PathBuf {
        self.root.join(SCHEMA_FILE)
    }

    /// Returns the path to a migration file by sequence number.
    ///
    /// Migration files are named with 5-digit zero-padded integers:
    /// `00001.json`, `00002.json`, etc.
    fn migration_path_by_number(&self, number: usize) -> PathBuf {
        self.migrations_dir().join(format!("{number:05}.json"))
    }

    /// Checks if the state directory is initialized.
    fn is_initialized_sync(&self) -> bool {
        self.migrations_dir().exists()
    }

    /// Creates the state directory structure.
    fn create_directories(&self) -> Result<(), StateError> {
        let migrations_dir = self.migrations_dir();
        if !migrations_dir.exists() {
            std::fs::create_dir_all(&migrations_dir).map_err(|source| {
                StateError::CreateDirectory {
                    path: migrations_dir,
                    source,
                }
            })?;
        }
        Ok(())
    }

    /// Reads the migration index from disk.
    fn read_index(&self) -> Result<MigrationIndex, StateError> {
        let index_path = self.index_path();

        if !index_path.exists() {
            return Ok(MigrationIndex::new());
        }

        let content = std::fs::read_to_string(&index_path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                StateError::NotInitialized {
                    path: self.root.clone(),
                }
            } else {
                StateError::ReadIndex { source }
            }
        })?;

        serde_json::from_str(&content).map_err(|source| StateError::InvalidIndexJson { source })
    }

    /// Writes the migration index to disk.
    fn write_index(&self, index: &MigrationIndex) -> Result<(), StateError> {
        let index_path = self.index_path();
        let content = serde_json::to_string_pretty(index)
            .map_err(|source| StateError::SerializeIndex { source })?;

        std::fs::write(&index_path, content).map_err(|source| StateError::WriteIndex { source })
    }

    /// Reads a migration from disk by its sequence number (1-indexed).
    fn read_migration_by_number(&self, number: usize) -> Result<Migration, StateError> {
        let path = self.migration_path_by_number(number);

        let content = std::fs::read_to_string(&path).map_err(|source| {
            StateError::ReadIndex { source } // Use a generic read error
        })?;

        serde_json::from_str(&content).map_err(|source| StateError::InvalidIndexJson { source })
    }

    /// Reads a migration by ID, looking it up in the index first.
    fn read_migration(&self, id: &MigrationId) -> Result<Migration, StateError> {
        let index = self.read_index()?;
        let position = index
            .position(id)
            .ok_or(StateError::MigrationNotFound { id: *id })?;

        // Position is 0-indexed, file numbers are 1-indexed
        self.read_migration_by_number(position + 1)
    }

    /// Writes a migration to disk with the next sequential number.
    fn write_migration(&self, migration: &Migration, number: usize) -> Result<(), StateError> {
        let path = self.migration_path_by_number(number);
        let content = serde_json::to_string_pretty(migration).map_err(|source| {
            StateError::SerializeMigration {
                id: migration.id,
                source,
            }
        })?;

        std::fs::write(&path, content).map_err(|source| StateError::WriteMigration {
            id: migration.id,
            source,
        })
    }

    /// Reads the current state from disk.
    fn read_state(&self) -> Result<Namespace, StateError> {
        let state_path = self.state_path();

        if !state_path.exists() {
            return Err(StateError::EmptyHistory);
        }

        let content = std::fs::read_to_string(&state_path)
            .map_err(|source| StateError::ReadState { source })?;

        serde_json::from_str(&content).map_err(|source| StateError::InvalidStateJson { source })
    }

    /// Writes the current state to disk.
    fn write_state(&self, state: &Namespace) -> Result<(), StateError> {
        let state_path = self.state_path();
        let content = serde_json::to_string_pretty(state)
            .map_err(|source| StateError::SerializeState { source })?;

        std::fs::write(&state_path, content).map_err(|source| StateError::WriteState { source })
    }

    /// Exports the schema as SQL DDL to the schema file.
    ///
    /// This method generates a `.tern/schema.sql` file containing SQL DDL
    /// statements that would recreate the schema from scratch. The file is
    /// useful for:
    ///
    /// - Viewing the current schema in a human-readable format
    /// - Model-first migration workflows (editing schema.sql to define changes)
    /// - Documentation and code review
    ///
    /// The generated SQL is ordered to respect dependencies (enums before
    /// tables that use them, tables before foreign keys that reference them,
    /// etc.).
    ///
    /// # Arguments
    ///
    /// * `state` - The namespace to export
    ///
    /// # Errors
    ///
    /// Returns an error if the backend is not initialized or if writing
    /// the file fails.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let backend = LocalFileBackend::default_location();
    /// let namespace = backend.get_current_state().await?;
    /// backend.export_schema(&namespace)?;
    /// // Schema is now written to .tern/schema.sql
    /// ```
    pub fn export_schema(&self, state: &Namespace) -> Result<(), StateError> {
        use super::SchemaExporter;

        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }

        let schema_path = self.schema_path();
        let sql = SchemaExporter::export(state);

        std::fs::write(&schema_path, sql).map_err(|source| StateError::WriteSchema { source })
    }

    /// Finds the nearest checkpoint migration at or before the given index.
    ///
    /// Returns the checkpoint state and the index to start applying operations from.
    fn find_nearest_checkpoint(
        &self,
        migrations: &[Migration],
        target_idx: usize,
    ) -> Result<(Namespace, usize), StateError> {
        // Search backwards from target for a checkpoint
        for (i, migration) in migrations[..=target_idx].iter().enumerate().rev() {
            if let Some(ref state) = migration.checkpoint_state {
                // Found a checkpoint - start applying from the next migration
                return Ok((state.clone(), i + 1));
            }
        }

        // No checkpoint found - this shouldn't happen if baseline was created properly
        Err(StateError::NoCheckpoint {
            id: migrations[target_idx].id,
        })
    }
}

#[async_trait]
impl StateBackend for LocalFileBackend {
    async fn initialize(&self) -> Result<(), StateError> {
        self.create_directories()?;

        // Create empty index if it doesn't exist
        let index_path = self.index_path();
        if !index_path.exists() {
            self.write_index(&MigrationIndex::new())?;
        }

        Ok(())
    }

    async fn is_initialized(&self) -> Result<bool, StateError> {
        Ok(self.is_initialized_sync())
    }

    async fn get_migration_index(&self) -> Result<MigrationIndex, StateError> {
        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }
        self.read_index()
    }

    async fn get_migration(&self, id: &MigrationId) -> Result<Migration, StateError> {
        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }
        self.read_migration(id)
    }

    async fn get_all_migrations(&self) -> Result<Vec<Migration>, StateError> {
        let index = self.get_migration_index().await?;
        let mut migrations = Vec::with_capacity(index.len());

        // Read migrations by sequence number (1-indexed)
        for i in 1..=index.len() {
            migrations.push(self.read_migration_by_number(i)?);
        }

        Ok(migrations)
    }

    async fn save_migration(&self, migration: &Migration) -> Result<(), StateError> {
        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }

        // Check if migration already exists
        let mut index = self.read_index()?;
        if index.contains(&migration.id) {
            return Err(StateError::DuplicateMigration { id: migration.id });
        }

        // Write migration file with next sequential number (1-indexed)
        let next_number = index.len() + 1;
        self.write_migration(migration, next_number)?;

        // Update index
        index.push(migration.id);
        self.write_index(&index)?;

        Ok(())
    }

    async fn get_current_state_hash(&self) -> Result<StateHash, StateError> {
        let index = self.get_migration_index().await?;

        if index.is_empty() {
            return Ok(StateHash::zero());
        }

        // Read the last migration
        let last_migration = self.read_migration_by_number(index.len())?;
        Ok(last_migration.resulting_state_hash)
    }

    async fn get_current_state(&self) -> Result<Namespace, StateError> {
        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }
        self.read_state()
    }

    async fn save_current_state(&self, state: &Namespace) -> Result<(), StateError> {
        if !self.is_initialized_sync() {
            return Err(StateError::NotInitialized {
                path: self.root.clone(),
            });
        }
        self.write_state(state)
    }

    async fn record_migration(
        &self,
        migration: &Migration,
        new_state: &Namespace,
    ) -> Result<(), StateError> {
        // Save the migration first
        self.save_migration(migration).await?;

        // Then update the current state
        self.save_current_state(new_state).await?;

        // Auto-regenerate schema.sql to keep it in sync with the current state.
        // This enables the model-first workflow where users can view/edit the
        // schema as SQL DDL.
        self.export_schema(new_state)?;

        Ok(())
    }

    async fn get_state_at(&self, migration_id: &MigrationId) -> Result<Namespace, StateError> {
        let migrations = self.get_all_migrations().await?;

        // Find target migration index
        let target_idx = migrations
            .iter()
            .position(|m| m.id == *migration_id)
            .ok_or(StateError::MigrationNotFound { id: *migration_id })?;

        // Find nearest checkpoint and apply operations
        let (mut state, start_idx) = self.find_nearest_checkpoint(&migrations, target_idx)?;

        // Apply operations from checkpoint to target (exclusive of checkpoint migration itself
        // since its state is already included)
        for migration in &migrations[start_idx..=target_idx] {
            state = state.apply(&migration.operations).map_err(|e| {
                StateError::ReconstructionFailed {
                    id: migration.id,
                    message: e.to_string(),
                }
            })?;
        }

        Ok(state)
    }

    async fn verify_chain(&self) -> Result<(), StateError> {
        let index = self.get_migration_index().await?;

        if index.is_empty() {
            return Ok(());
        }

        let mut expected_parent = StateHash::zero();

        for i in 1..=index.len() {
            let migration = self.read_migration_by_number(i)?;

            if migration.parent_state_hash != expected_parent {
                return Err(StateError::BrokenChain {
                    id: migration.id,
                    parent: migration.parent_state_hash,
                    expected: expected_parent,
                });
            }

            expected_parent = migration.resulting_state_hash;
        }

        Ok(())
    }

    async fn get_migrations_since(
        &self,
        state_hash: &StateHash,
    ) -> Result<Vec<Migration>, StateError> {
        let index = self.get_migration_index().await?;
        let mut migrations = Vec::new();
        let mut found_state = state_hash.is_zero();

        for i in 1..=index.len() {
            let migration = self.read_migration_by_number(i)?;

            if found_state {
                migrations.push(migration);
            } else if migration.parent_state_hash == *state_hash {
                found_state = true;
                migrations.push(migration);
            }
        }

        Ok(migrations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::Namespace;
    use tempfile::TempDir;

    fn create_temp_backend() -> (TempDir, LocalFileBackend) {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));
        (temp_dir, backend)
    }

    /// Helper to create a simple state hash for testing.
    fn test_state_hash(value: u8) -> StateHash {
        StateHash::from_bytes([value; 32])
    }

    mod initialization_tests {
        use super::*;

        #[tokio::test]
        async fn initialize_creates_directory_structure() {
            let (temp_dir, backend) = create_temp_backend();

            // Not initialized initially
            assert!(!backend.is_initialized().await.unwrap());

            // Initialize
            backend.initialize().await.unwrap();

            // Should be initialized now
            assert!(backend.is_initialized().await.unwrap());

            // Check directory structure
            assert!(temp_dir.path().join(".tern/migrations").exists());
            assert!(temp_dir.path().join(".tern/migrations/index.json").exists());
        }

        #[tokio::test]
        async fn initialize_is_idempotent() {
            let (_temp_dir, backend) = create_temp_backend();

            backend.initialize().await.unwrap();
            backend.initialize().await.unwrap(); // Should not fail

            assert!(backend.is_initialized().await.unwrap());
        }

        #[tokio::test]
        async fn operations_fail_before_init() {
            let (_temp_dir, backend) = create_temp_backend();

            let result = backend.get_migration_index().await;
            assert!(matches!(result, Err(StateError::NotInitialized { .. })));
        }
    }

    mod index_tests {
        use super::*;

        #[tokio::test]
        async fn empty_index_initially() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let index = backend.get_migration_index().await.unwrap();
            assert!(index.is_empty());
        }

        #[tokio::test]
        async fn current_state_hash_zero_when_empty() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let hash = backend.get_current_state_hash().await.unwrap();
            assert!(hash.is_zero());
        }
    }

    mod migration_tests {
        use super::*;

        #[tokio::test]
        async fn save_and_get_migration() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let migration = Migration::baseline(ns);
            let id = migration.id;

            backend.save_migration(&migration).await.unwrap();

            // Get by ID
            let retrieved = backend.get_migration(&id).await.unwrap();
            assert_eq!(retrieved.id, id);
            assert!(retrieved.is_baseline());

            // Verify index updated
            let index = backend.get_migration_index().await.unwrap();
            assert_eq!(index.len(), 1);
            assert!(index.contains(&id));
        }

        #[tokio::test]
        async fn save_duplicate_fails() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let migration = Migration::baseline(ns);

            backend.save_migration(&migration).await.unwrap();

            // Try to save the same migration again
            let result = backend.save_migration(&migration).await;
            assert!(matches!(result, Err(StateError::DuplicateMigration { .. })));
        }

        #[tokio::test]
        async fn get_missing_migration_fails() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let id = MigrationId::from_bytes([42u8; 32]);
            let result = backend.get_migration(&id).await;
            assert!(matches!(result, Err(StateError::MigrationNotFound { .. })));
        }

        #[tokio::test]
        async fn get_all_migrations_returns_in_order() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            // Create a chain of migrations
            let ns1 = Namespace::empty("public");
            let m1 = Migration::baseline(ns1.clone());
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second migration",
                vec![],
                m1.resulting_state_hash,
                test_state_hash(2),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            let m3 = Migration::new(
                "Third migration",
                vec![],
                m2.resulting_state_hash,
                test_state_hash(3),
                vec![],
            );
            backend.save_migration(&m3).await.unwrap();

            let all = backend.get_all_migrations().await.unwrap();
            assert_eq!(all.len(), 3);
            assert_eq!(all[0].id, m1.id);
            assert_eq!(all[1].id, m2.id);
            assert_eq!(all[2].id, m3.id);
        }
    }

    mod state_hash_tests {
        use super::*;

        #[tokio::test]
        async fn current_state_hash_tracks_latest() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            // Initially zero
            assert!(backend.get_current_state_hash().await.unwrap().is_zero());

            // After first migration
            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            let expected_hash = m1.resulting_state_hash;
            backend.save_migration(&m1).await.unwrap();

            assert_eq!(
                backend.get_current_state_hash().await.unwrap(),
                expected_hash
            );

            // After second migration
            let result_hash = test_state_hash(99);
            let m2 = Migration::new("Second", vec![], expected_hash, result_hash, vec![]);
            backend.save_migration(&m2).await.unwrap();

            assert_eq!(backend.get_current_state_hash().await.unwrap(), result_hash);
        }
    }

    mod chain_verification_tests {
        use super::*;

        #[tokio::test]
        async fn verify_chain_succeeds_for_valid_chain() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                m1.resulting_state_hash,
                test_state_hash(22),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            backend.verify_chain().await.unwrap();
        }

        #[tokio::test]
        async fn verify_chain_empty_succeeds() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            backend.verify_chain().await.unwrap();
        }
    }

    mod migrations_since_tests {
        use super::*;

        #[tokio::test]
        async fn get_migrations_since_zero() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                m1.resulting_state_hash,
                test_state_hash(22),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            // Get all migrations since zero (from the beginning)
            let since = backend
                .get_migrations_since(&StateHash::zero())
                .await
                .unwrap();
            assert_eq!(since.len(), 2);
        }

        #[tokio::test]
        async fn get_migrations_since_specific_hash() {
            let (_temp_dir, backend) = create_temp_backend();
            backend.initialize().await.unwrap();

            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns);
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                m1.resulting_state_hash,
                test_state_hash(22),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            let m3 = Migration::new(
                "Third",
                vec![],
                m2.resulting_state_hash,
                test_state_hash(33),
                vec![],
            );
            backend.save_migration(&m3).await.unwrap();

            // Get migrations since first migration's result
            let since = backend
                .get_migrations_since(&m1.resulting_state_hash)
                .await
                .unwrap();
            assert_eq!(since.len(), 2);
            assert_eq!(since[0].id, m2.id);
            assert_eq!(since[1].id, m3.id);
        }
    }

    mod file_path_tests {
        use super::*;

        #[test]
        fn default_location_is_tern_dir() {
            let backend = LocalFileBackend::default_location();
            assert_eq!(backend.root(), Path::new(DEFAULT_STATE_DIR));
        }

        #[test]
        fn at_path_appends_tern_dir() {
            let backend = LocalFileBackend::at_path("/some/project");
            assert_eq!(backend.root(), Path::new("/some/project/.tern"));
        }

        #[test]
        fn migration_path_uses_sequential_numbers() {
            let backend = LocalFileBackend::new("/test/.tern");

            // Test various sequence numbers
            assert_eq!(
                backend.migration_path_by_number(1),
                PathBuf::from("/test/.tern/migrations/00001.json")
            );
            assert_eq!(
                backend.migration_path_by_number(42),
                PathBuf::from("/test/.tern/migrations/00042.json")
            );
            assert_eq!(
                backend.migration_path_by_number(99999),
                PathBuf::from("/test/.tern/migrations/99999.json")
            );
        }
    }

    mod persistence_tests {
        use super::*;

        #[tokio::test]
        async fn data_persists_across_backend_instances() {
            let temp_dir = TempDir::new().unwrap();
            let root_path = temp_dir.path().join(".tern");

            // First instance - initialize and save
            {
                let backend = LocalFileBackend::new(&root_path);
                backend.initialize().await.unwrap();

                let ns = Namespace::empty("public");
                let migration = Migration::baseline(ns);
                backend.save_migration(&migration).await.unwrap();
            }

            // Second instance - should see the saved migration
            {
                let backend = LocalFileBackend::new(&root_path);
                assert!(backend.is_initialized().await.unwrap());

                let index = backend.get_migration_index().await.unwrap();
                assert_eq!(index.len(), 1);

                let migrations = backend.get_all_migrations().await.unwrap();
                assert_eq!(migrations.len(), 1);
                assert!(migrations[0].is_baseline());
            }
        }

        #[tokio::test]
        async fn files_are_numbered_sequentially() {
            let temp_dir = TempDir::new().unwrap();
            let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));
            backend.initialize().await.unwrap();

            // Save three migrations
            let ns = Namespace::empty("public");
            let m1 = Migration::baseline(ns.clone());
            backend.save_migration(&m1).await.unwrap();

            let m2 = Migration::new(
                "Second",
                vec![],
                m1.resulting_state_hash,
                test_state_hash(2),
                vec![],
            );
            backend.save_migration(&m2).await.unwrap();

            let m3 = Migration::new(
                "Third",
                vec![],
                m2.resulting_state_hash,
                test_state_hash(3),
                vec![],
            );
            backend.save_migration(&m3).await.unwrap();

            // Check that files exist with correct names
            let migrations_dir = temp_dir.path().join(".tern/migrations");
            assert!(migrations_dir.join("00001.json").exists());
            assert!(migrations_dir.join("00002.json").exists());
            assert!(migrations_dir.join("00003.json").exists());
            assert!(!migrations_dir.join("00004.json").exists());
        }
    }
}
