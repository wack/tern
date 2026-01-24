//! Integration tests for state reconstruction and migration chain verification.
//!
//! These tests exercise the state backend functionality:
//! 1. Migration recording and retrieval
//! 2. State reconstruction via `get_state_at()`
//! 3. Migration chain integrity verification
//! 4. Checkpoint-based state reconstruction
//! 5. LocalFileBackend persistence
//!
//! These tests use both `InMemoryBackend` (for unit-style testing) and
//! `LocalFileBackend` (for persistence testing with tempfile).

use insta::assert_yaml_snapshot;
use tempfile::TempDir;
use tern::db::diff::diff_namespaces;
use tern::db::history::builder::NamespaceBuilder;
use tern::db::migrate::MigrationPlan;
use tern::db::model::Namespace;
use tern::db::state::local::LocalFileBackend;
use tern::db::state::{Migration, MigrationId, StateBackend, StateError, StateHash};

// ============================================================================
// Test Helpers
// ============================================================================

/// Create a LocalFileBackend in a temporary directory.
fn create_temp_backend() -> (TempDir, LocalFileBackend) {
    let temp_dir = TempDir::new().unwrap();
    let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));
    (temp_dir, backend)
}

/// Build a migration from source to target namespace.
fn build_migration(
    source: &Namespace,
    target: &Namespace,
    description: &str,
    parent_hash: StateHash,
) -> (Migration, Namespace) {
    let diff = diff_namespaces(source, target);
    let plan = MigrationPlan::from_diff(&diff);
    let resulting_hash = StateHash::from_namespace(target);

    let migration = Migration::new(
        description,
        plan.operations,
        vec![],
        parent_hash,
        resulting_hash,
        vec![],
    );

    (migration, target.clone())
}

/// Summary for YAML snapshot of state reconstruction test.
#[derive(Debug, serde::Serialize)]
struct StateReconstructionSummary {
    migration_count: usize,
    reconstructed_table_count: usize,
    reconstructed_table_names: Vec<String>,
    matches_expected: bool,
}

// ============================================================================
// Basic State Backend Operations (InMemoryBackend via LocalFileBackend)
// ============================================================================

mod basic_operations {
    use super::*;

    #[tokio::test]
    async fn save_and_retrieve_baseline_migration() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let namespace = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let baseline = Migration::baseline(namespace.clone());
        let baseline_id = baseline.id;

        backend.save_migration(&baseline).await.unwrap();
        backend.save_current_state(&namespace).await.unwrap();

        // Retrieve by ID
        let retrieved = backend.get_migration(&baseline_id).await.unwrap();
        assert_eq!(retrieved.id, baseline_id);
        assert!(retrieved.is_baseline());
        assert!(retrieved.is_checkpoint());

        // Verify index
        let index = backend.get_migration_index().await.unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.contains(&baseline_id));

        // Verify current state
        let current_state = backend.get_current_state().await.unwrap();
        assert_eq!(current_state.tables.len(), 1);
        assert_eq!(current_state.tables[0].name.as_ref(), "users");
    }

    #[tokio::test]
    async fn record_migration_atomically() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let initial = Namespace::empty("public");
        let baseline = Migration::baseline(initial.clone());
        backend.save_migration(&baseline).await.unwrap();
        backend.save_current_state(&initial).await.unwrap();

        // Build a migration that adds a table
        let target = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let (migration, new_state) = build_migration(
            &initial,
            &target,
            "Add users table",
            baseline.resulting_state_hash,
        );

        // Record migration atomically
        backend
            .record_migration(&migration, &new_state)
            .await
            .unwrap();

        // Verify both migration and state were saved
        let index = backend.get_migration_index().await.unwrap();
        assert_eq!(index.len(), 2);

        let current = backend.get_current_state().await.unwrap();
        assert_eq!(current.tables.len(), 1);
    }

    #[tokio::test]
    async fn get_current_state_hash_tracks_latest() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Initially zero
        assert!(backend.get_current_state_hash().await.unwrap().is_zero());

        // After baseline
        let initial = Namespace::empty("public");
        let baseline = Migration::baseline(initial.clone());
        let baseline_hash = baseline.resulting_state_hash;
        backend.save_migration(&baseline).await.unwrap();

        assert_eq!(
            backend.get_current_state_hash().await.unwrap(),
            baseline_hash
        );

        // After second migration
        let target = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let (m2, _) = build_migration(&initial, &target, "Add users", baseline_hash);
        let m2_hash = m2.resulting_state_hash;
        backend.save_migration(&m2).await.unwrap();

        assert_eq!(backend.get_current_state_hash().await.unwrap(), m2_hash);
    }
}

