# Standalone Migration Executables via WebAssembly

## Overview

This document describes the design for compiling database migrations into standalone, platform-native executables using WebAssembly. When run, these executables connect to a PostgreSQL database and apply the migration.

### Goals

1. **Self-contained executables**: A single binary that contains all migration logic
2. **Platform independence**: Generate executables for any supported target platform
3. **No runtime dependencies**: Users don't need Tern installed to run migrations
4. **Breaking change warnings**: Warn users about potentially dangerous operations
5. **Inspectable artifacts**: Extract metadata and SQL without executing
6. **State-based tracking**: Maintain migration history in a state backend rather than comparing two live databases

### Non-Goals (Deferred)

- User-provided code hooks for custom migration logic
- Rollback execution (generate rollback SQL for reference only)
- Multi-database support (PostgreSQL only initially)

---

## Architecture

### High-Level Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Compilation Pipeline                         │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────┐     ┌─────────┐     ┌─────────┐     ┌──────────────┐  │
│  │ Source  │     │ Schema  │     │Migration│     │    Wasm      │  │
│  │ Schema  │────▶│  Diff   │────▶│  Plan   │────▶│  Component   │  │
│  └─────────┘     └─────────┘     └─────────┘     └──────┬───────┘  │
│                                                         │          │
│  ┌─────────┐     ┌─────────────────────────────────────▼────────┐  │
│  │ Target  │     │                                              │  │
│  │ Schema  │────▶│           Standalone Executable              │  │
│  └─────────┘     │  ┌────────────────────────────────────────┐  │  │
│                  │  │  • Embedded Wasmtime runtime           │  │  │
│                  │  │  • Embedded Wasm component             │  │  │
│                  │  │  • PostgreSQL client (tokio-postgres)  │  │  │
│                  │  │  • CLI argument parser                 │  │  │
│                  │  └────────────────────────────────────────┘  │  │
│                  └──────────────────────────────────────────────┘  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Standalone Executable Structure

```
┌──────────────────────────────────────────────────────────────────┐
│                    Standalone Migration Executable                │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │  CLI Layer                                                 │  │
│  │  • Parse arguments (--database-url, --dry-run, etc.)       │  │
│  │  • Display metadata and warnings                           │  │
│  │  • Report execution progress                               │  │
│  └─────────────────────────┬──────────────────────────────────┘  │
│                            │                                     │
│  ┌─────────────────────────▼──────────────────────────────────┐  │
│  │  Wasmtime Runtime (embedded)                               │  │
│  │  • Component instantiation                                 │  │
│  │  • Host function dispatch                                  │  │
│  └─────────────────────────┬──────────────────────────────────┘  │
│                            │                                     │
│  ┌─────────────────────────▼──────────────────────────────────┐  │
│  │  Host Functions                                            │  │
│  │  • database::execute(sql) → u64                            │  │
│  │  • log::log(level, message)                                │  │
│  └─────────────────────────┬──────────────────────────────────┘  │
│                            │                                     │
│  ┌─────────────────────────▼──────────────────────────────────┐  │
│  │  Migration Component (embedded .wasm bytes)                │  │
│  │  • Metadata (id, description, warnings)                    │  │
│  │  • SQL statements                                          │  │
│  │  • Execution logic                                         │  │
│  └────────────────────────────────────────────────────────────┘  │
│                            │                                     │
│  ┌─────────────────────────▼──────────────────────────────────┐  │
│  │  PostgreSQL Client (tokio-postgres + rustls)               │  │
│  │  • Connection management                                   │  │
│  │  • Transaction handling                                    │  │
│  │  • TLS support                                             │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
```

---

## Migration State Backend

### Motivation

Rather than comparing two live databases to generate migrations (the original `--from` and `--to` approach), Tern uses a **state backend** to track migration history. This provides several advantages:

1. **Reproducibility**: The exact sequence of migrations can be replayed on any database
2. **Auditability**: Full history of schema changes is preserved
3. **Offline operation**: Migrations can be generated without connecting to a "source" database
4. **Team collaboration**: State files can be checked into version control

### Schema State Model

The state backend stores two key pieces of information:

1. **Current schema state**: A serialized `Namespace` representing the expected database schema after all migrations have been applied
2. **Migration history**: An ordered list of migrations that have been applied, each identified by a content-addressable hash

```
┌─────────────────────────────────────────────────────────────────────┐
│                         State Backend                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Schema State (Namespace)                                    │   │
│  │  • Serialized representation of expected database schema     │   │
│  │  • Tables, columns, constraints, indexes, enums, sequences   │   │
│  │  • Serves as the "from" state for migration generation       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Migration History                                           │   │
│  │  • Ordered list of applied migrations                        │   │
│  │  • Each migration identified by MigrationId (content hash)   │   │
│  │  • Contains operations and metadata                          │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Migration Identification

Each migration is uniquely identified by a **content-addressable hash** (`MigrationId`). This hash is computed from:

- The operations in the migration (serialized `Vec<Operation>`)
- The source schema state hash
- Optionally, a user-provided description

```rust
/// A content-addressable identifier for a migration.
///
/// The hash is computed from the migration's operations and metadata,
/// ensuring that identical migrations produce identical IDs regardless
/// of when or where they were generated.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MigrationId(pub [u8; 32]); // SHA-256 hash

impl MigrationId {
    /// Compute the migration ID from its content.
    pub fn from_content(
        operations: &[Operation],
        parent_state_hash: &StateHash,
        description: &str,
    ) -> Self {
        use sha2::{Sha256, Digest};

        let mut hasher = Sha256::new();

        // Hash the parent state
        hasher.update(parent_state_hash.as_bytes());

        // Hash the operations (using canonical serialization)
        let ops_json = serde_json::to_vec(operations)
            .expect("operations should be serializable");
        hasher.update(&ops_json);

        // Hash the description
        hasher.update(description.as_bytes());

        let result = hasher.finalize();
        Self(result.into())
    }

    /// Format as a short hex string (first 8 characters).
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }

    /// Format as full hex string.
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }
}

/// A hash of the schema state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StateHash(pub [u8; 32]);

impl StateHash {
    /// Compute the state hash from a namespace.
    pub fn from_namespace(namespace: &Namespace) -> Self {
        use sha2::{Sha256, Digest};

        let mut hasher = Sha256::new();
        let json = serde_json::to_vec(namespace)
            .expect("namespace should be serializable");
        hasher.update(&json);

        Self(hasher.finalize().into())
    }
}
```

### Migration Record

A recorded migration contains all information needed to understand and replay the migration:

```rust
/// A recorded migration in the history.
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
    pub checkpoint_state: Option<Namespace>,
}
```

### State Backend Trait

The state backend is abstracted behind a trait, allowing different storage implementations:

```rust
/// Trait for migration state storage backends.
#[async_trait]
pub trait StateBackend: Send + Sync {
    /// Get the current schema state.
    async fn get_current_state(&self) -> Result<Namespace, StateError>;

    /// Get the state hash of the current schema.
    async fn get_current_state_hash(&self) -> Result<StateHash, StateError>;

    /// Get the full migration history.
    async fn get_history(&self) -> Result<Vec<Migration>, StateError>;

    /// Get a specific migration by ID.
    async fn get_migration(&self, id: &MigrationId) -> Result<Option<Migration>, StateError>;

    /// Record a new migration and update the current state.
    async fn record_migration(
        &self,
        migration: &Migration,
        new_state: &Namespace,
    ) -> Result<(), StateError>;

    /// Check if a migration has been applied.
    async fn is_applied(&self, id: &MigrationId) -> Result<bool, StateError>;

    /// Get the schema state at a specific point in history.
    async fn get_state_at(&self, migration_id: &MigrationId) -> Result<Namespace, StateError>;

    /// Initialize the backend with an initial schema state.
    async fn initialize(&self, initial_state: &Namespace) -> Result<(), StateError>;
}
```

### Backend Implementations

#### Local File Backend

The local file backend stores state in the filesystem, suitable for projects that want to check migration state into version control:

```
.tern/
├── state.json              # Current schema state (Namespace)
├── state.hash              # Current state hash (for quick comparison)
└── migrations/
    ├── index.json          # Ordered list of migration IDs
    ├── 1a2b3c4d.json       # Individual migration files
    ├── 5e6f7g8h.json
    └── ...
