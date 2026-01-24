//! Migration state tracking and storage.
//!
//! This module provides the infrastructure for tracking migration history and
//! schema state. It enables:
//!
//! - **Content-addressable migrations**: Each migration has a unique ID computed
//!   from its content, ensuring deduplication and integrity verification.
//!
//! - **State hashing**: Schema states are hashed to enable quick comparison and
//!   chain verification without loading full schema data.
//!
//! - **Pluggable backends**: The `StateBackend` trait allows different storage
//!   implementations (local files, remote services, etc.).
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                        Migration History                             │
//! ├─────────────────────────────────────────────────────────────────────┤
//! │                                                                      │
//! │   ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐     │
//! │   │ State 0  │───▶│ State 1  │───▶│ State 2  │───▶│ State 3  │     │
//! │   │ (empty)  │    │          │    │          │    │ (current)│     │
//! │   └──────────┘    └──────────┘    └──────────┘    └──────────┘     │
//! │        │               │               │               │            │
//! │        ▼               ▼               ▼               ▼            │
//! │   ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐     │
//! │   │ Hash: 0  │    │Hash: abc │    │Hash: def │    │Hash: 123 │     │
//! │   └──────────┘    └──────────┘    └──────────┘    └──────────┘     │
//! │                         │               │               │           │
//! │                         ▼               ▼               ▼           │
//! │                    ┌────────────────────────────────────────┐      │
//! │                    │           Migration Chain              │      │
//! │                    │                                        │      │
//! │                    │  m1 ─────▶ m2 ─────▶ m3               │      │
//! │                    │  (id: x)   (id: y)   (id: z)          │      │
//! │                    │  ops: [...] ops: [...] ops: [...]     │      │
//! │                    └────────────────────────────────────────┘      │
//! │                                                                     │
//! └─────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ## Creating a baseline from an existing database
//!
//! ```ignore
//! use tern::db::state::{LocalFileBackend, StateBackend, Migration};
//! use tern::db::query::load_namespace;
//!
//! // Initialize the state directory
//! let backend = LocalFileBackend::default_location();
//! backend.initialize().await?;
//!
//! // Load the current database schema
//! let namespace = load_namespace(&catalog, "public").await?;
//!
//! // Create and save a baseline migration
//! let baseline = Migration::baseline(namespace);
//! backend.save_migration(&baseline).await?;
//! ```
//!
//! ## Recording a new migration
//!
//! ```ignore
//! use tern::db::state::{Migration, StateHash};
//! use tern::db::diff::diff_namespaces;
//! use tern::db::migrate::MigrationPlan;
//!
//! // Diff the schemas
//! let diff = diff_namespaces(&current_schema, &target_schema);
//! let plan = MigrationPlan::from_diff(&diff);
//!
//! // Create the migration record
//! let parent_hash = backend.get_current_state_hash().await?;
//! let new_state = current_schema.apply(&plan.operations)?;
//! let new_hash = StateHash::from_namespace(&new_state);
//!
//! let migration = Migration::new(
//!     "Add users table",
//!     plan.operations,
//!     parent_hash,
//!     new_hash,
//!     vec![], // breaking changes
//! );
//!
//! backend.save_migration(&migration).await?;
//! ```
//!
//! # Storage Format
//!
//! The `LocalFileBackend` stores migrations in the following structure:
//!
//! ```text
//! .tern/
//! └── migrations/
//!     ├── index.json           # Ordered list of migration IDs
//!     ├── 0123456789abcdef.json # Individual migration files
//!     ├── fedcba9876543210.json
//!     └── ...
//! ```
//!
//! Migration files are JSON-serialized `Migration` structs.
//!
//! # Content Addressing
//!
//! Migration IDs are computed from:
//! - The parent state hash
//! - The operations (canonical JSON)
//! - The description
//!
//! This ensures:
//! - Identical migrations produce identical IDs
//! - Any modification changes the ID
//! - History integrity can be verified
//!
//! # Hash Stability
//!
//! The hash algorithm (XxHash3 64-bit) and seeds are fixed to ensure that:
//! - The same content always produces the same hash
//! - Hashes remain stable across tern versions
//! - Existing migration histories remain valid after upgrades

mod error;
mod exporter;
mod init;
pub mod local;
mod types;

pub use error::StateError;
pub use exporter::{ExportConfig, SchemaExporter};
pub use init::{InitError, init_empty, init_from_database, verify_state};
pub use local::{DEFAULT_STATE_DIR, LocalFileBackend};
pub use types::{CachedState, Migration, MigrationId, MigrationIndex, StateHash};

// Re-export InMemoryBackend for testing in other modules
#[cfg(test)]
pub(crate) use tests::InMemoryBackend;

use async_trait::async_trait;

use crate::db::model::Namespace;