// ============================================================================
// State Reconstruction Tests
// ============================================================================

mod state_reconstruction {
    use super::*;

    #[tokio::test]
    async fn reconstruct_state_at_baseline() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let initial = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();

        let baseline = Migration::baseline(initial.clone());
        let baseline_id = baseline.id;
        backend.save_migration(&baseline).await.unwrap();

        // Reconstruct state at baseline
        let reconstructed = backend.get_state_at(&baseline_id).await.unwrap();

        // Should match the original state
        assert_eq!(reconstructed.tables.len(), 1);
        assert_eq!(reconstructed.tables[0].name.as_ref(), "users");

        // Hash should match
        let reconstructed_hash = StateHash::from_namespace(&reconstructed);
        assert_eq!(reconstructed_hash, baseline.resulting_state_hash);
    }

    #[tokio::test]
    async fn reconstruct_state_after_single_migration() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Baseline: empty schema
        let initial = Namespace::empty("public");
        let baseline = Migration::baseline(initial.clone());
        backend.save_migration(&baseline).await.unwrap();

        // Migration 1: add users table
        let target1 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();

        let (m1, _) = build_migration(
            &initial,
            &target1,
            "Add users table",
            baseline.resulting_state_hash,
        );
        let m1_id = m1.id;
        let m1 = m1.with_checkpoint(target1.clone());
        backend.save_migration(&m1).await.unwrap();

        // Reconstruct at m1
        let reconstructed = backend.get_state_at(&m1_id).await.unwrap();

        assert_eq!(reconstructed.tables.len(), 1);
        assert_eq!(reconstructed.tables[0].name.as_ref(), "users");
        assert_eq!(reconstructed.tables[0].columns.len(), 2);
    }

    #[tokio::test]
    async fn reconstruct_state_through_migration_chain() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Baseline: empty
        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        // M1: Add users table
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users", m0.resulting_state_hash);
        let m1_id = m1.id;
        let m1 = m1.with_checkpoint(ns1.clone());
        backend.save_migration(&m1).await.unwrap();

        // M2: Add posts table
        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("title", "text")
            })
            .build();
        let (m2, _) = build_migration(&ns1, &ns2, "Add posts", m1.resulting_state_hash);
        let m2_id = m2.id;
        let m2 = m2.with_checkpoint(ns2.clone());
        backend.save_migration(&m2).await.unwrap();

        // M3: Add comments table
        let ns3 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
            })
            .table("posts", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("title", "text")
            })
            .table("comments", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("body", "text")
            })
            .build();
        let (m3, _) = build_migration(&ns2, &ns3, "Add comments", m2.resulting_state_hash);
        let m3_id = m3.id;
        let m3 = m3.with_checkpoint(ns3.clone());
        backend.save_migration(&m3).await.unwrap();

        // Reconstruct at each point
        let state_m1 = backend.get_state_at(&m1_id).await.unwrap();
        let state_m2 = backend.get_state_at(&m2_id).await.unwrap();
        let state_m3 = backend.get_state_at(&m3_id).await.unwrap();

        assert_eq!(state_m1.tables.len(), 1);
        assert_eq!(state_m2.tables.len(), 2);
        assert_eq!(state_m3.tables.len(), 3);

        // Verify table names
        let table_names_m3: Vec<_> = state_m3.tables.iter().map(|t| t.name.as_ref()).collect();
        assert!(table_names_m3.contains(&"users"));
        assert!(table_names_m3.contains(&"posts"));
        assert!(table_names_m3.contains(&"comments"));
    }

    #[tokio::test]
    async fn reconstruct_state_with_multiple_operations() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Baseline with initial schema
        let ns0 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        // M1: Add column and new table
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
                    .column("email", "text")
            })
            .table("roles", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column("name", "text")
            })
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add email and roles", m0.resulting_state_hash);
        let m1_id = m1.id;
        let m1 = m1.with_checkpoint(ns1.clone());
        backend.save_migration(&m1).await.unwrap();

        // Reconstruct
        let reconstructed = backend.get_state_at(&m1_id).await.unwrap();

        // Verify users table has 3 columns
        let users_table = reconstructed
            .tables
            .iter()
            .find(|t| t.name.as_ref() == "users")
            .unwrap();
        assert_eq!(users_table.columns.len(), 3);

        // Verify roles table exists
        let roles_table = reconstructed
            .tables
            .iter()
            .find(|t| t.name.as_ref() == "roles");
        assert!(roles_table.is_some());
    }

    #[tokio::test]
    async fn reconstruct_state_not_found() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns = Namespace::empty("public");
        let baseline = Migration::baseline(ns);
        backend.save_migration(&baseline).await.unwrap();

        // Try to reconstruct with non-existent migration ID
        let fake_id = MigrationId::from_bytes([99u8; 32]);
        let result = backend.get_state_at(&fake_id).await;

        assert!(matches!(result, Err(StateError::MigrationNotFound { .. })));
    }
}