```

```rust
/// Local filesystem state backend.
pub struct LocalFileBackend {
    /// Root directory for state files.
    root: PathBuf,
}

impl LocalFileBackend {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = project_root.as_ref().join(".tern");
        if !root.exists() {
            return Err(StateError::NotInitialized);
        }
        Ok(Self { root })
    }

    pub fn init(project_root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = project_root.as_ref().join(".tern");
        std::fs::create_dir_all(&root)?;
        std::fs::create_dir_all(root.join("migrations"))?;

        // Initialize empty index
        let index = MigrationIndex { migrations: vec![] };
        let index_path = root.join("migrations/index.json");
        std::fs::write(&index_path, serde_json::to_string_pretty(&index)?)?;

        Ok(Self { root })
    }
}

#[async_trait]
impl StateBackend for LocalFileBackend {
    async fn get_current_state(&self) -> Result<Namespace, StateError> {
        let path = self.root.join("state.json");
        if !path.exists() {
            return Err(StateError::NotInitialized);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        Ok(serde_json::from_str(&content)?)
    }

    async fn record_migration(
        &self,
        migration: &Migration,
        new_state: &Namespace,
    ) -> Result<(), StateError> {
        // Write migration file
        let migration_path = self.root
            .join("migrations")
            .join(format!("{}.json", migration.id.short()));
        tokio::fs::write(
            &migration_path,
            serde_json::to_string_pretty(migration)?,
        ).await?;

        // Update index
        let index_path = self.root.join("migrations/index.json");
        let mut index: MigrationIndex = serde_json::from_str(
            &tokio::fs::read_to_string(&index_path).await?
        )?;
        index.migrations.push(migration.id.clone());
        tokio::fs::write(&index_path, serde_json::to_string_pretty(&index)?).await?;

        // Update current state
        let state_path = self.root.join("state.json");
        tokio::fs::write(&state_path, serde_json::to_string_pretty(new_state)?).await?;

        // Update state hash
        let hash = StateHash::from_namespace(new_state);
        let hash_path = self.root.join("state.hash");
        tokio::fs::write(&hash_path, hash.to_hex()).await?;

        Ok(())
    }

    // ... other methods
}

/// Index of migrations in order of application.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MigrationIndex {
    migrations: Vec<MigrationId>,
}
```

#### Remote Backend (Future)

A remote backend could store state in a database or cloud service:

```rust
/// PostgreSQL-based state backend.
///
/// Stores migration state in a `_tern_migrations` schema in the target database.
pub struct PostgresBackend {
    client: tokio_postgres::Client,
}

/// S3-based state backend for distributed teams.
pub struct S3Backend {
    bucket: String,
    prefix: String,
    client: aws_sdk_s3::Client,
}
```

### Migration Generation Workflow

With the state backend, the migration generation workflow changes:

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Migration Generation Workflow                     │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. Load "from" state from backend                                  │
│     ┌─────────────┐                                                 │
│     │   State     │ ──── get_current_state() ───▶ Namespace (from)  │
│     │   Backend   │                                                 │
│     └─────────────┘                                                 │
│                                                                     │
│  2. Load "to" state from live database                              │
│     ┌─────────────┐                                                 │
│     │   Live DB   │ ──── introspect schema ───▶ Namespace (to)      │
│     └─────────────┘                                                 │
│                                                                     │
│  3. Generate diff                                                   │
│     Namespace (from) ─┬─▶ diff_namespaces() ───▶ NamespaceDiff      │
│     Namespace (to)  ──┘                                             │
│                                                                     │
│  4. Create migration                                                │
│     NamespaceDiff ───▶ MigrationPlan ───▶ Migration                 │
│                                                                     │
│  5. Compile to executable                                           │
│     Migration ───▶ Wasm Component ───▶ Standalone Executable        │
│                                                                     │
│  6. Record migration (after successful application)                 │
│     ┌─────────────┐                                                 │
│     │   State     │ ◀── record_migration() ───── Migration          │
│     │   Backend   │                                                 │
│     └─────────────┘                                                 │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### State Reconstruction

The state backend supports reconstructing the schema state at any point in history:

```rust
impl LocalFileBackend {
    /// Reconstruct schema state at a specific migration.
    ///
    /// This walks the migration history, starting from the nearest
    /// checkpoint, and applies operations to rebuild the state.
    pub async fn get_state_at(&self, target_id: &MigrationId) -> Result<Namespace, StateError> {
        let history = self.get_history().await?;

        // Find the target migration
        let target_idx = history.iter()
            .position(|m| &m.id == target_id)
            .ok_or(StateError::MigrationNotFound(target_id.clone()))?;

        // Find the nearest checkpoint before the target
        let (start_state, start_idx) = self.find_nearest_checkpoint(&history, target_idx).await?;

        // Apply operations from checkpoint to target
        let mut state = start_state;
        let mut oid_gen = OidGenerator::new(state.highest_oid() + 1);

        for migration in &history[start_idx..=target_idx] {
            for op in &migration.operations {
                state.apply(op, &mut oid_gen)?;
            }
        }

        Ok(state)
    }

    async fn find_nearest_checkpoint(
        &self,
        history: &[Migration],
        before_idx: usize,
    ) -> Result<(Namespace, usize), StateError> {
        // Walk backwards to find a checkpoint
        for (i, migration) in history[..=before_idx].iter().enumerate().rev() {
            if let Some(ref checkpoint) = migration.checkpoint_state {
                return Ok((checkpoint.clone(), i + 1));
            }
        }

        // No checkpoint found, start from empty state
        Ok((Namespace::empty("public"), 0))
    }
}
```

### Initialization and Baseline

When adopting Tern on an existing database, the current schema becomes the initial state:

```rust
/// Initialize state backend from an existing database.
pub async fn init_from_database(
    backend: &impl StateBackend,
    database_url: &str,
    schema_name: &str,
) -> Result<(), StateError> {
    // Introspect current database schema
    let (client, connection) = tokio_postgres::connect(database_url, NoTls).await?;
    tokio::spawn(connection);

    let catalog = PostgresCatalog::new(&client);
    let namespace = load_namespace(&catalog, schema_name).await?;

    // Initialize backend with current state as baseline
    backend.initialize(&namespace).await?;

    // Create a baseline migration record
    let baseline = Migration {
        id: MigrationId::from_content(&[], &StateHash::zero(), "baseline"),
        description: "Baseline migration from existing database".to_string(),
        created_at: Utc::now(),
        operations: vec![],
        parent_state_hash: StateHash::zero(),
        resulting_state_hash: StateHash::from_namespace(&namespace),
        breaking_changes: vec![],
        checkpoint_state: Some(namespace),
    };

    backend.record_migration(&baseline, &namespace).await?;

    Ok(())
}
```

---

## WIT Interface Definition

The WebAssembly Interface Types (WIT) definition specifies the contract between the host (runtime) and guest (migration component).

```wit
// tern-migration.wit
package tern:migration@0.1.0;

/// Database access provided by the runtime
interface database {
    /// Error from a database operation
    record db-error {
        message: string,
    }

    /// Execute a SQL statement, returns rows affected
    execute: func(sql: string) -> result<u64, db-error>;
}

/// Logging provided by the runtime
interface log {
    /// Log levels
    enum level {
        info,
        warn,
        error,
    }

    /// Emit a log message
    log: func(level: level, message: string);
}

/// The migration interface
interface migration {
    /// Migration metadata
    record metadata {
        /// Unique identifier (e.g., "20240115_120000_add_status_column")
        id: string,
        /// Human-readable description
        description: string,
        /// Warnings (e.g., breaking changes)
        warnings: list<string>,
        /// Number of SQL statements
        statement-count: u32,
    }

    /// Get migration metadata (cheap, no execution)
    describe: func() -> metadata;

