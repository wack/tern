//! Core types for migration state tracking.
//!
//! This module defines the fundamental types for content-addressable migration
//! identification and schema state hashing.
//!
//! # Hash Algorithm
//!
//! Uses BLAKE3, a cryptographic hash function that provides:
//! - Collision resistance: Computationally infeasible to find two inputs with the same hash
//! - Preimage resistance: Cannot reverse a hash to find the original input
//! - Tamper resistance: Any modification to input produces a completely different hash
//! - High performance: Faster than SHA-256 while being cryptographically secure
//!
//! Hash outputs are 256 bits (32 bytes), displayed as 64 hexadecimal characters.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::db::diff::breaking::BreakingChange;
use crate::db::migrate::Operation;
use crate::db::model::Namespace;

// =============================================================================
// MigrationId
// =============================================================================

/// A content-addressable identifier for a migration.
///
/// The hash is computed from the migration's operations and metadata,
/// ensuring that identical migrations produce identical IDs regardless
/// of when or where they were generated.
///
/// Uses BLAKE3 for cryptographic security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MigrationId(#[serde(with = "hash_serde")] pub [u8; 32]);

impl MigrationId {
    /// Domain separation tag for migration ID hashing.
    /// This ensures migration hashes won't collide with state hashes.
    const DOMAIN: &'static str = "tern.migration.v1";

    /// Compute the migration ID from its content.
    ///
    /// The hash is computed from:
    /// - Domain separation tag
    /// - The parent state hash
    /// - The operations (serialized as canonical JSON)
    /// - The description
    ///
    /// This ensures that two migrations with identical content produce
    /// identical IDs, enabling content-addressable storage.
    pub fn from_content(
        operations: &[Operation],
        parent_state_hash: &StateHash,
        description: &str,
    ) -> Self {
        let mut hasher = blake3::Hasher::new();

        // Domain separation
        hasher.update(Self::DOMAIN.as_bytes());
        hasher.update(&[0u8]); // null separator

        // Hash the parent state
        hasher.update(&parent_state_hash.0);

        // Hash the operations (using canonical JSON serialization)
        let ops_json =
            serde_json::to_vec(operations).expect("operations should be serializable to JSON");
        hasher.update(&(ops_json.len() as u64).to_le_bytes()); // length prefix
        hasher.update(&ops_json);

        // Hash the description
        hasher.update(&(description.len() as u64).to_le_bytes()); // length prefix
        hasher.update(description.as_bytes());

        Self(*hasher.finalize().as_bytes())
    }

    /// Create a MigrationId from raw bytes.
    ///
    /// This is primarily used for deserialization and testing.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Format as a 64-character hexadecimal string.
    ///
    /// This is the canonical string representation of a migration ID.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Format as a short hexadecimal string (first 16 characters).
    ///
    /// Useful for display purposes where the full hash is too long.
    #[must_use]
    pub fn to_short_hex(&self) -> String {
        hex::encode(&self.0[..8])
    }

    /// Parse a migration ID from a hexadecimal string.
    ///
    /// # Errors
    ///
    /// Returns `None` if the string is not valid hexadecimal or is not 64 characters.
    #[must_use]
    pub fn from_hex(hex_str: &str) -> Option<Self> {
        if hex_str.len() != 64 {
            return None;
        }
        let bytes = hex::decode(hex_str).ok()?;
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(Self(arr))
    }

    /// Parse a migration ID from a hexadecimal string, accepting short prefixes.
    ///
    /// This is useful for user input where the full hash may be abbreviated.
    /// The hash will be zero-padded on the right if shorter than 64 characters.
    ///
    /// # Errors
    ///
    /// Returns `None` if the string is not valid hexadecimal or is too long.
    #[must_use]
    pub fn from_hex_prefix(hex_str: &str) -> Option<Self> {
        if hex_str.len() > 64 {
            return None;
        }
        // Pad with zeros on the right
        let padded = format!("{:0<64}", hex_str);
        Self::from_hex(&padded)
    }

    /// A zero migration ID, used for the initial/baseline state.
    #[must_use]
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    /// Returns true if this is the zero migration ID.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl fmt::Display for MigrationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

// =============================================================================
// StateHash
// =============================================================================

/// A hash of the schema state.
///
/// Uses BLAKE3 for cryptographic security.
/// The hash is computed from the canonical JSON serialization of a Namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateHash(#[serde(with = "hash_serde")] pub [u8; 32]);

impl StateHash {
    /// Domain separation tag for state hashing.
    /// This ensures state hashes won't collide with migration hashes.
    const DOMAIN: &'static str = "tern.state.v1";

