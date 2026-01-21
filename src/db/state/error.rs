//! Error types for state backend operations.
//!
//! This module defines errors that can occur when reading, writing, or
//! validating migration state.

// Allow false positives from thiserror derive macro - these fields ARE used
// in the generated Display implementations via the #[error("...")] attribute.
#![allow(unused_assignments)]

use std::path::PathBuf;

use thiserror::Error;

use super::types::{MigrationId, StateHash};

/// Errors that can occur during state backend operations.
#[derive(Debug, Error, miette::Diagnostic)]
pub enum StateError {
    /// The migration directory has not been initialized.
    #[error("migration directory not initialized: {path}")]
    #[diagnostic(
        code(tern::state::not_initialized),
        help("Run 'tern init' to initialize the migration directory")
    )]
    NotInitialized {
        /// The path that was expected to exist.
        path: PathBuf,
    },

    /// Failed to read migration index.
    #[error("failed to read migration index: {source}")]
    #[diagnostic(code(tern::state::read_index))]
    ReadIndex {
        #[source]
        source: std::io::Error,
    },

    /// Failed to write migration index.
    #[error("failed to write migration index: {source}")]
    #[diagnostic(code(tern::state::write_index))]
    WriteIndex {
        #[source]
        source: std::io::Error,
    },

    /// Failed to read a migration.
    #[error("failed to read migration {id}: {source}")]
    #[diagnostic(code(tern::state::read_migration))]
    ReadMigration {
        /// The migration ID that failed to read.
        id: MigrationId,
        #[source]
        source: std::io::Error,
    },

    /// Failed to write a migration.
    #[error("failed to write migration {id}: {source}")]
    #[diagnostic(code(tern::state::write_migration))]
    WriteMigration {
        /// The migration ID that failed to write.
        id: MigrationId,
        #[source]
        source: std::io::Error,
    },

    /// A migration was not found.
    #[error("migration not found: {id}")]
    #[diagnostic(code(tern::state::migration_not_found))]
    MigrationNotFound {
        /// The migration ID that was not found.
        id: MigrationId,
    },

    /// Failed to parse migration JSON.
    #[error("invalid migration JSON for {id}: {source}")]
    #[diagnostic(code(tern::state::invalid_json))]
    InvalidMigrationJson {
        /// The migration ID that failed to parse.
        id: MigrationId,
        #[source]
        source: serde_json::Error,
    },

    /// Failed to serialize migration to JSON.
    #[error("failed to serialize migration {id}: {source}")]
    #[diagnostic(code(tern::state::serialize_migration))]
    SerializeMigration {
        /// The migration ID that failed to serialize.
        id: MigrationId,
        #[source]
        source: serde_json::Error,
    },

    /// Failed to parse migration index JSON.
    #[error("invalid migration index JSON: {source}")]
    #[diagnostic(code(tern::state::invalid_index_json))]
    InvalidIndexJson {
        #[source]
        source: serde_json::Error,
    },

    /// Failed to serialize migration index to JSON.
    #[error("failed to serialize migration index: {source}")]
    #[diagnostic(code(tern::state::serialize_index))]
    SerializeIndex {
        #[source]
        source: serde_json::Error,
    },

    /// Migration already exists.
    #[error("migration already exists: {id}")]
    #[diagnostic(
        code(tern::state::duplicate_migration),
        help("Migration with the same content already exists in the history")
    )]
    DuplicateMigration {
        /// The duplicate migration ID.
        id: MigrationId,
    },

    /// State hash mismatch during migration.
    #[error("state hash mismatch: expected {expected}, got {actual}")]
    #[diagnostic(
        code(tern::state::hash_mismatch),
        help("The migration history may be corrupted or out of sync")
    )]
    HashMismatch {
        /// The expected state hash.
        expected: StateHash,
        /// The actual state hash.
        actual: StateHash,
    },

    /// Failed to create directory.
    #[error("failed to create directory {path}: {source}")]
    #[diagnostic(code(tern::state::create_dir))]
    CreateDirectory {
        /// The path that failed to create.
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Invalid migration chain - breaks parent link.
    #[error("migration {id} has parent hash {parent} but expected {expected}")]
    #[diagnostic(
        code(tern::state::broken_chain),
        help("The migration chain is broken - verify migration history")
    )]
    BrokenChain {
        /// The migration with the broken chain.
        id: MigrationId,
        /// The parent hash in the migration.
        parent: StateHash,
        /// The expected parent hash.
        expected: StateHash,
    },

    /// Empty migration history when one was expected.
    #[error("migration history is empty")]
    #[diagnostic(
        code(tern::state::empty_history),
        help("Initialize with a baseline migration using 'tern init'")
    )]
    EmptyHistory,

    /// Generic I/O error.
    #[error("I/O error: {source}")]
    #[diagnostic(code(tern::state::io))]
    Io {
        #[source]
        source: std::io::Error,
    },
}

impl From<std::io::Error> for StateError {
    fn from(source: std::io::Error) -> Self {
        Self::Io { source }
    }
}