    /// Execute the migration, returns error message on failure
    run: func() -> result<_, string>;
}

/// World for a Tern migration component
world tern-migration {
    import database;
    import log;
    export migration;
}
```

---

## Project Structure

```
tern/
├── Cargo.toml                        # Workspace root
├── src/                              # Main tern crate (existing)
│   └── db/
│       ├── compile/                  # NEW: Migration compilation
│       │   ├── mod.rs
│       │   ├── codegen.rs            # Wasm component generation
│       │   ├── executable.rs         # Standalone executable builder
│       │   └── error.rs
│       ├── state/                    # NEW: State backend
│       │   ├── mod.rs                # StateBackend trait, re-exports
│       │   ├── types.rs              # MigrationId, StateHash, Migration
│       │   ├── local.rs              # LocalFileBackend implementation
│       │   ├── postgres.rs           # PostgresBackend (future)
│       │   └── error.rs              # StateError types
│       └── ...
├── crates/
│   ├── tern-migration-wit/           # NEW: WIT definitions
│   │   ├── Cargo.toml
│   │   └── wit/
│   │       └── migration.wit
│   ├── tern-migration-guest/         # NEW: Guest-side bindings
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   └── lib.rs
│   │   └── wit/                      # Symlink to tern-migration-wit/wit
│   └── tern-migration-runner/        # NEW: Standalone runner template
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs               # CLI entry point
│       │   ├── host.rs               # Host function implementations
│       │   └── runtime.rs            # Wasmtime setup
│       └── build.rs                  # Embeds component at compile time
└── tests/
    ├── migration_compile_tests.rs
    └── state_backend_tests.rs        # NEW: State backend tests
```

---

## Implementation Plan

### Phase 1: WIT Definition & Crate Setup

**Goal**: Establish the interface contract and create supporting crates.

#### Task 1.1: Create WIT definitions crate

Create `crates/tern-migration-wit/` with the WIT interface.

**Files to create:**
- `crates/tern-migration-wit/Cargo.toml`
- `crates/tern-migration-wit/wit/migration.wit`

```toml
# crates/tern-migration-wit/Cargo.toml
[package]
name = "tern-migration-wit"
version = "0.1.0"
edition = "2024"
description = "WIT definitions for Tern migration components"

# This crate just holds the WIT files; no Rust code needed
```

```wit
// crates/tern-migration-wit/wit/migration.wit
package tern:migration@0.1.0;

interface database {
    record db-error {
        message: string,
    }

    execute: func(sql: string) -> result<u64, db-error>;
}

interface log {
    enum level {
        info,
        warn,
        error,
    }

    log: func(level: level, message: string);
}

interface migration {
    record metadata {
        id: string,
        description: string,
        warnings: list<string>,
        statement-count: u32,
    }

    describe: func() -> metadata;
    run: func() -> result<_, string>;
}

world tern-migration {
    import database;
    import log;
    export migration;
}
```

#### Task 1.2: Create guest bindings crate

Create `crates/tern-migration-guest/` for generating migration components.

**Files to create:**
- `crates/tern-migration-guest/Cargo.toml`
- `crates/tern-migration-guest/src/lib.rs`

```toml
# crates/tern-migration-guest/Cargo.toml
[package]
name = "tern-migration-guest"
version = "0.1.0"
edition = "2024"
description = "Guest-side bindings for Tern migration components"

[dependencies]
wit-bindgen = "0.36"

[lib]
crate-type = ["cdylib"]
```

```rust
// crates/tern-migration-guest/src/lib.rs

wit_bindgen::generate!({
    world: "tern-migration",
    path: "../tern-migration-wit/wit",
});

pub use exports::tern::migration::migration::{Guest, Metadata};
pub use tern::migration::database;
pub use tern::migration::log::{self, Level};

/// Macro to define a migration with embedded SQL
#[macro_export]
macro_rules! define_migration {
    (
        id: $id:expr,
        description: $desc:expr,
        warnings: [$($warning:expr),* $(,)?],
        statements: [$($sql:expr),* $(,)?]
    ) => {
        struct GeneratedMigration;

        impl $crate::Guest for GeneratedMigration {
            fn describe() -> $crate::Metadata {
                $crate::Metadata {
                    id: $id.to_string(),
                    description: $desc.to_string(),
                    warnings: vec![$($warning.to_string()),*],
                    statement_count: [$($sql),*].len() as u32,
                }
            }

            fn run() -> Result<(), String> {
                let statements: &[&str] = &[$($sql),*];
                let total = statements.len();

                for (i, sql) in statements.iter().enumerate() {
                    $crate::log::log(
                        $crate::Level::Info,
                        &format!("[{}/{}] Executing statement", i + 1, total),
                    );

                    $crate::database::execute(sql)
                        .map_err(|e| format!("Statement {} failed: {}", i + 1, e.message))?;
                }

                Ok(())
            }
        }

        export!(GeneratedMigration);
    };
}
```

#### Task 1.3: Update workspace Cargo.toml

```toml
# Cargo.toml (workspace root)
[workspace]
members = [
    ".",
    "crates/tern-migration-wit",
    "crates/tern-migration-guest",
    "crates/tern-migration-runner",
]
```

---

### Phase 2: Migration Component Generation

**Goal**: Generate Wasm components from `MigrationPlan`.

#### Task 2.1: Add compilation module to main crate

**Files to create:**
- `src/db/compile/mod.rs`
- `src/db/compile/error.rs`
- `src/db/compile/codegen.rs`

```rust
// src/db/compile/mod.rs

mod codegen;
mod error;

pub use codegen::MigrationCompiler;
pub use error::CompileError;
```

```rust
// src/db/compile/error.rs

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum CompileError {
    #[error("Failed to generate migration source: {0}")]
    CodegenFailed(String),

    #[error("Failed to compile migration component: {0}")]
    CompilationFailed(String),

    #[error("cargo-component not found; install with: cargo install cargo-component")]
    CargoComponentNotFound,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
```

#### Task 2.2: Implement component code generation

```rust
// src/db/compile/codegen.rs

use crate::db::diff::BreakingChangeAnalysis;
use crate::db::migrate::{MigrationPlan, MigrationScript, PostgresRenderer};
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use super::CompileError;

/// Compiles migration plans into Wasm components
pub struct MigrationCompiler {
    /// Path to cargo-component (if not in PATH)
    cargo_component_path: Option<PathBuf>,
}

impl MigrationCompiler {
    pub fn new() -> Self {
        Self {
            cargo_component_path: None,
        }
    }

    /// Compile a migration plan into a Wasm component
    pub fn compile(
        &self,
        id: &str,
        description: &str,
        plan: &MigrationPlan,
        breaking_changes: &BreakingChangeAnalysis,
    ) -> Result<Vec<u8>, CompileError> {
        // 1. Render SQL statements
        let script = plan.render(&PostgresRenderer::default())
            .map_err(|e| CompileError::CodegenFailed(e.to_string()))?;

        let statements: Vec<String> = script.all_statements().collect();

        // 2. Collect warnings from breaking changes
        let warnings: Vec<String> = breaking_changes
            .iter()
            .map(|bc| bc.description.clone())
            .collect();

        // 3. Generate Rust source
        let source = self.generate_source(id, description, &statements, &warnings);

        // 4. Compile to Wasm component
        self.compile_source(&source)
    }

    fn generate_source(
        &self,
        id: &str,
        description: &str,
        statements: &[String],
        warnings: &[String],
    ) -> String {
        let statements_array = statements
            .iter()
            .map(|s| format!("r#\"{}\"#", s.replace("\"#", "\"##")))
            .collect::<Vec<_>>()
            .join(",\n        ");

        let warnings_array = warnings
            .iter()
            .map(|w| format!("\"{}\"", w.escape_default()))
            .collect::<Vec<_>>()
            .join(",\n        ");

        format!(
            r#"
use tern_migration_guest::define_migration;

define_migration! {{
    id: "{id}",
    description: "{description}",
    warnings: [
        {warnings_array}
    ],
    statements: [
        {statements_array}
    ]
}}
"#,
            id = id.escape_default(),
            description = description.escape_default(),
        )
    }

    fn compile_source(&self, source: &str) -> Result<Vec<u8>, CompileError> {
        // Create temporary project directory
        let temp_dir = TempDir::new()?;
        let project_dir = temp_dir.path();

        // Write Cargo.toml
        let cargo_toml = r#"
[package]
name = "migration"
version = "0.1.0"
edition = "2024"

[dependencies]
tern-migration-guest = { path = "GUEST_CRATE_PATH" }

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "tern:migration"
"#;
        // Note: In real implementation, GUEST_CRATE_PATH needs to be resolved
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

        // Write source
        std::fs::create_dir_all(project_dir.join("src"))?;
        std::fs::write(project_dir.join("src/lib.rs"), source)?;

        // Run cargo-component build
        let cargo_component = self
            .cargo_component_path
            .as_deref()
            .unwrap_or(Path::new("cargo-component"));

        let output = Command::new(cargo_component)
            .args(["build", "--release"])
            .current_dir(project_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CompileError::CompilationFailed(stderr.to_string()));
        }

        // Read the compiled component
        let wasm_path = project_dir
            .join("target/wasm32-wasip1/release/migration.wasm");

        std::fs::read(&wasm_path).map_err(CompileError::Io)
    }
}
```

#### Task 2.3: Add dependencies to main crate

```toml
# Cargo.toml additions
[dependencies]
tempfile = "3"
```

---

### Phase 3: Standalone Runner Implementation

**Goal**: Create the runner that becomes the standalone executable.

#### Task 3.1: Create runner crate

**Files to create:**
- `crates/tern-migration-runner/Cargo.toml`
- `crates/tern-migration-runner/src/main.rs`
- `crates/tern-migration-runner/src/runtime.rs`
- `crates/tern-migration-runner/src/host.rs`
- `crates/tern-migration-runner/build.rs`

```toml
# crates/tern-migration-runner/Cargo.toml
[package]
name = "tern-migration-runner"
version = "0.1.0"
edition = "2024"
description = "Standalone runner for Tern migration components"