    /// Compute the state hash from a namespace.
    ///
    /// The hash is computed from the canonical JSON serialization of the
    /// namespace, ensuring that structurally identical schemas produce
    /// identical hashes.
    pub fn from_namespace(namespace: &Namespace) -> Self {
        let mut hasher = blake3::Hasher::new();

        // Domain separation
        hasher.update(Self::DOMAIN.as_bytes());
        hasher.update(&[0u8]); // null separator

        // Hash the namespace
        let json = serde_json::to_vec(namespace).expect("namespace should be serializable to JSON");
        hasher.update(&json);

        Self(*hasher.finalize().as_bytes())
    }

    /// Create a StateHash from raw bytes.
    ///
    /// This is primarily used for deserialization and testing.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Get the raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Format as a 64-character hexadecimal string.
    ///
    /// This is the canonical string representation of a state hash.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Format as a short hexadecimal string (first 16 characters).
    ///
    /// Useful for display purposes where the full hash is too long.
    #[must_use]
    pub fn to_short_hex(&self) -> String {
        hex::encode(&self.0[..8])
    }

    /// Parse a state hash from a hexadecimal string.
    ///
    /// # Errors
    ///
    /// Returns `None` if the string is not valid hexadecimal or is not 64 characters.
    #[must_use]
    pub fn from_hex(hex_str: &str) -> Option<Self> {
        if hex_str.len() != 64 {
            return None;
        }
        let bytes = hex::decode(hex_str).ok()?;
        let arr: [u8; 32] = bytes.try_into().ok()?;
        Some(Self(arr))
    }

    /// A zero state hash, representing an empty/initial state.
    #[must_use]
    pub const fn zero() -> Self {
        Self([0u8; 32])
    }

    /// Returns true if this is the zero state hash.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 32]
    }
}

impl fmt::Display for StateHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Custom serde module for [u8; 32] as hex strings.
mod hash_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64 hex characters"))
    }
}

// =============================================================================
// Migration
// =============================================================================

/// A recorded migration in the history.
///
/// Contains all information needed to understand and replay the migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Migration {
    /// Unique identifier (content hash).
    pub id: MigrationId,

    /// Human-readable description.
    pub description: String,

    /// When the migration was created (not applied).
    pub created_at: DateTime<Utc>,

    /// The operations that make up this migration.
    pub operations: Vec<Operation>,

    /// Hash of the schema state before this migration.
    pub parent_state_hash: StateHash,

    /// Hash of the schema state after this migration.
    pub resulting_state_hash: StateHash,

    /// Breaking changes detected in this migration.
    pub breaking_changes: Vec<BreakingChange>,

    /// Optional: the full schema state after this migration.
    /// Included for checkpoint migrations to allow fast state reconstruction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_state: Option<Namespace>,
}

impl Migration {
    /// Creates a new migration with the given parameters.
    ///
    /// The migration ID is computed automatically from the content.
    #[must_use]
    pub fn new(
        description: impl Into<String>,
        operations: Vec<Operation>,
        parent_state_hash: StateHash,
        resulting_state_hash: StateHash,
        breaking_changes: Vec<BreakingChange>,
    ) -> Self {
        let description = description.into();
        let id = MigrationId::from_content(&operations, &parent_state_hash, &description);

        Self {
            id,
            description,
            created_at: Utc::now(),
            operations,
            parent_state_hash,
            resulting_state_hash,
            breaking_changes,
            checkpoint_state: None,
        }
    }

    /// Creates a baseline migration from an existing database state.
    ///
    /// A baseline migration has no operations and captures the current
    /// state as the starting point for future migrations.
    #[must_use]
    pub fn baseline(namespace: Namespace) -> Self {
        let description = "Baseline migration from existing database".to_string();
        let id = MigrationId::from_content(&[], &StateHash::zero(), &description);
        let resulting_hash = StateHash::from_namespace(&namespace);

        Self {
            id,
            description,
            created_at: Utc::now(),
            operations: vec![],
            parent_state_hash: StateHash::zero(),
            resulting_state_hash: resulting_hash,
            breaking_changes: vec![],
            checkpoint_state: Some(namespace),
        }
    }

    /// Creates a checkpoint migration that includes the full schema state.
    ///
    /// Checkpoint migrations enable fast state reconstruction by storing
    /// the complete schema state, avoiding the need to replay all prior
    /// operations.
    #[must_use]
    pub fn with_checkpoint(mut self, state: Namespace) -> Self {
        self.checkpoint_state = Some(state);
        self
    }