// ============================================================================
// Migration Chain Verification Tests
// ============================================================================

mod chain_verification {
    use super::*;

    #[tokio::test]
    async fn verify_chain_succeeds_for_valid_chain() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Create a valid chain of migrations
        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users", m0.resulting_state_hash);
        backend.save_migration(&m1).await.unwrap();

        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .table("posts", |t| t.column("id", "integer"))
            .build();
        let (m2, _) = build_migration(&ns1, &ns2, "Add posts", m1.resulting_state_hash);
        backend.save_migration(&m2).await.unwrap();

        // Should succeed
        backend.verify_chain().await.unwrap();
    }

    #[tokio::test]
    async fn verify_chain_empty_succeeds() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Empty chain should verify successfully
        backend.verify_chain().await.unwrap();
    }

    #[tokio::test]
    async fn verify_chain_single_migration_succeeds() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns = Namespace::empty("public");
        let baseline = Migration::baseline(ns);
        backend.save_migration(&baseline).await.unwrap();

        backend.verify_chain().await.unwrap();
    }
}

// ============================================================================
// Migrations Since Tests
// ============================================================================

mod migrations_since {
    use super::*;

    #[tokio::test]
    async fn get_all_migrations_since_zero() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users", m0.resulting_state_hash);
        backend.save_migration(&m1).await.unwrap();

        // Get all since zero should return all migrations
        let all = backend
            .get_migrations_since(&StateHash::zero())
            .await
            .unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn get_migrations_since_specific_hash() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        let m0_hash = m0.resulting_state_hash;
        backend.save_migration(&m0).await.unwrap();

        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users", m0_hash);
        let m1_hash = m1.resulting_state_hash;
        backend.save_migration(&m1).await.unwrap();

        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .table("posts", |t| t.column("id", "integer"))
            .build();
        let (m2, _) = build_migration(&ns1, &ns2, "Add posts", m1_hash);
        backend.save_migration(&m2).await.unwrap();

        // Get migrations since first migration (should return m1 and m2)
        let since_m0 = backend.get_migrations_since(&m0_hash).await.unwrap();
        assert_eq!(since_m0.len(), 2);
        assert_eq!(since_m0[0].description, "Add users");
        assert_eq!(since_m0[1].description, "Add posts");

        // Get migrations since second migration (should return only m2)
        let since_m1 = backend.get_migrations_since(&m1_hash).await.unwrap();
        assert_eq!(since_m1.len(), 1);
        assert_eq!(since_m1[0].description, "Add posts");
    }

    #[tokio::test]
    async fn get_migrations_since_latest_returns_empty() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns = Namespace::empty("public");
        let baseline = Migration::baseline(ns);
        let latest_hash = baseline.resulting_state_hash;
        backend.save_migration(&baseline).await.unwrap();

        // Get migrations since the latest should return empty
        let since_latest = backend.get_migrations_since(&latest_hash).await.unwrap();
        assert!(since_latest.is_empty());
    }
}

// ============================================================================
// Persistence Tests
// ============================================================================

mod persistence {
    use super::*;

    #[tokio::test]
    async fn data_persists_across_backend_instances() {
        let temp_dir = TempDir::new().unwrap();
        let root_path = temp_dir.path().join(".tern");

        // Create migration IDs for verification
        let baseline_id;
        let m1_id;

        // First instance - save migrations
        {
            let backend = LocalFileBackend::new(&root_path);
            backend.initialize().await.unwrap();

            let ns0 = NamespaceBuilder::new("public")
                .table("users", |t| t.column("id", "integer"))
                .build();
            let baseline = Migration::baseline(ns0.clone());
            baseline_id = baseline.id;
            backend.save_migration(&baseline).await.unwrap();
            backend.save_current_state(&ns0).await.unwrap();

            let ns1 = NamespaceBuilder::new("public")
                .table("users", |t| t.column("id", "integer"))
                .table("posts", |t| t.column("id", "integer"))
                .build();
            let (m1, _) = build_migration(&ns0, &ns1, "Add posts", baseline.resulting_state_hash);
            m1_id = m1.id;
            backend.save_migration(&m1).await.unwrap();
            backend.save_current_state(&ns1).await.unwrap();
        }

        // Second instance - verify data persisted
        {
            let backend = LocalFileBackend::new(&root_path);
            assert!(backend.is_initialized().await.unwrap());

            let index = backend.get_migration_index().await.unwrap();
            assert_eq!(index.len(), 2);

            // Retrieve migrations by ID
            let retrieved_baseline = backend.get_migration(&baseline_id).await.unwrap();
            assert!(retrieved_baseline.is_baseline());

            let retrieved_m1 = backend.get_migration(&m1_id).await.unwrap();
            assert_eq!(retrieved_m1.description, "Add posts");

            // Verify current state
            let current = backend.get_current_state().await.unwrap();
            assert_eq!(current.tables.len(), 2);
        }
    }