[dependencies]
# Wasm runtime
wasmtime = { version = "28", features = ["component-model"] }

# CLI
clap = { version = "4", features = ["derive", "env"] }

# Database
tokio = { version = "1", features = ["full"] }
tokio-postgres = { version = "0.7", features = ["with-chrono-0_4"] }
postgres-native-tls = "0.5"
native-tls = "0.2"

# Error handling
miette = { version = "7", features = ["fancy"] }
thiserror = "2"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Serialization (for --describe --format=json)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Async utilities
futures = "0.3"

[build-dependencies]
# For embedding the component at compile time
```

#### Task 3.2: Implement CLI entry point

```rust
// crates/tern-migration-runner/src/main.rs

mod host;
mod runtime;

use clap::Parser;
use miette::{IntoDiagnostic, Result};
use tracing_subscriber::EnvFilter;

/// A database migration executable generated by Tern
#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Run without executing (show what would be done)
    #[arg(long, default_value = "false")]
    dry_run: bool,

    /// Skip confirmation prompt for migrations with warnings
    #[arg(long, default_value = "false")]
    yes: bool,

    /// Show migration metadata and exit
    #[arg(long, default_value = "false")]
    describe: bool,

    /// Output format for --describe
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,
}

// The component bytes are embedded at compile time
const COMPONENT_BYTES: &[u8] = include_bytes!(env!("MIGRATION_COMPONENT_PATH"));

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // Create runtime and load component
    let runtime = runtime::MigrationRuntime::new(COMPONENT_BYTES)?;

    // Get metadata
    let metadata = runtime.describe()?;

    // Handle --describe
    if cli.describe {
        print_metadata(&metadata, &cli.format);
        return Ok(());
    }

    // Display warnings and confirm
    if !metadata.warnings.is_empty() && !cli.yes {
        eprintln!("This migration has warnings:\n");
        for warning in &metadata.warnings {
            eprintln!("  - {}", warning);
        }
        eprintln!();

        if !cli.dry_run {
            eprint!("Proceed anyway? [y/N] ");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).into_diagnostic()?;
            if !input.trim().eq_ignore_ascii_case("y") {
                eprintln!("Aborted.");
                return Ok(());
            }
        }
    }

    // Connect to database
    eprintln!("Connecting to database...");
    let client = connect_database(&cli.database_url).await?;

    // Execute migration
    eprintln!(
        "Executing migration: {} ({} statements)",
        metadata.id, metadata.statement_count
    );

    if cli.dry_run {
        eprintln!("[DRY RUN] Would execute {} statements", metadata.statement_count);
        runtime.run_dry()?;
    } else {
        runtime.run(&client).await?;
    }

    eprintln!("Migration completed successfully");
    Ok(())
}

fn print_metadata(metadata: &runtime::Metadata, format: &str) {
    match format {
        "json" => {
            println!(
                "{}",
                serde_json::to_string_pretty(metadata).unwrap()
            );
        }
        _ => {
            println!("Migration: {}", metadata.id);
            println!("Description: {}", metadata.description);
            println!("Statements: {}", metadata.statement_count);
            if !metadata.warnings.is_empty() {
                println!("Warnings:");
                for warning in &metadata.warnings {
                    println!("  - {}", warning);
                }
            }
        }
    }
}

async fn connect_database(url: &str) -> Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .into_diagnostic()?;

    // Spawn connection handler
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Database connection error: {}", e);
        }
    });

    Ok(client)
}
```

#### Task 3.3: Implement Wasmtime runtime wrapper

```rust
// crates/tern-migration-runner/src/runtime.rs

use miette::{miette, Result};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Client;
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, Store};

use crate::host::HostState;

/// Metadata about the migration
#[derive(Debug, Clone, serde::Serialize)]
pub struct Metadata {
    pub id: String,
    pub description: String,
    pub warnings: Vec<String>,
    pub statement_count: u32,
}

/// Runtime for executing migration components
pub struct MigrationRuntime {
    engine: Engine,
    component: Component,
}

impl MigrationRuntime {
    pub fn new(component_bytes: &[u8]) -> Result<Self> {
        let mut config = Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config)
            .map_err(|e| miette!("Failed to create Wasm engine: {}", e))?;

        let component = Component::new(&engine, component_bytes)
            .map_err(|e| miette!("Failed to load migration component: {}", e))?;

        Ok(Self { engine, component })
    }

    /// Get migration metadata without executing
    pub fn describe(&self) -> Result<Metadata> {
        let mut store = Store::new(&self.engine, HostState::new_dry_run());
        let instance = self.instantiate(&mut store)?;

        // Call describe export
        let describe = instance
            .get_typed_func::<(), (Metadata,)>(&mut store, "describe")
            .map_err(|e| miette!("Failed to get describe function: {}", e))?;

        let (metadata,) = describe
            .call(&mut store, ())
            .map_err(|e| miette!("Failed to call describe: {}", e))?;

        Ok(metadata)
    }

    /// Execute migration in dry-run mode (no database)
    pub fn run_dry(&self) -> Result<()> {
        let mut store = Store::new(&self.engine, HostState::new_dry_run());
        let instance = self.instantiate(&mut store)?;

        let run = instance
            .get_typed_func::<(), (Result<(), String>,)>(&mut store, "run")
            .map_err(|e| miette!("Failed to get run function: {}", e))?;

        let (result,) = run
            .call(&mut store, ())
            .map_err(|e| miette!("Migration execution failed: {}", e))?;

        result.map_err(|e| miette!("Migration failed: {}", e))
    }

    /// Execute migration against database
    pub async fn run(&self, client: &Client) -> Result<()> {
        let state = HostState::new_with_client(client);
        let mut store = Store::new(&self.engine, state);
        let instance = self.instantiate(&mut store)?;

        let run = instance
            .get_typed_func::<(), (Result<(), String>,)>(&mut store, "run")
            .map_err(|e| miette!("Failed to get run function: {}", e))?;

        let (result,) = run
            .call(&mut store, ())
            .map_err(|e| miette!("Migration execution failed: {}", e))?;

        result.map_err(|e| miette!("Migration failed: {}", e))
    }

    fn instantiate(
        &self,
        store: &mut Store<HostState>,
    ) -> Result<wasmtime::component::Instance> {
        let mut linker = Linker::new(&self.engine);

        // Add host function implementations
        crate::host::add_to_linker(&mut linker)
            .map_err(|e| miette!("Failed to link host functions: {}", e))?;

        linker
            .instantiate(&mut *store, &self.component)
            .map_err(|e| miette!("Failed to instantiate component: {}", e))
    }
}
```

#### Task 3.4: Implement host functions

```rust
// crates/tern-migration-runner/src/host.rs

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Client;
use wasmtime::component::Linker;