/// Trait for migration state storage backends.
///
/// This trait defines the interface for storing and retrieving migration
/// history. Implementations may store data locally (files), remotely
/// (database, cloud storage), or in memory (for testing).
///
/// All methods are async to support remote backends that require network I/O.
#[async_trait]
pub trait StateBackend: Send + Sync {
    /// Initializes the state storage.
    ///
    /// This should create any necessary directories, tables, or other
    /// infrastructure required by the backend. It should be safe to call
    /// multiple times (idempotent).
    async fn initialize(&self) -> Result<(), StateError>;

    /// Checks if the backend has been initialized.
    async fn is_initialized(&self) -> Result<bool, StateError>;

    /// Gets the migration index (ordered list of migration IDs).
    async fn get_migration_index(&self) -> Result<MigrationIndex, StateError>;

    /// Gets a specific migration by ID.
    async fn get_migration(&self, id: &MigrationId) -> Result<Migration, StateError>;

    /// Gets all migrations in order.
    ///
    /// This loads all migration data and may be expensive for large histories.
    /// Consider using `get_migration_index()` and loading migrations lazily
    /// when possible.
    async fn get_all_migrations(&self) -> Result<Vec<Migration>, StateError>;

    /// Saves a new migration.
    ///
    /// This appends the migration to the history. Returns an error if a
    /// migration with the same ID already exists.
    async fn save_migration(&self, migration: &Migration) -> Result<(), StateError>;

    /// Gets the current state hash (from the last migration).
    ///
    /// Returns `StateHash::zero()` if no migrations have been recorded.
    async fn get_current_state_hash(&self) -> Result<StateHash, StateError>;

    /// Gets the current schema state.
    ///
    /// Returns the full `Namespace` representing the current schema state.
    /// This is stored separately from migrations and updated when migrations
    /// are recorded.
    ///
    /// # Errors
    ///
    /// Returns `StateError::EmptyHistory` if no migrations have been recorded.
    async fn get_current_state(&self) -> Result<Namespace, StateError>;

    /// Saves the current schema state.
    ///
    /// This stores the full `Namespace` for fast retrieval without needing
    /// to reconstruct it from the migration history.
    async fn save_current_state(&self, state: &Namespace) -> Result<(), StateError>;

    /// Records a migration and updates the current state atomically.
    ///
    /// This is a convenience method that:
    /// 1. Saves the migration to the history
    /// 2. Updates the current state to reflect the migration
    ///
    /// This is preferred over calling `save_migration()` and `save_current_state()`
    /// separately as it ensures atomic updates.
    async fn record_migration(
        &self,
        migration: &Migration,
        new_state: &Namespace,
    ) -> Result<(), StateError>;

    /// Gets the schema state at a specific migration.
    ///
    /// This reconstructs the schema state as it was after the specified
    /// migration was applied. For efficiency, it finds the nearest checkpoint
    /// migration and applies subsequent operations.
    ///
    /// # Arguments
    ///
    /// * `migration_id` - The ID of the migration to reconstruct state for
    ///
    /// # Errors
    ///
    /// Returns `StateError::MigrationNotFound` if the migration doesn't exist.
    async fn get_state_at(&self, migration_id: &MigrationId) -> Result<Namespace, StateError>;

    /// Verifies the integrity of the migration chain.
    ///
    /// Checks that each migration's parent hash matches the previous
    /// migration's resulting hash.
    async fn verify_chain(&self) -> Result<(), StateError>;

    /// Gets all migrations since a given state hash.
    ///
    /// Returns migrations in order, starting from (but not including)
    /// the migration that produced the given state hash.
    ///
    /// If `state_hash` is zero, returns all migrations.
    async fn get_migrations_since(
        &self,
        state_hash: &StateHash,
    ) -> Result<Vec<Migration>, StateError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory backend for testing the trait interface.
    ///
    /// This implementation stores everything in memory and is useful for
    /// testing code that uses `StateBackend` without filesystem access.
    pub struct InMemoryBackend {
        migrations: std::sync::RwLock<Vec<Migration>>,
        current_state: std::sync::RwLock<Option<Namespace>>,
        initialized: std::sync::atomic::AtomicBool,
    }