    #[tokio::test]
    async fn files_numbered_sequentially() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalFileBackend::new(temp_dir.path().join(".tern"));
        backend.initialize().await.unwrap();

        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users", m0.resulting_state_hash);
        backend.save_migration(&m1).await.unwrap();

        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .table("posts", |t| t.column("id", "integer"))
            .build();
        let (m2, _) = build_migration(&ns1, &ns2, "Add posts", m1.resulting_state_hash);
        backend.save_migration(&m2).await.unwrap();

        // Verify files exist with sequential names
        let migrations_dir = temp_dir.path().join(".tern/migrations");
        assert!(migrations_dir.join("00001.json").exists());
        assert!(migrations_dir.join("00002.json").exists());
        assert!(migrations_dir.join("00003.json").exists());
        assert!(!migrations_dir.join("00004.json").exists());
    }
}

// ============================================================================
// Snapshot Tests for State Reconstruction
// ============================================================================

mod snapshot_tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_state_reconstruction_chain() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        // Build a chain of migrations
        let ns0 = Namespace::empty("public");
        let m0 = Migration::baseline(ns0.clone());
        backend.save_migration(&m0).await.unwrap();

        let ns1 = NamespaceBuilder::new("public")
            .enum_type("status", ["active", "inactive"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .column("status", "status")
            })
            .build();
        let (m1, _) = build_migration(&ns0, &ns1, "Add users with enum", m0.resulting_state_hash);
        let m1_id = m1.id;
        let m1 = m1.with_checkpoint(ns1.clone());
        backend.save_migration(&m1).await.unwrap();

        let ns2 = NamespaceBuilder::new("public")
            .enum_type("status", ["active", "inactive"])
            .table("users", |t| {
                t.column_with("id", "integer", |c| c.not_null())
                    .column_with("email", "text", |c| c.not_null())
                    .column("status", "status")
                    .primary_key(["id"])
                    .unique(["email"])
            })
            .build();
        let (m2, _) = build_migration(&ns1, &ns2, "Add constraints", m1.resulting_state_hash);
        let m2_id = m2.id;
        let m2 = m2.with_checkpoint(ns2.clone());
        backend.save_migration(&m2).await.unwrap();

        // Reconstruct at each point
        let state_m1 = backend.get_state_at(&m1_id).await.unwrap();
        let state_m2 = backend.get_state_at(&m2_id).await.unwrap();

        // Create summaries
        let summary_m1 = StateReconstructionSummary {
            migration_count: 2,
            reconstructed_table_count: state_m1.tables.len(),
            reconstructed_table_names: state_m1
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
            matches_expected: state_m1.tables.len() == 1 && state_m1.tables[0].columns.len() == 3,
        };

        let summary_m2 = StateReconstructionSummary {
            migration_count: 3,
            reconstructed_table_count: state_m2.tables.len(),
            reconstructed_table_names: state_m2
                .tables
                .iter()
                .map(|t| t.name.as_ref().to_string())
                .collect(),
            matches_expected: state_m2.tables.len() == 1
                && state_m2.tables[0].constraints.len() == 2,
        };

        assert_yaml_snapshot!("state_at_m1", summary_m1);
        assert_yaml_snapshot!("state_at_m2", summary_m2);
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

mod error_handling {
    use super::*;

    #[tokio::test]
    async fn duplicate_migration_fails() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let ns = Namespace::empty("public");
        let baseline = Migration::baseline(ns);

        backend.save_migration(&baseline).await.unwrap();