/// State available to host functions during execution
pub struct HostState {
    /// Database client (None for dry-run mode)
    client: Option<Arc<Mutex<Client>>>,
    /// Whether we're in dry-run mode
    dry_run: bool,
    /// Collected SQL statements (for dry-run inspection)
    collected_statements: Vec<String>,
}

impl HostState {
    pub fn new_dry_run() -> Self {
        Self {
            client: None,
            dry_run: true,
            collected_statements: Vec::new(),
        }
    }

    pub fn new_with_client(client: &Client) -> Self {
        // Note: In real implementation, need to handle ownership properly
        Self {
            client: Some(Arc::new(Mutex::new(client.clone()))),
            dry_run: false,
            collected_statements: Vec::new(),
        }
    }
}

/// Database error returned to guest
#[derive(Clone)]
pub struct DbError {
    pub message: String,
}

/// Log level from guest
#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Add host function implementations to the linker
pub fn add_to_linker(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    // database::execute
    linker.root().func_wrap(
        "tern:migration/database",
        "execute",
        |mut caller: wasmtime::StoreContextMut<'_, HostState>,
         sql: String|
         -> Result<u64, DbError> {
            let state = caller.data_mut();

            if state.dry_run {
                // In dry-run mode, just log the SQL
                eprintln!("  [DRY RUN] {}", truncate_sql(&sql, 80));
                state.collected_statements.push(sql);
                return Ok(0);
            }

            // Execute against real database
            let client = state.client.as_ref().ok_or_else(|| DbError {
                message: "No database connection".to_string(),
            })?;

            // Note: This is synchronous; real implementation needs async handling
            let result = futures::executor::block_on(async {
                let client = client.lock().await;
                client.execute(&sql, &[]).await
            });

            match result {
                Ok(rows) => Ok(rows),
                Err(e) => Err(DbError {
                    message: e.to_string(),
                }),
            }
        },
    )?;

    // log::log
    linker.root().func_wrap(
        "tern:migration/log",
        "log",
        |_caller: wasmtime::StoreContextMut<'_, HostState>,
         level: LogLevel,
         message: String| {
            match level {
                LogLevel::Info => tracing::info!("{}", message),
                LogLevel::Warn => tracing::warn!("{}", message),
                LogLevel::Error => tracing::error!("{}", message),
            }
        },
    )?;

    Ok(())
}

fn truncate_sql(sql: &str, max_len: usize) -> String {
    let sql = sql.replace('\n', " ");
    if sql.len() <= max_len {
        sql
    } else {
        format!("{}...", &sql[..max_len - 3])
    }
}
```

#### Task 3.5: Create build script for embedding component

```rust
// crates/tern-migration-runner/build.rs

fn main() {
    // The MIGRATION_COMPONENT_PATH env var is set by Tern during compilation
    println!("cargo:rerun-if-env-changed=MIGRATION_COMPONENT_PATH");

    if let Ok(path) = std::env::var("MIGRATION_COMPONENT_PATH") {
        println!("cargo:rerun-if-changed={}", path);
    }
}
```

---

### Phase 4: Executable Generation Pipeline

**Goal**: Combine component generation with runner compilation.

#### Task 4.1: Add executable builder to compile module

**Files to create/update:**
- `src/db/compile/mod.rs` (update)
- `src/db/compile/executable.rs` (new)

```rust
// src/db/compile/mod.rs (updated)

mod codegen;
mod error;
mod executable;

pub use codegen::MigrationCompiler;
pub use error::CompileError;
pub use executable::{ExecutableBuilder, Target};
```

```rust
// src/db/compile/executable.rs

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use super::CompileError;

/// Target platform for cross-compilation
#[derive(Debug, Clone, Copy)]
pub enum Target {
    /// Native (current platform)
    Native,
    /// x86_64 Linux (GNU libc)
    X86_64LinuxGnu,
    /// x86_64 Linux (musl, static)
    X86_64LinuxMusl,
    /// x86_64 macOS
    X86_64MacOS,
    /// aarch64 macOS (Apple Silicon)
    Aarch64MacOS,
    /// x86_64 Windows
    X86_64Windows,
}

impl Target {
    pub fn rust_target(&self) -> Option<&'static str> {
        match self {
            Target::Native => None,
            Target::X86_64LinuxGnu => Some("x86_64-unknown-linux-gnu"),
            Target::X86_64LinuxMusl => Some("x86_64-unknown-linux-musl"),
            Target::X86_64MacOS => Some("x86_64-apple-darwin"),
            Target::Aarch64MacOS => Some("aarch64-apple-darwin"),
            Target::X86_64Windows => Some("x86_64-pc-windows-msvc"),
        }
    }

    pub fn binary_extension(&self) -> &'static str {
        match self {
            Target::X86_64Windows => ".exe",
            _ => "",
        }
    }
}

/// Builds standalone migration executables
pub struct ExecutableBuilder {
    /// Path to the runner crate
    runner_crate_path: PathBuf,
}

impl ExecutableBuilder {
    pub fn new() -> Self {
        // In a real implementation, this would find the installed runner crate
        Self {
            runner_crate_path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("crates/tern-migration-runner"),
        }
    }

    /// Build a standalone executable from a compiled Wasm component
    pub fn build(
        &self,
        component_bytes: &[u8],
        output_path: &Path,
        target: Target,
    ) -> Result<(), CompileError> {
        // Create temp directory for the component
        let temp_dir = TempDir::new()?;
        let component_path = temp_dir.path().join("migration.wasm");
        std::fs::write(&component_path, component_bytes)?;

        // Build the runner with the component embedded
        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--release")
            .current_dir(&self.runner_crate_path)
            .env("MIGRATION_COMPONENT_PATH", &component_path);

        // Add target if cross-compiling
        if let Some(rust_target) = target.rust_target() {
            cmd.arg("--target").arg(rust_target);
        }

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CompileError::CompilationFailed(stderr.to_string()));
        }

        // Copy the binary to the output path
        let binary_name = format!("tern-migration-runner{}", target.binary_extension());
        let binary_path = if let Some(rust_target) = target.rust_target() {
            self.runner_crate_path
                .join("target")
                .join(rust_target)
                .join("release")
                .join(&binary_name)
        } else {
            self.runner_crate_path
                .join("target/release")
                .join(&binary_name)
        };

        std::fs::copy(&binary_path, output_path)?;

        Ok(())
    }
}
```

#### Task 4.2: Create high-level compilation API

```rust
// src/db/compile/mod.rs (add high-level API)

use crate::db::diff::{diff_namespaces, BreakingChangeAnalysis};
use crate::db::migrate::MigrationPlan;
use crate::db::model::Namespace;
use std::path::Path;

/// Options for migration compilation
pub struct CompileOptions {
    /// Migration identifier
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Target platform
    pub target: Target,
}

/// Result of migration compilation
pub struct CompilationResult {
    /// Warnings (from breaking changes)
    pub warnings: Vec<String>,
    /// Number of SQL statements in the migration
    pub statement_count: usize,
}