    impl Default for InMemoryBackend {
        fn default() -> Self {
            Self {
                migrations: std::sync::RwLock::new(Vec::new()),
                current_state: std::sync::RwLock::new(None),
                initialized: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl InMemoryBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Finds the nearest checkpoint migration at or before the given index.
        fn find_nearest_checkpoint(
            migrations: &[Migration],
            target_idx: usize,
        ) -> Option<(Namespace, usize)> {
            for (i, migration) in migrations[..=target_idx].iter().enumerate().rev() {
                if let Some(ref state) = migration.checkpoint_state {
                    return Some((state.clone(), i + 1));
                }
            }
            None
        }
    }

    #[async_trait]
    impl StateBackend for InMemoryBackend {
        async fn initialize(&self) -> Result<(), StateError> {
            self.initialized
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn is_initialized(&self) -> Result<bool, StateError> {
            Ok(self.initialized.load(std::sync::atomic::Ordering::SeqCst))
        }

        async fn get_migration_index(&self) -> Result<MigrationIndex, StateError> {
            let migrations = self.migrations.read().unwrap();
            let mut index = MigrationIndex::new();
            for m in migrations.iter() {
                index.push(m.id);
            }
            Ok(index)
        }

        async fn get_migration(&self, id: &MigrationId) -> Result<Migration, StateError> {
            let migrations = self.migrations.read().unwrap();
            migrations
                .iter()
                .find(|m| m.id == *id)
                .cloned()
                .ok_or(StateError::MigrationNotFound { id: *id })
        }

        async fn get_all_migrations(&self) -> Result<Vec<Migration>, StateError> {
            Ok(self.migrations.read().unwrap().clone())
        }

        async fn save_migration(&self, migration: &Migration) -> Result<(), StateError> {
            let mut migrations = self.migrations.write().unwrap();
            if migrations.iter().any(|m| m.id == migration.id) {
                return Err(StateError::DuplicateMigration { id: migration.id });
            }
            migrations.push(migration.clone());
            Ok(())
        }

        async fn get_current_state_hash(&self) -> Result<StateHash, StateError> {
            let migrations = self.migrations.read().unwrap();
            Ok(migrations
                .last()
                .map(|m| m.resulting_state_hash)
                .unwrap_or(StateHash::zero()))
        }

        async fn get_current_state(&self) -> Result<Namespace, StateError> {
            self.current_state
                .read()
                .unwrap()
                .clone()
                .ok_or(StateError::EmptyHistory)
        }

        async fn save_current_state(&self, state: &Namespace) -> Result<(), StateError> {
            *self.current_state.write().unwrap() = Some(state.clone());
            Ok(())
        }

        async fn record_migration(
            &self,
            migration: &Migration,
            new_state: &Namespace,
        ) -> Result<(), StateError> {
            self.save_migration(migration).await?;
            self.save_current_state(new_state).await?;
            Ok(())
        }

        async fn get_state_at(&self, migration_id: &MigrationId) -> Result<Namespace, StateError> {
            let migrations = self.migrations.read().unwrap();

            let target_idx = migrations
                .iter()
                .position(|m| m.id == *migration_id)
                .ok_or(StateError::MigrationNotFound { id: *migration_id })?;

            let (mut state, start_idx) = Self::find_nearest_checkpoint(&migrations, target_idx)
                .ok_or(StateError::NoCheckpoint {
                    id: migrations[target_idx].id,
                })?;

            for migration in &migrations[start_idx..=target_idx] {
                state = state.apply(&migration.up_operations).map_err(|e| {
                    StateError::ReconstructionFailed {
                        id: migration.id,
                        message: e.to_string(),
                    }
                })?;
            }

            Ok(state)
        }

        async fn verify_chain(&self) -> Result<(), StateError> {
            let migrations = self.migrations.read().unwrap();
            let mut expected_parent = StateHash::zero();

            for m in migrations.iter() {
                if m.parent_state_hash != expected_parent {
                    return Err(StateError::BrokenChain {
                        id: m.id,
                        parent: m.parent_state_hash,
                        expected: expected_parent,
                    });
                }
                expected_parent = m.resulting_state_hash;
            }
            Ok(())
        }

        async fn get_migrations_since(
            &self,
            state_hash: &StateHash,
        ) -> Result<Vec<Migration>, StateError> {
            let migrations = self.migrations.read().unwrap();
            let mut result = Vec::new();
            let mut found = state_hash.is_zero();

            for m in migrations.iter() {
                if found {
                    result.push(m.clone());
                } else if m.parent_state_hash == *state_hash {
                    found = true;
                    result.push(m.clone());
                }
            }
            Ok(result)
        }
    }

    mod in_memory_backend_tests {
        use super::*;
        use crate::db::model::Namespace;

        #[tokio::test]
        async fn basic_operations() {
            let backend = InMemoryBackend::new();
            backend.initialize().await.unwrap();

            assert!(backend.is_initialized().await.unwrap());

            let ns = Namespace::empty("public");
            let migration = Migration::baseline(ns);
            backend.save_migration(&migration).await.unwrap();

            let retrieved = backend.get_migration(&migration.id).await.unwrap();
            assert_eq!(retrieved.id, migration.id);

            let all = backend.get_all_migrations().await.unwrap();
            assert_eq!(all.len(), 1);

            let index = backend.get_migration_index().await.unwrap();
            assert_eq!(index.len(), 1);
        }

        #[tokio::test]
        async fn chain_verification() {
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

            backend.verify_chain().await.unwrap();
        }
    }
}