    /// Returns true if this migration has breaking changes.
    #[must_use]
    pub fn has_breaking_changes(&self) -> bool {
        !self.breaking_changes.is_empty()
    }

    /// Returns true if this migration is a checkpoint (includes full state).
    #[must_use]
    pub fn is_checkpoint(&self) -> bool {
        self.checkpoint_state.is_some()
    }

    /// Returns true if this is a baseline migration (no operations).
    #[must_use]
    pub fn is_baseline(&self) -> bool {
        self.operations.is_empty() && self.parent_state_hash.is_zero()
    }

    /// Returns the number of operations in this migration.
    #[must_use]
    pub fn operation_count(&self) -> usize {
        self.operations.len()
    }
}

// =============================================================================
// MigrationIndex
// =============================================================================

/// Index of migrations in order of application.
///
/// This is stored in `.tern/migrations/index.json` and provides
/// the ordered list of migration IDs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MigrationIndex {
    /// Ordered list of migration IDs.
    pub migrations: Vec<MigrationId>,
}

impl MigrationIndex {
    /// Creates a new empty migration index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of migrations in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.migrations.len()
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.migrations.is_empty()
    }

    /// Adds a migration ID to the index.
    pub fn push(&mut self, id: MigrationId) {
        self.migrations.push(id);
    }

    /// Returns true if the index contains the given migration ID.
    #[must_use]
    pub fn contains(&self, id: &MigrationId) -> bool {
        self.migrations.contains(id)
    }

    /// Returns the position of a migration ID in the index, if present.
    #[must_use]
    pub fn position(&self, id: &MigrationId) -> Option<usize> {
        self.migrations.iter().position(|m| m == id)
    }

    /// Returns the last migration ID, if any.
    #[must_use]
    pub fn last(&self) -> Option<&MigrationId> {
        self.migrations.last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod migration_id_tests {
        use super::*;

        #[test]
        fn zero_has_correct_hex() {
            let zero = MigrationId::zero();
            assert_eq!(zero.to_hex().len(), 64);
            assert!(zero.to_hex().chars().all(|c| c == '0'));
        }

        #[test]
        fn is_zero_works() {
            assert!(MigrationId::zero().is_zero());
            assert!(!MigrationId::from_bytes([1u8; 32]).is_zero());
        }

        #[test]
        fn hex_format_is_64_characters() {
            let id = MigrationId::from_bytes([0xab; 32]);
            assert_eq!(id.to_hex().len(), 64);
            assert!(id.to_hex().chars().all(|c| c == 'a' || c == 'b'));
        }

        #[test]
        fn short_hex_is_16_characters() {
            let id = MigrationId::from_bytes([0xab; 32]);
            assert_eq!(id.to_short_hex().len(), 16);
        }

        #[test]
        fn from_hex_works() {
            let hex = "a".repeat(64);
            let id = MigrationId::from_hex(&hex).unwrap();
            assert_eq!(id.0, [0xaa; 32]);
        }

        #[test]
        fn from_hex_rejects_wrong_length() {
            assert!(MigrationId::from_hex("abc").is_none());
            assert!(MigrationId::from_hex(&"a".repeat(65)).is_none());
        }

        #[test]
        fn from_hex_rejects_invalid() {
            assert!(MigrationId::from_hex(&"g".repeat(64)).is_none());
        }

        #[test]
        fn from_hex_prefix_pads_with_zeros() {
            let id = MigrationId::from_hex_prefix("ab").unwrap();
            assert!(id.to_hex().starts_with("ab"));
            assert!(id.to_hex()[2..].chars().all(|c| c == '0'));
        }

        #[test]
        fn display_uses_hex() {
            let id = MigrationId::from_bytes([0xab; 32]);
            let displayed = format!("{}", id);
            assert_eq!(displayed.len(), 64);
        }

        #[test]
        fn different_content_produces_different_ids() {
            let parent = StateHash::zero();
            let id1 = MigrationId::from_content(&[], &parent, "desc1");
            let id2 = MigrationId::from_content(&[], &parent, "desc2");
            assert_ne!(id1, id2);
        }

        #[test]
        fn same_content_produces_same_id() {
            let parent = StateHash::zero();
            let id1 = MigrationId::from_content(&[], &parent, "test");
            let id2 = MigrationId::from_content(&[], &parent, "test");
            assert_eq!(id1, id2);
        }

        #[test]
        fn different_parent_produces_different_id() {
            let parent1 = StateHash::zero();
            let parent2 = StateHash::from_bytes([1u8; 32]);
            let id1 = MigrationId::from_content(&[], &parent1, "test");
            let id2 = MigrationId::from_content(&[], &parent2, "test");
            assert_ne!(id1, id2);
        }
    }

    mod state_hash_tests {
        use super::*;

        #[test]
        fn zero_has_correct_hex() {
            let zero = StateHash::zero();
            assert_eq!(zero.to_hex().len(), 64);
            assert!(zero.to_hex().chars().all(|c| c == '0'));
        }

        #[test]
        fn is_zero_works() {
            assert!(StateHash::zero().is_zero());
            assert!(!StateHash::from_bytes([1u8; 32]).is_zero());
        }

        #[test]
        fn hex_format_is_64_characters() {
            let hash = StateHash::from_bytes([0xfe; 32]);
            assert_eq!(hash.to_hex().len(), 64);
        }

        #[test]
        fn short_hex_is_16_characters() {
            let hash = StateHash::from_bytes([0xfe; 32]);
            assert_eq!(hash.to_short_hex().len(), 16);
        }

        #[test]
        fn from_hex_works() {
            let hex = "f".repeat(64);
            let hash = StateHash::from_hex(&hex).unwrap();
            assert_eq!(hash.0, [0xff; 32]);
        }

        #[test]
        fn display_uses_hex() {
            let hash = StateHash::from_bytes([0x12; 32]);
            let displayed = format!("{}", hash);
            assert_eq!(displayed.len(), 64);
        }

        #[test]
        fn same_namespace_produces_same_hash() {
            let ns1 = Namespace::empty("public");
            let ns2 = Namespace::empty("public");
            assert_eq!(
                StateHash::from_namespace(&ns1),
                StateHash::from_namespace(&ns2)
            );
        }

        #[test]
        fn different_namespace_produces_different_hash() {
            let ns1 = Namespace::empty("public");
            let ns2 = Namespace::empty("private");
            assert_ne!(
                StateHash::from_namespace(&ns1),
                StateHash::from_namespace(&ns2)
            );
        }
    }

    mod migration_tests {
        use super::*;

        #[test]
        fn baseline_migration_properties() {
            let ns = Namespace::empty("public");
            let migration = Migration::baseline(ns);

            assert!(migration.is_baseline());
            assert!(migration.is_checkpoint());
            assert!(!migration.has_breaking_changes());
            assert_eq!(migration.operation_count(), 0);
            assert!(migration.parent_state_hash.is_zero());
        }

        #[test]
        fn new_migration_computes_id() {
            let parent = StateHash::zero();
            let result = StateHash::from_bytes([1u8; 32]);
            let m1 = Migration::new("test", vec![], parent, result, vec![]);
            let m2 = Migration::new("test", vec![], parent, result, vec![]);

            // Same content should produce same ID
            assert_eq!(m1.id, m2.id);
        }

        #[test]
        fn with_checkpoint_sets_state() {
            let parent = StateHash::zero();
            let result = StateHash::from_bytes([1u8; 32]);
            let migration = Migration::new("test", vec![], parent, result, vec![]);
            assert!(!migration.is_checkpoint());

            let ns = Namespace::empty("public");
            let migration = migration.with_checkpoint(ns);
            assert!(migration.is_checkpoint());
        }
    }

    mod migration_index_tests {
        use super::*;

        #[test]
        fn new_index_is_empty() {
            let index = MigrationIndex::new();
            assert!(index.is_empty());
            assert_eq!(index.len(), 0);
        }

        #[test]
        fn push_adds_migration() {
            let mut index = MigrationIndex::new();
            let id = MigrationId::from_bytes([1u8; 32]);
            index.push(id);

            assert!(!index.is_empty());
            assert_eq!(index.len(), 1);
            assert!(index.contains(&id));
        }

        #[test]
        fn position_finds_migration() {
            let mut index = MigrationIndex::new();
            let id1 = MigrationId::from_bytes([1u8; 32]);
            let id2 = MigrationId::from_bytes([2u8; 32]);
            let id3 = MigrationId::from_bytes([3u8; 32]);

            index.push(id1);
            index.push(id2);
            index.push(id3);

            assert_eq!(index.position(&id1), Some(0));
            assert_eq!(index.position(&id2), Some(1));
            assert_eq!(index.position(&id3), Some(2));
            assert_eq!(index.position(&MigrationId::from_bytes([99u8; 32])), None);
        }

        #[test]
        fn last_returns_last_migration() {
            let mut index = MigrationIndex::new();
            assert_eq!(index.last(), None);

            let id1 = MigrationId::from_bytes([1u8; 32]);
            let id2 = MigrationId::from_bytes([2u8; 32]);

            index.push(id1);
            assert_eq!(index.last(), Some(&id1));

            index.push(id2);
            assert_eq!(index.last(), Some(&id2));
        }
    }

    mod serde_tests {
        use super::*;

        #[test]
        fn migration_id_roundtrips_through_json() {
            let id = MigrationId::from_bytes([0xab; 32]);
            let json = serde_json::to_string(&id).unwrap();
            let deserialized: MigrationId = serde_json::from_str(&json).unwrap();
            assert_eq!(id, deserialized);
        }

        #[test]
        fn state_hash_roundtrips_through_json() {
            let hash = StateHash::from_bytes([0xcd; 32]);
            let json = serde_json::to_string(&hash).unwrap();
            let deserialized: StateHash = serde_json::from_str(&json).unwrap();
            assert_eq!(hash, deserialized);
        }

        #[test]
        fn migration_roundtrips_through_json() {
            let ns = Namespace::empty("public");
            let migration = Migration::baseline(ns);
            let json = serde_json::to_string(&migration).unwrap();
            let deserialized: Migration = serde_json::from_str(&json).unwrap();
            assert_eq!(migration.id, deserialized.id);
            assert_eq!(migration.description, deserialized.description);
        }

        #[test]
        fn migration_index_roundtrips_through_json() {
            let mut index = MigrationIndex::new();
            index.push(MigrationId::from_bytes([1u8; 32]));
            index.push(MigrationId::from_bytes([2u8; 32]));

            let json = serde_json::to_string(&index).unwrap();
            let deserialized: MigrationIndex = serde_json::from_str(&json).unwrap();
            assert_eq!(index.len(), deserialized.len());
        }
    }

    /// Hash stability tests.
    ///
    /// These tests verify that hash values remain stable across versions.
    /// If any of these tests fail, it indicates a breaking change in the
    /// hash algorithm that would invalidate existing migration histories.
    ///
    /// DO NOT UPDATE THESE EXPECTED VALUES unless you are intentionally
    /// making a breaking change to the hash format.
    mod hash_stability_tests {
        use super::*;

        #[test]
        fn migration_id_empty_operations_is_stable() {
            let parent_hash = StateHash::zero();
            let operations: Vec<Operation> = vec![];
            let description = "";

            let id = MigrationId::from_content(&operations, &parent_hash, description);

            // This value must remain stable across versions
            // BLAKE3 hash of: domain + null + parent(zeros) + len(0) + ops([]) + len(0) + desc("")
            assert_eq!(
                id.to_hex(),
                "5ac039f690bd2a0f12892e4b9be6ca586bbfe63c1f612f2840485a90c2e5884f",
                "MigrationId hash for empty operations has changed! This is a breaking change."
            );
        }

        #[test]
        fn migration_id_with_description_is_stable() {
            let parent_hash = StateHash::zero();
            let operations: Vec<Operation> = vec![];
            let description = "Add users table";

            let id = MigrationId::from_content(&operations, &parent_hash, description);

            // This value must remain stable across versions
            assert_eq!(
                id.to_hex(),
                "bfc82bb0258f1cb757ae1dc972f558ddcc0b4d588131e96103cbd979cd5668f4",
                "MigrationId hash with description has changed! This is a breaking change."
            );
        }

        #[test]
        fn migration_id_with_parent_hash_is_stable() {
            // Parent hash with all 0x12 bytes
            let parent_hash = StateHash::from_bytes([0x12; 32]);
            let operations: Vec<Operation> = vec![];
            let description = "";

            let id = MigrationId::from_content(&operations, &parent_hash, description);

            // This value must remain stable across versions
            assert_eq!(
                id.to_hex(),
                "bb027ea09672b498cc6cfb8ae1d655bf746f5649188399679662478c919d8367",
                "MigrationId hash with parent hash has changed! This is a breaking change."
            );
        }

        #[test]
        fn state_hash_empty_namespace_is_stable() {
            let namespace = Namespace::empty("public");

            let hash = StateHash::from_namespace(&namespace);

            // This value must remain stable across versions
            assert_eq!(
                hash.to_hex(),
                "3fa55ab02853d2d983c739392e9e19bc11c1292b56f20abeae00ee7dd0f477b3",
                "StateHash for empty namespace has changed! This is a breaking change."
            );
        }
    }
}