/// Compile a migration from source to target schema
pub fn compile_migration(
    source: &Namespace,
    target: &Namespace,
    output_path: &Path,
    options: CompileOptions,
) -> Result<CompilationResult, CompileError> {
    // 1. Diff schemas
    let diff = diff_namespaces(source, target);

    // 2. Analyze breaking changes
    let breaking_changes = BreakingChangeAnalysis::from_diff(&diff);

    // 3. Generate migration plan
    let plan = MigrationPlan::from_diff(&diff)
        .map_err(|e| CompileError::CodegenFailed(e.to_string()))?;

    // 4. Compile to Wasm component
    let compiler = MigrationCompiler::new();
    let component_bytes = compiler.compile(
        &options.id,
        &options.description,
        &plan,
        &breaking_changes,
    )?;

    // 5. Build standalone executable
    let builder = ExecutableBuilder::new();
    builder.build(&component_bytes, output_path, options.target)?;

    Ok(CompilationResult {
        warnings: breaking_changes
            .iter()
            .map(|bc| bc.description.clone())
            .collect(),
        statement_count: plan.operations.len(),
    })
}
```

---

### Phase 5: CLI Integration

**Goal**: Add CLI commands for compiling and inspecting migrations.

#### Task 5.1: Add CLI commands

Update `src/cli/mod.rs` to add new commands:

```rust
// src/cli/mod.rs (additions)

#[derive(Debug, Parser)]
pub enum CliCommand {
    // ... existing commands ...

    /// Initialize a new Tern project with state backend
    #[command(name = "init")]
    Init {
        /// Initialize from an existing database (creates baseline)
        #[arg(long, env = "DATABASE_URL")]
        from: Option<String>,

        /// Schema name to introspect
        #[arg(long, default_value = "public")]
        schema: String,

        /// State backend type
        #[arg(long, default_value = "local")]
        backend: StateBackendType,
    },

    /// Show state backend status
    #[command(name = "status")]
    Status,

    /// Compile a migration to a standalone executable
    #[command(name = "compile")]
    Compile {
        /// Database URL to compare against state backend
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,

        /// Output path for the executable
        #[arg(short, long)]
        output: PathBuf,

        /// Migration description
        #[arg(long)]
        description: String,

        /// Target platform
        #[arg(long, default_value = "native")]
        target: String,

        /// Record the migration to state backend after compilation
        #[arg(long, default_value = "false")]
        record: bool,

        /// Show what would be generated without writing files
        #[arg(long, default_value = "false")]
        dry_run: bool,

        /// Bypass state backend and compare two databases directly (legacy mode)
        #[arg(long, default_value = "false")]
        no_state: bool,

        /// Source database URL (only with --no-state)
        #[arg(long, requires = "no_state")]
        from: Option<String>,
    },

    /// List migration history
    #[command(name = "history")]
    History {
        /// Output format
        #[arg(long, default_value = "text")]
        format: String,

        /// Number of migrations to show
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Show details of a specific migration
    #[command(name = "show")]
    Show {
        /// Migration ID (short or full hash)
        migration_id: String,

        /// Output format (text, json, sql)
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Record a migration as applied
    #[command(name = "record")]
    Record {
        /// Migration ID to record
        migration_id: Option<String>,

        /// Path to migration executable file
        #[arg(long)]
        migration_file: Option<PathBuf>,
    },

    /// Inspect a compiled migration (extract metadata/SQL)
    #[command(name = "inspect")]
    Inspect {
        /// Path to the migration executable
        path: PathBuf,

        /// Output format (text, json, sql)
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Verify state backend matches database
    #[command(name = "verify")]
    Verify {
        /// Database URL to verify against
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },

    /// Export current schema state
    #[command(name = "export-state")]
    ExportState {
        /// Output file path
        #[arg(short, long)]
        output: PathBuf,

        /// Output format
        #[arg(long, default_value = "json")]
        format: String,
    },
}

/// State backend types.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StateBackendType {
    /// Local filesystem backend (.tern/ directory)
    Local,
    /// PostgreSQL backend (stores in _tern_migrations schema)
    Postgres,
}
```

#### Task 5.2: Implement command handlers

**Files to create:**
- `src/cli/commands/init.rs`
- `src/cli/commands/compile.rs`
- `src/cli/commands/history.rs`
- `src/cli/commands/inspect.rs`
- `src/cli/commands/verify.rs`

```rust
// src/cli/commands/init.rs

use crate::db::state::{LocalFileBackend, StateBackend, init_from_database};
use crate::db::model::Namespace;
use miette::Result;
use std::path::Path;

pub async fn run_init(
    from: Option<&str>,
    schema: &str,
    backend_type: StateBackendType,
) -> Result<()> {
    let project_root = std::env::current_dir()?;

    // Check if already initialized
    if project_root.join(".tern").exists() {
        return Err(miette::miette!(
            "Tern is already initialized in this directory. \
             Use --force to reinitialize."
        ));
    }

    eprintln!("Initializing Tern state backend...");

    match backend_type {
        StateBackendType::Local => {
            let backend = LocalFileBackend::init(&project_root)?;

            if let Some(database_url) = from {
                eprintln!("Creating baseline from database {}...", database_url);
                init_from_database(&backend, database_url, schema).await?;
                eprintln!("Baseline migration created.");
            } else {
                // Initialize with empty state
                let empty = Namespace::empty(schema);
                backend.initialize(&empty).await?;
                eprintln!("Initialized with empty schema state.");
            }
        }
        StateBackendType::Postgres => {
            // Future: implement PostgreSQL backend initialization
            return Err(miette::miette!("PostgreSQL backend not yet implemented"));
        }
    }

    eprintln!("Tern initialized successfully.");
    eprintln!("State directory: .tern/");

    Ok(())
}
```

```rust
// src/cli/commands/compile.rs

use crate::db::compile::{compile_migration, CompileOptions, Target};
use crate::db::query::{load_namespace, PostgresCatalog};
use crate::db::state::{LocalFileBackend, StateBackend, Migration, MigrationId, StateHash};
use crate::db::diff::diff_namespaces;
use crate::db::migrate::MigrationPlan;
use chrono::Utc;
use miette::Result;
use std::path::PathBuf;

pub async fn run_compile(
    database_url: &str,
    output: &PathBuf,
    description: &str,
    target_str: &str,
    record: bool,
    dry_run: bool,
    no_state: bool,
    from: Option<&str>,
) -> Result<()> {
    // Parse target platform
    let target = parse_target(target_str)?;

    // Load schemas based on mode
    let (source, target_schema) = if no_state {
        // Legacy mode: compare two databases directly
        let from_url = from.ok_or_else(|| {
            miette::miette!("--from is required when using --no-state")
        })?;
        eprintln!("Loading source schema from {}...", from_url);
        let source = load_schema_from_url(from_url, "public").await?;

        eprintln!("Loading target schema from {}...", database_url);
        let target = load_schema_from_url(database_url, "public").await?;

        (source, target)
    } else {
        // State backend mode: load from state, compare to database
        let project_root = std::env::current_dir()?;
        let backend = LocalFileBackend::open(&project_root)?;

        eprintln!("Loading source schema from state backend...");
        let source = backend.get_current_state().await?;
        let source_hash = backend.get_current_state_hash().await?;

        eprintln!("Loading target schema from {}...", database_url);
        let target = load_schema_from_url(database_url, source.name.as_ref()).await?;

        (source, target)
    };

    // Generate diff
    eprintln!("Computing schema diff...");
    let diff = diff_namespaces(&source, &target_schema);

    if diff.is_empty() {
        eprintln!("No schema changes detected.");
        return Ok(());
    }

    // Create migration plan
    let plan = MigrationPlan::from_diff(&diff);
    eprintln!("Generated {} operations", plan.len());

    if dry_run {
        // Just show what would be generated
        eprintln!("\nDry run - would generate migration with:");
        for (i, op) in plan.operations.iter().enumerate() {
            eprintln!("  {}. {}", i + 1, op.description());
        }
        return Ok(());
    }

    // Create migration record
    let parent_hash = if no_state {
        StateHash::from_namespace(&source)
    } else {
        let backend = LocalFileBackend::open(&std::env::current_dir()?)?;
        backend.get_current_state_hash().await?
    };

    let migration_id = MigrationId::from_content(
        &plan.operations,
        &parent_hash,
        description,
    );

    eprintln!("Migration ID: {}", migration_id.short());

    // Compile to executable
    eprintln!("Compiling migration...");
    let result = compile_migration(
        &source,
        &target_schema,
        output,
        CompileOptions {
            id: migration_id.short(),
            description: description.to_string(),
            target,
        },
    )?;

    eprintln!("Compiled migration to {}", output.display());
    eprintln!("  Statements: {}", result.statement_count);

    if !result.warnings.is_empty() {
        eprintln!("  Breaking changes:");
        for warning in &result.warnings {
            eprintln!("    - {}", warning);
        }
    }

    // Optionally record the migration
    if record && !no_state {
        let backend = LocalFileBackend::open(&std::env::current_dir()?)?;

        let migration = Migration {
            id: migration_id,
            description: description.to_string(),
            created_at: Utc::now(),
            operations: plan.operations,
            parent_state_hash: parent_hash,
            resulting_state_hash: StateHash::from_namespace(&target_schema),
            breaking_changes: result.warnings.iter()
                .map(|w| BreakingChange { description: w.clone() })
                .collect(),
            checkpoint_state: None,
        };

        backend.record_migration(&migration, &target_schema).await?;
        eprintln!("Migration recorded to state backend.");
    }

    Ok(())
}

fn parse_target(s: &str) -> Result<Target> {
    match s {
        "native" => Ok(Target::Native),
        "x86_64-linux" | "x86_64-linux-gnu" => Ok(Target::X86_64LinuxGnu),
        "x86_64-linux-musl" => Ok(Target::X86_64LinuxMusl),
        "x86_64-macos" => Ok(Target::X86_64MacOS),
        "aarch64-macos" => Ok(Target::Aarch64MacOS),
        "x86_64-windows" => Ok(Target::X86_64Windows),
        _ => Err(miette::miette!("Unknown target: {}", s)),
    }
}

async fn load_schema_from_url(url: &str, schema: &str) -> Result<Namespace> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .into_diagnostic()?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Connection error: {}", e);
        }
    });

    let catalog = PostgresCatalog::new(&client);
    load_namespace(&catalog, schema).await.into_diagnostic()
}
```

```rust
// src/cli/commands/verify.rs