        // Try to save the same migration again
        let result = backend.save_migration(&baseline).await;
        assert!(matches!(result, Err(StateError::DuplicateMigration { .. })));
    }

    #[tokio::test]
    async fn get_migration_not_found() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let fake_id = MigrationId::from_bytes([42u8; 32]);
        let result = backend.get_migration(&fake_id).await;

        assert!(matches!(result, Err(StateError::MigrationNotFound { .. })));
    }

    #[tokio::test]
    async fn operations_fail_before_init() {
        let (_temp_dir, backend) = create_temp_backend();

        let result = backend.get_migration_index().await;
        assert!(matches!(result, Err(StateError::NotInitialized { .. })));
    }

    #[tokio::test]
    async fn get_current_state_fails_when_empty() {
        let (_temp_dir, backend) = create_temp_backend();
        backend.initialize().await.unwrap();

        let result = backend.get_current_state().await;
        assert!(matches!(result, Err(StateError::EmptyHistory)));
    }
}

// ============================================================================
// Migration Properties Tests
// ============================================================================

mod migration_properties {
    use super::*;

    #[tokio::test]
    async fn baseline_migration_properties() {
        let ns = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let baseline = Migration::baseline(ns.clone());

        assert!(baseline.is_baseline());
        assert!(baseline.is_checkpoint());
        assert!(!baseline.has_breaking_changes());
        assert_eq!(baseline.operation_count(), 0);
        assert!(baseline.parent_state_hash.is_zero());
        assert!(!baseline.resulting_state_hash.is_zero());
    }

    #[tokio::test]
    async fn regular_migration_properties() {
        let ns0 = Namespace::empty("public");
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let (migration, _) = build_migration(&ns0, &ns1, "Add users", StateHash::zero());

        assert!(!migration.is_baseline());
        assert!(!migration.is_checkpoint());
        assert!(!migration.has_breaking_changes());
        assert!(migration.operation_count() > 0);
    }

    #[tokio::test]
    async fn checkpoint_migration_properties() {
        let ns0 = Namespace::empty("public");
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let (migration, _) = build_migration(&ns0, &ns1, "Add users", StateHash::zero());
        let checkpoint = migration.with_checkpoint(ns1);

        assert!(checkpoint.is_checkpoint());
        assert!(checkpoint.checkpoint_state.is_some());
    }
}

// ============================================================================
// State Hash Stability Tests
// ============================================================================

mod state_hash_stability {
    use super::*;

    #[test]
    fn empty_namespace_hash_is_stable() {
        let ns = Namespace::empty("public");
        let hash = StateHash::from_namespace(&ns);

        // This hash should match the one documented in the design document
        assert_eq!(
            hash.to_hex(),
            "3fa55ab02853d2d983c739392e9e19bc11c1292b56f20abeae00ee7dd0f477b3",
            "Empty namespace hash has changed! This is a breaking change."
        );
    }

    #[test]
    fn hash_from_hex_roundtrip() {
        let ns = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let hash = StateHash::from_namespace(&ns);
        let hex = hash.to_hex();
        let parsed = StateHash::from_hex(&hex).unwrap();

        assert_eq!(hash, parsed);
    }

    #[test]
    fn different_schemas_have_different_hashes() {
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let ns2 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "bigint"))
            .build();

        let hash1 = StateHash::from_namespace(&ns1);
        let hash2 = StateHash::from_namespace(&ns2);

        assert_ne!(hash1, hash2);
    }
}

// ============================================================================
// Migration ID Tests
// ============================================================================

mod migration_id_tests {
    use super::*;

    #[test]
    fn migration_id_from_content_is_deterministic() {
        let ns0 = Namespace::empty("public");
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let parent_hash = StateHash::from_namespace(&ns0);

        let (m1, _) = build_migration(&ns0, &ns1, "Add users", parent_hash);
        let (m2, _) = build_migration(&ns0, &ns1, "Add users", parent_hash);

        // Same input should produce same ID
        assert_eq!(m1.id, m2.id);
    }

    #[test]
    fn migration_id_from_hex_roundtrip() {
        let ns0 = Namespace::empty("public");
        let ns1 = NamespaceBuilder::new("public")
            .table("users", |t| t.column("id", "integer"))
            .build();

        let (migration, _) = build_migration(&ns0, &ns1, "Add users", StateHash::zero());

        let hex = migration.id.to_hex();
        let parsed = MigrationId::from_hex(&hex).unwrap();

        assert_eq!(migration.id, parsed);
    }

    #[test]
    fn migration_id_short_hex_is_16_chars() {
        let ns = Namespace::empty("public");
        let baseline = Migration::baseline(ns);

        assert_eq!(baseline.id.to_short_hex().len(), 16);
    }
}