use crate::db::query::{load_namespace, PostgresCatalog};
use crate::db::state::{LocalFileBackend, StateBackend, StateHash};
use miette::Result;

pub async fn run_verify(database_url: &str) -> Result<()> {
    let project_root = std::env::current_dir()?;
    let backend = LocalFileBackend::open(&project_root)?;

    // Get state from backend
    let state = backend.get_current_state().await?;
    let state_hash = backend.get_current_state_hash().await?;

    // Load schema from database
    eprintln!("Connecting to database...");
    let (client, connection) = tokio_postgres::connect(database_url, tokio_postgres::NoTls)
        .await
        .into_diagnostic()?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Connection error: {}", e);
        }
    });

    let catalog = PostgresCatalog::new(&client);
    let db_namespace = load_namespace(&catalog, state.name.as_ref())
        .await
        .into_diagnostic()?;

    let db_hash = StateHash::from_namespace(&db_namespace);

    // Compare
    eprintln!("State Hash:    {}", state_hash.to_hex());
    eprintln!("Database Hash: {}", db_hash.to_hex());

    if state_hash == db_hash {
        eprintln!("Status: ✓ In sync");
        Ok(())
    } else {
        eprintln!("Status: ✗ Out of sync");

        // Show differences
        let diff = diff_namespaces(&state, &db_namespace);
        if !diff.is_empty() {
            eprintln!("\nDifferences:");
            // ... output diff summary
        }

        Err(miette::miette!("State backend and database are out of sync"))
    }
}
```

---

### Phase 6: Testing

**Goal**: Comprehensive tests for the compilation pipeline.

#### Task 6.1: Unit tests for code generation

```rust
// src/db/compile/codegen.rs (tests)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::history::NamespaceBuilder;

    #[test]
    fn test_generate_source_simple() {
        let compiler = MigrationCompiler::new();

        let source = compiler.generate_source(
            "test_migration",
            "A test migration",
            &["CREATE TABLE foo (id INT)".to_string()],
            &[],
        );

        assert!(source.contains("test_migration"));
        assert!(source.contains("CREATE TABLE foo"));
    }

    #[test]
    fn test_generate_source_with_warnings() {
        let compiler = MigrationCompiler::new();

        let source = compiler.generate_source(
            "breaking_migration",
            "Migration with breaking changes",
            &["ALTER TABLE foo DROP COLUMN bar".to_string()],
            &["Column 'bar' will be dropped".to_string()],
        );

        assert!(source.contains("Column 'bar' will be dropped"));
    }

    #[test]
    fn test_generate_source_escapes_sql() {
        let compiler = MigrationCompiler::new();

        let source = compiler.generate_source(
            "test",
            "Test",
            &["SELECT 'hello \"world\"'".to_string()],
            &[],
        );

        // Should compile without syntax errors
        assert!(source.contains("SELECT"));
    }
}
```

#### Task 6.2: Integration tests

```rust
// tests/compile_integration_tests.rs

use tern::db::compile::{compile_migration, CompileOptions, Target};
use tern::db::history::NamespaceBuilder;
use tempfile::tempdir;

#[test]
fn test_compile_simple_migration() {
    let source = NamespaceBuilder::new("public")
        .table("users", |t| {
            t.column("id", "integer").not_null()
        })
        .build();

    let target = NamespaceBuilder::new("public")
        .table("users", |t| {
            t.column("id", "integer").not_null()
             .column("email", "text")
        })
        .build();

    let temp_dir = tempdir().unwrap();
    let output = temp_dir.path().join("migration");

    let result = compile_migration(
        &source,
        &target,
        &output,
        CompileOptions {
            id: "add_email_column".to_string(),
            description: "Add email column to users".to_string(),
            target: Target::Native,
        },
    ).unwrap();

    assert!(output.exists());
    assert_eq!(result.statement_count, 1);
    assert!(result.warnings.is_empty());
}

#[test]
fn test_compile_with_breaking_changes() {
    let source = NamespaceBuilder::new("public")
        .table("users", |t| {
            t.column("id", "integer").not_null()
             .column("name", "text") // nullable
        })
        .build();

    let target = NamespaceBuilder::new("public")
        .table("users", |t| {
            t.column("id", "integer").not_null()
             .column("name", "text").not_null() // now NOT NULL
        })
        .build();

    let temp_dir = tempdir().unwrap();
    let output = temp_dir.path().join("migration");

    let result = compile_migration(
        &source,
        &target,
        &output,
        CompileOptions {
            id: "make_name_required".to_string(),
            description: "Make name column required".to_string(),
            target: Target::Native,
        },
    ).unwrap();

    assert!(!result.warnings.is_empty());
    assert!(result.warnings[0].contains("NOT NULL"));
}
```

---

## CLI Usage

Once implemented, the CLI will support the following commands:

### State Backend Initialization

```bash
# Initialize a new project with empty state
tern init
# Creates .tern/ directory with empty state

# Initialize from an existing database (baseline)
tern init --from postgres://localhost/mydb --schema public
# Introspects database and creates baseline migration

# Check state backend status
tern status
# Output:
# State Backend: local (.tern/)
# Current State Hash: a1b2c3d4
# Migrations: 5 applied
# Last Migration: 20240115_add_preferences (2024-01-15)
```

### Migration Generation (State Backend)

```bash
# Generate a migration by comparing state backend to live database
tern compile \
  --database-url postgres://localhost/mydb \
  --output ./migrations/20240115_schema_update \
  --description "Add user preferences table"
# Uses state backend as "from", live database as "to"

# Generate and immediately record (for development workflows)
tern compile \
  --database-url postgres://localhost/mydb \
  --output ./migrations/20240115_schema_update \
  --description "Add user preferences table" \
  --record
# Also updates state backend with the new migration

# Compile for a specific platform
tern compile \
  --database-url postgres://localhost/mydb \
  --output ./dist/migration-linux \
  --target x86_64-linux-musl \
  --description "Add user preferences table"

# Show what would be generated without writing files
tern compile \
  --database-url postgres://localhost/mydb \
  --dry-run
# Outputs diff summary and SQL preview
```

### Migration History Management

```bash
# List migration history
tern history
# Output:
# ID        Description                     Applied At
# a1b2c3d4  Baseline migration              2024-01-01 10:00:00
# e5f6g7h8  Add users table                 2024-01-05 14:30:00
# i9j0k1l2  Add user preferences            2024-01-15 09:15:00

# Show details of a specific migration
tern show a1b2c3d4
# Output:
# Migration: a1b2c3d4
# Description: Add users table
# Created: 2024-01-05 14:30:00
# Parent State: 00000000
# Resulting State: b2c3d4e5
# Operations: 3
#   1. CreateTable public.users
#   2. CreateIndex public.users_email_idx
#   3. AddConstraint public.users_email_unique
# Breaking Changes: none

# Show SQL for a migration
tern show a1b2c3d4 --format sql

# Get state at a specific point in history
tern state-at a1b2c3d4 --output schema.json
```

### Inspecting Compiled Migrations

```bash
# Inspect a compiled migration
tern inspect ./migrations/20240115_schema_update
# Output:
# Migration: 20240115_schema_update
# Description: Add user preferences table
# ID: i9j0k1l2
# Statements: 5
# Warnings:
#   - Column 'users.email' will be made NOT NULL

# Extract SQL from compiled migration
tern inspect ./migrations/20240115_schema_update --format sql
# Output:
# -- Migration: 20240115_schema_update
# -- Description: Add user preferences table
# -- ID: i9j0k1l2
# BEGIN;
# ALTER TABLE users ADD COLUMN preferences JSONB;
# ...
# COMMIT;

# Output metadata as JSON
tern inspect ./migrations/20240115_schema_update --format json
```

### Running Compiled Migrations

```bash
# Run the compiled migration
./migrations/20240115_schema_update --database-url postgres://localhost/mydb

# Run with dry-run mode
./migrations/20240115_schema_update --database-url postgres://localhost/mydb --dry-run

# Skip warning confirmation
./migrations/20240115_schema_update --database-url postgres://localhost/mydb --yes

# Get migration info
./migrations/20240115_schema_update --describe
./migrations/20240115_schema_update --describe --format json

# Record the migration as applied (updates state backend)
tern record i9j0k1l2
# or
tern record --migration-file ./migrations/20240115_schema_update
```

### Advanced: Manual State Management

```bash
# Export current state to a file
tern export-state --output schema.json

# Import state from a file (careful: overwrites current state)
tern import-state --input schema.json --force

# Verify state matches database
tern verify --database-url postgres://localhost/mydb
# Output:
# State Hash: a1b2c3d4
# Database Hash: a1b2c3d4
# Status: ✓ In sync

# If out of sync:
# Output:
# State Hash: a1b2c3d4
# Database Hash: x9y8z7w6
# Status: ✗ Out of sync
# Differences:
#   - Table 'users': column 'email' type differs (text vs varchar(255))
```

### Legacy Mode (Direct Database Comparison)

For cases where state backend is not desired, direct database comparison is still supported:

```bash
# Compare two databases directly (bypasses state backend)
tern compile \
  --from postgres://localhost/mydb_dev \
  --to postgres://localhost/mydb_staging \
  --output ./migrations/20240115_schema_update \
  --description "Sync staging to dev" \
  --no-state
```

---

## Dependencies Summary

### Main Tern Crate

```toml
[dependencies]
# Existing dependencies...

# New dependencies for compilation
tempfile = "3"
chrono = { version = "0.4", features = ["serde"] }

# New dependencies for state backend
sha2 = "0.10"              # Content-addressable hashing
hex = "0.4"                # Hex encoding for hash display
async-trait = "0.1"        # For async StateBackend trait
```

### tern-migration-guest Crate

```toml
[dependencies]
wit-bindgen = "0.36"
```

### tern-migration-runner Crate

```toml
[dependencies]
wasmtime = { version = "28", features = ["component-model"] }
clap = { version = "4", features = ["derive", "env"] }
tokio = { version = "1", features = ["full"] }
tokio-postgres = { version = "0.7" }
postgres-native-tls = "0.5"
native-tls = "0.2"
miette = { version = "7", features = ["fancy"] }
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
futures = "0.3"
```

### Build Tools Required

- `cargo-component` - For compiling Wasm components
- Cross-compilation targets as needed (e.g., `rustup target add x86_64-unknown-linux-musl`)

---

## Task Checklist

| Phase | Task | Description |
|-------|------|-------------|
| **1** | 1.1 | Create `tern-migration-wit` crate with WIT definitions |
| | 1.2 | Create `tern-migration-guest` crate with guest bindings |
| | 1.3 | Update workspace `Cargo.toml` |
| **2** | 2.1 | Add `db::compile` module structure |
| | 2.2 | Implement `MigrationCompiler` with code generation |
| | 2.3 | Add tempfile dependency |
| **3** | 3.1 | Create `tern-migration-runner` crate with Cargo.toml |
| | 3.2 | Implement CLI entry point (`main.rs`) |
| | 3.3 | Implement Wasmtime runtime wrapper (`runtime.rs`) |
| | 3.4 | Implement host functions (`host.rs`) |
| | 3.5 | Create build script for component embedding |
| **4** | 4.1 | Implement `ExecutableBuilder` |
| | 4.2 | Create high-level `compile_migration` API |
| **5** | 5.1 | Add `db::state` module structure |
| | 5.2 | Implement `MigrationId` and `StateHash` types with SHA-256 hashing |
| | 5.3 | Implement `Migration` record type |
| | 5.4 | Define `StateBackend` trait |
| | 5.5 | Implement `LocalFileBackend` |
| | 5.6 | Add state initialization from existing database |
| | 5.7 | Add state reconstruction from history |
| **6** | 6.1 | Add `init`, `status`, `history`, `verify` CLI commands |
| | 6.2 | Update `compile` command to use state backend |
| | 6.3 | Add `record` command for recording applied migrations |
| | 6.4 | Implement command handlers |
| **7** | 7.1 | Unit tests for code generation |
| | 7.2 | Unit tests for state backend |
| | 7.3 | Integration tests for full pipeline |
| | 7.4 | Integration tests for state reconstruction |

---

## Future Extensions

This design intentionally leaves room for future enhancements:

1. **User code hooks**: Add optional exports (`before_run`, `after_run`, `on_error`) that user-provided components can implement

2. **Rollback execution**: Currently rollback SQL is generated but not executed; could add `--rollback` flag

3. **Multi-database support**: Abstract the `database` interface to support MySQL, SQLite, etc.

4. **Checkpoints**: For long-running migrations, add checkpoint/resume capability

5. **Progress reporting**: Add progress callbacks for migrations with many statements

6. **Remote execution**: Allow migrations to execute against remote databases through a secure tunnel

### State Backend Extensions

7. **PostgreSQL Backend**: Store migration state directly in the target database's `_tern_migrations` schema. This eliminates the need for local `.tern/` directory and works well for teams that don't want to check state files into version control.

8. **S3/Cloud Storage Backend**: Store state in cloud object storage for distributed teams. Supports locking to prevent concurrent migrations.

9. **State Branching**: Support multiple "branches" of migration state for feature branches that diverge from main. Includes branch merging with conflict detection.

10. **Migration Squashing**: Combine multiple migrations into a single "squashed" migration with a checkpoint. Useful for reducing history size while preserving the ability to recreate old states.

11. **Drift Detection**: Automatic detection of schema drift (manual changes made outside of Tern). Could trigger warnings or block migrations until resolved.

12. **Team Notifications**: Webhooks or integrations to notify team members when migrations are recorded or applied.

### Migration Record Extensions

13. **Execution Tracking**: Store when and where each migration was actually executed (not just recorded). Track execution time, affected rows, etc.

14. **Partial Migrations**: Support for migrations that failed partway through, with the ability to resume from the failure point.

15. **Migration Dependencies**: Explicit dependencies between migrations beyond just linear history. Useful for parallel development workflows.
