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

## Implementation Status

### Completed: Migration State Backend (Phase 1)

The core state backend infrastructure has been implemented in `src/db/state/`. This provides the foundation for tracking migration history and schema state.

#### Implemented Components

| Component | Location | Description |
|-----------|----------|-------------|
| `MigrationId` | `src/db/state/types.rs` | Content-addressable 256-bit hash using BLAKE3 |
| `StateHash` | `src/db/state/types.rs` | Schema state hash using BLAKE3 |
| `Migration` | `src/db/state/types.rs` | Full migration record with operations and metadata |
| `MigrationIndex` | `src/db/state/types.rs` | Ordered list of migration IDs |
| `StateBackend` trait | `src/db/state/mod.rs` | Abstract interface for storage backends |
| `LocalFileBackend` | `src/db/state/local.rs` | Filesystem implementation |
| `StateError` | `src/db/state/error.rs` | Error types with miette diagnostics |
| `InMemoryBackend` | `src/db/state/mod.rs` | Test double (in tests module) |
| `init_from_database()` | `src/db/state/init.rs` | Initialize backend from existing database |
| `init_empty()` | `src/db/state/init.rs` | Initialize backend with empty schema |
| `verify_state()` | `src/db/state/init.rs` | Verify backend matches database |

#### StateBackend Trait Methods

| Method | Description |
|--------|-------------|
| `initialize()` | Create directory structure |
| `is_initialized()` | Check if backend is set up |
| `get_migration_index()` | Get ordered list of migration IDs |
| `get_migration()` | Get specific migration by ID |
| `get_all_migrations()` | Get all migrations in order |
| `save_migration()` | Save a new migration |
| `get_current_state_hash()` | Get hash of current schema state |
| `get_current_state()` | Get full `Namespace` of current state |
| `save_current_state()` | Store current schema state |
| `record_migration()` | Atomically save migration and update state |
| `get_state_at()` | Reconstruct state at any migration point |
| `verify_chain()` | Verify migration chain integrity |
| `get_migrations_since()` | Get migrations since a given state hash |

#### Key Design Decisions Made

1. **BLAKE3 instead of XxHash3**: Uses cryptographic BLAKE3 for tamper-resistant content addressing. Provides 256-bit hashes (64 hex characters) with domain separation between migration IDs and state hashes.

2. **Sequential file naming**: Migration files are named `00001.json`, `00002.json`, etc. instead of using hash-based filenames. This allows easy browsing with `ls` while the actual hash is stored in the file content.

3. **Hex-encoded hashes in JSON**: Hashes are serialized as 64-character hex strings for human readability.

4. **Checkpoint-based state reconstruction**: `get_state_at()` finds the nearest checkpoint migration and applies subsequent operations to reconstruct state at any point in history.

#### Directory Structure

```
.tern/
├── state.json        # Current schema state (Namespace)
└── migrations/
    ├── index.json    # Ordered list of migration IDs
    ├── 00001.json    # First migration (baseline)
    ├── 00002.json    # Second migration
    └── ...
```

#### Hash Stability

Hash stability tests are in place to detect any accidental changes to the hashing algorithm. The following hashes are pinned:

```rust
// MigrationId for empty operations with zero parent:
"5ac039f690bd2a0f12892e4b9be6ca586bbfe63c1f612f2840485a90c2e5884f"

// StateHash for empty "public" namespace:
"3fa55ab02853d2d983c739392e9e19bc11c1292b56f20abeae00ee7dd0f477b3"
```

---

### Completed: WIT Definitions & Guest Bindings (Phase 2)

The WebAssembly interface contract and guest bindings have been implemented in `crates/tern-migration-wit/` and `crates/tern-migration-guest/`.

#### Implemented Components

| Component | Location | Description |
|-----------|----------|-------------|
| WIT definitions | `crates/tern-migration-wit/wit/migration.wit` | Interface contract for migration components |
| Guest bindings | `crates/tern-migration-guest/src/lib.rs` | wit-bindgen generated types and `define_migration!` macro |

#### WIT Interface Features

- `database` interface with `execute()` and `query()` functions
- `log` interface with severity levels (debug, info, warn, error)
- `migration` interface with `describe()`, `get-statements()`, and `run()` exports
- `breaking-change` record with `mitigation-strategy` enum (dual-write, backfill, ratchet, destructive)
- Rich `db-error` record with PostgreSQL error details (code, constraint name, table name)

---

### Completed: Migration Component Generation (Phase 3)

The compilation module has been implemented in `src/db/compile/` providing Rust source code generation from migration plans.

#### Implemented Components

| Component | Location | Description |
|-----------|----------|-------------|
| `MigrationCompiler` | `src/db/compile/codegen.rs` | Generates Rust source from migration operations |
| `CompilationResult` | `src/db/compile/codegen.rs` | Result containing source code and metadata |
| `CompiledStatement` | `src/db/compile/codegen.rs` | Statement with SQL, description, and sequence |
| `CompiledBreakingChange` | `src/db/compile/codegen.rs` | Breaking change with mitigation strategy |
| `CompileError` | `src/db/compile/error.rs` | Error types with miette diagnostics |

#### Key Features

- Generates Rust source code using `define_migration!` macro
- Converts `MigrationPlan` operations to SQL statements via `PostgresRenderer`
- Extracts breaking changes from `BreakingChangeAnalysis`
- Supports full file output or macro-only output for embedding
- Proper escaping of SQL strings containing quotes, newlines, etc.

---

### Completed: Migration Runner (Phase 4)

The standalone runner has been implemented in `crates/tern-migration-runner/` using Wasmtime v41.

#### Implemented Components

| Component | Location | Description |
|-----------|----------|-------------|
| `MigrationRuntime` | `src/runtime.rs` | Wasmtime wrapper for loading and executing components |
| `HostState` | `src/host.rs` | State management for database connection and dry-run mode |
| `RuntimeError` | `src/error.rs` | Error types for runtime operations |
| `DatabaseError` | `src/error.rs` | Error types for database operations |
| `CliError` | `src/error.rs` | Error types for CLI operations |
| CLI | `src/main.rs` | Command-line interface with clap |

#### CLI Features

- `--database-url` / `DATABASE_URL` - PostgreSQL connection string
- `--dry-run` - Show SQL without executing
- `--describe` - Show migration metadata
- `--show-sql` - List all SQL statements
- `--format json|text` - Output format selection
- `--yes` / `-y` - Skip breaking change confirmation
- `--component` - Path to WebAssembly component file
- `--verbose` / `-v` - Enable debug logging

#### Host Function Implementations

- `database::execute()` - Execute SQL with row count return
- `database::query()` - Execute query returning JSON results
- `log::log()` - Emit log messages via tracing

#### Key Design Decisions

1. **Wasmtime v41**: Uses latest Wasmtime with Component Model support and `HasSelf<T>` pattern for host function linking.

2. **Async bridging**: Host functions use `tokio::runtime::Handle::try_current()` to bridge async database operations within sync Wasm context.

3. **Dry-run mode**: Records SQL statements without execution for preview functionality.

4. **Rich error handling**: PostgreSQL errors include error code, constraint name, and table name for detailed diagnostics.

---

### Completed: Executable Generation Pipeline (Phase 5)

The executable generation infrastructure has been implemented in `src/db/compile/`.

#### Implemented Components

| Component | Location | Description |
|-----------|----------|-------------|
| `Target` | `src/db/compile/executable.rs` | Cross-compilation target enum (Native, Linux, macOS, Windows) |
| `ExecutableBuilder` | `src/db/compile/executable.rs` | Builds standalone executables from Wasm components |
| `BuildResult` | `src/db/compile/executable.rs` | Result containing output path, target, and component size |
| `CompileOptions` | `src/db/compile/mod.rs` | Configuration for migration compilation |
| `MigrationCompilationResult` | `src/db/compile/mod.rs` | Full compilation artifacts with migration record |
| `compile_migration()` | `src/db/compile/mod.rs` | High-level API for compiling schema diffs |
| `compile_and_extract_sql()` | `src/db/compile/mod.rs` | Helper for testing/validation |

#### Key Features

- **Target platforms**: Supports Native, x86_64-linux-gnu, x86_64-linux-musl, x86_64-macos, aarch64-macos, x86_64-windows
- **High-level API**: `compile_migration()` handles the full pipeline from schema diff to compiled source
- **Cross-compilation support**: Verifies target toolchain installation before building
- **Error handling**: Extended `CompileError` with variants for executable compilation errors
- **Comprehensive tests**: Unit tests for Target, ExecutableBuilder, CompileOptions, and compile_migration API

#### Usage Example

```rust
use tern::db::compile::{compile_migration, CompileOptions, Target};

// Compile a migration from schema diff
let result = compile_migration(&source, &target, CompileOptions::new("Add email column"))?;

println!("Migration ID: {}", result.migration_id());
println!("Statements: {}", result.statement_count());
if result.has_breaking_changes() {
    println!("Warning: This migration has breaking changes!");
}
```

---

### Completed: CLI Integration (Phase 6)

The CLI commands and handlers have been implemented in `src/cli/commands/`.

#### Implemented Commands

| Command | Location | Description |
|---------|----------|-------------|
| `init` | `src/cli/commands/init.rs` | Initialize new Tern projects with state backend (empty or from database) |
| `status` | `src/cli/commands/status.rs` | Display current state backend information and schema summary |
| `compile` | `src/cli/commands/compile.rs` | Generate migration source code by comparing state to live database |
| `history` | `src/cli/commands/history.rs` | List migration history with formatted output |
| `show` | `src/cli/commands/show.rs` | Display detailed migration information with SQL output option |
| `record` | `src/cli/commands/record.rs` | Record migrations as applied without execution |
| `inspect` | `src/cli/commands/inspect.rs` | Examine migration files (JSON or Rust source) |
| `verify` | `src/cli/commands/verify.rs` | Check state backend matches database schema |
| `verify-chain` | `src/cli/commands/verify.rs` | Validate migration chain integrity |

#### Key Features

- All commands support `--format` option (text/json/sql where applicable)
- Rich output formatting for both human-readable and machine consumption
- Comprehensive unit tests for each command handler
- Proper error handling with helpful diagnostic messages
- Commands use `--path` option to specify non-default state directory location

#### CLI Usage Examples

```bash
# Initialize empty state backend
tern init

# Initialize from existing database
tern init --from postgres://localhost/mydb

# Check current status
tern status --format json

# Compile migration from database diff
tern compile --database-url postgres://localhost/mydb --description "Add users table" --show-sql

# View migration history
tern history --limit 10

# Show migration details
tern show abc123 --format sql

# Verify state matches database
tern verify --database-url postgres://localhost/mydb

# Verify migration chain integrity
tern verify-chain
```

---

### Completed: Testing (Phase 7)

Comprehensive integration tests have been implemented for the compilation pipeline and state reconstruction.

#### Implemented Test Files

| Test File | Location | Description |
|-----------|----------|-------------|
| Compilation Integration Tests | `tests/compile_integration_tests.rs` | Full compilation pipeline tests |
| State Reconstruction Tests | `tests/state_reconstruction_tests.rs` | State backend and reconstruction tests |

#### Compilation Integration Tests (32 tests)

- **Simple migrations**: Create table, add column, column with default
- **Generated source code**: Macro generation, string escaping, snapshot tests
- **Migration ID determinism**: Same schemas produce same IDs, different descriptions produce different IDs
- **State hash tests**: Stability, roundtrip, hex encoding
- **Breaking change detection**: Dropping table/column, adding NOT NULL, unique constraints
- **Complex migrations**: Multi-table schemas, foreign keys, views and sequences
- **Low-level API tests**: Manual compilation, compiler config effects
- **Plan consistency tests**: Statement count verification

#### State Reconstruction Tests (30 tests)

- **Basic operations**: Save/retrieve migrations, atomic recording, state hash tracking
- **State reconstruction**: Baseline, single migration, migration chains, multiple operations
- **Chain verification**: Valid chains, empty chains, single migration
- **Migrations since**: From zero hash, specific hash, latest hash
- **Persistence**: Data survives across backend instances, sequential file naming
- **Error handling**: Duplicate migrations, not found, uninitialized backend
- **Migration properties**: Baseline, checkpoint, regular migration properties
- **State hash stability**: Empty namespace hash pinned, deterministic hashing

#### Snapshot Tests

Snapshot tests using `cargo-insta` for deterministic output verification:
- Generated source code structure
- Compilation summaries (YAML format)
- SQL extraction
- State reconstruction summaries

---

## Remaining Work

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         Compilation Pipeline                         │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────┐     ┌─────────┐     ┌─────────┐     ┌───────────┐ │
│  │ State       │     │ Schema  │     │Migration│     │   Wasm    │ │
│  │ Backend     │────▶│  Diff   │────▶│  Plan   │────▶│ Component │ │
│  │ (DONE)      │     │ (exists)│     │ (exists)│     │           │ │
│  └─────────────┘     └─────────┘     └─────────┘     └─────┬─────┘ │
│                                                            │       │
│  ┌─────────────┐     ┌─────────────────────────────────────▼─────┐ │
│  │ Target      │     │                                           │ │
│  │ Database    │────▶│           Standalone Executable           │ │
│  └─────────────┘     │  ┌─────────────────────────────────────┐  │ │
│                      │  │  • Embedded Wasmtime runtime        │  │ │
│                      │  │  • Embedded Wasm component          │  │ │
│                      │  │  • PostgreSQL client                │  │ │
│                      │  │  • CLI argument parser              │  │ │
│                      │  └─────────────────────────────────────┘  │ │
│                      └───────────────────────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Phase 1: State Backend Enhancements

**Goal**: Complete the state backend with initialization and state reconstruction features.

### Task 1.1: Add `get_current_state()` method

The current `StateBackend` trait needs a method to retrieve the full schema state, not just the hash.

```rust
// Add to StateBackend trait in src/db/state/mod.rs
async fn get_current_state(&self) -> Result<Namespace, StateError>;
```

For `LocalFileBackend`, this requires storing `state.json` alongside migrations:

```
.tern/
├── state.json              # Current schema state (Namespace)
└── migrations/
    ├── index.json
    └── ...
```

### Task 1.2: Implement `init_from_database()`

Create a function to initialize the state backend from an existing database:

```rust
// src/db/state/init.rs

/// Initialize state backend from an existing database.
pub async fn init_from_database(
    backend: &impl StateBackend,
    catalog: &impl Catalog,
    schema_name: &str,
) -> Result<Migration, StateError> {
    // 1. Introspect current database schema
    let namespace = load_namespace(catalog, schema_name).await?;

    // 2. Initialize backend
    backend.initialize().await?;

    // 3. Create and save baseline migration
    let baseline = Migration::baseline(namespace);
    backend.save_migration(&baseline).await?;

    // 4. Save current state
    backend.save_current_state(&baseline.checkpoint_state.unwrap()).await?;

    Ok(baseline)
}
```

### Task 1.3: Implement state reconstruction

Add `get_state_at()` to reconstruct schema state at any point in history:

```rust
// Add to StateBackend trait
async fn get_state_at(&self, migration_id: &MigrationId) -> Result<Namespace, StateError>;
```

Implementation walks the migration history from the nearest checkpoint:

```rust
impl LocalFileBackend {
    pub async fn get_state_at(&self, target_id: &MigrationId) -> Result<Namespace, StateError> {
        let migrations = self.get_all_migrations().await?;

        // Find target migration
        let target_idx = migrations.iter()
            .position(|m| m.id == *target_id)
            .ok_or(StateError::MigrationNotFound { id: *target_id })?;

        // Find nearest checkpoint before target
        let (mut state, start_idx) = self.find_nearest_checkpoint(&migrations, target_idx)?;

        // Apply operations from checkpoint to target
        let mut oid_gen = OidGenerator::new(state.highest_oid() + 1);
        for migration in &migrations[start_idx..=target_idx] {
            state = state.apply(&migration.operations, &mut oid_gen)?;
        }

        Ok(state)
    }
}
```

### Task 1.4: Add `record_migration()` method

Add a method that both saves a migration and updates the current state:

```rust
// Add to StateBackend trait
async fn record_migration(
    &self,
    migration: &Migration,
    new_state: &Namespace,
) -> Result<(), StateError>;
```

---

## Phase 2: WIT Definition & Crate Setup

**Goal**: Establish the WebAssembly interface contract and create supporting crates.

### Task 2.1: Create WIT definitions crate

Create `crates/tern-migration-wit/` with the WIT interface:

```
crates/tern-migration-wit/
├── Cargo.toml
└── wit/
    └── migration.wit
```

```wit
// crates/tern-migration-wit/wit/migration.wit
package tern:migration@0.1.0;

/// Database access provided by the runtime
interface database {
    record db-error {
        message: string,
    }

    /// Execute a SQL statement, returns rows affected
    execute: func(sql: string) -> result<u64, db-error>;
}

/// Logging provided by the runtime
interface log {
    enum level {
        info,
        warn,
        error,
    }

    log: func(level: level, message: string);
}

/// The migration interface
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

### Task 2.2: Create guest bindings crate

Create `crates/tern-migration-guest/` for generating migration components:

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
                for (i, sql) in statements.iter().enumerate() {
                    $crate::log::log(
                        $crate::Level::Info,
                        &format!("[{}/{}] Executing", i + 1, statements.len()),
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

### Task 2.3: Update workspace Cargo.toml

```toml
[workspace]
members = [
    ".",
    "crates/tern-migration-wit",
    "crates/tern-migration-guest",
    "crates/tern-migration-runner",
]
```

---

## Phase 3: Migration Component Generation

**Goal**: Generate Wasm components from `MigrationPlan`.

### Task 3.1: Add compilation module

Create `src/db/compile/`:

```
src/db/compile/
├── mod.rs
├── error.rs
└── codegen.rs
```

### Task 3.2: Implement `MigrationCompiler`

```rust
// src/db/compile/codegen.rs

pub struct MigrationCompiler {
    cargo_component_path: Option<PathBuf>,
}

impl MigrationCompiler {
    /// Compile a migration plan into a Wasm component
    pub fn compile(
        &self,
        migration: &Migration,
        plan: &MigrationPlan,
    ) -> Result<Vec<u8>, CompileError> {
        // 1. Render SQL statements
        let script = plan.render(&PostgresRenderer::default())?;
        let statements: Vec<String> = script.all_statements().collect();

        // 2. Collect warnings from breaking changes
        let warnings: Vec<String> = migration.breaking_changes
            .iter()
            .map(|bc| bc.description())
            .collect();

        // 3. Generate Rust source using define_migration! macro
        let source = self.generate_source(
            &migration.id.to_hex(),
            &migration.description,
            &statements,
            &warnings,
        );

        // 4. Compile to Wasm component using cargo-component
        self.compile_source(&source)
    }
}
```

---

## Phase 4: Standalone Runner Implementation

**Goal**: Create the runner that becomes the standalone executable.

### Task 4.1: Create runner crate

Create `crates/tern-migration-runner/`:

```
crates/tern-migration-runner/
├── Cargo.toml
├── src/
│   ├── main.rs      # CLI entry point
│   ├── runtime.rs   # Wasmtime setup
│   └── host.rs      # Host function implementations
└── build.rs         # Embeds component at compile time
```

### Task 4.2: Implement CLI entry point

```rust
// crates/tern-migration-runner/src/main.rs

#[derive(Parser)]
struct Cli {
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    #[arg(long, default_value = "false")]
    dry_run: bool,

    #[arg(long, default_value = "false")]
    yes: bool,

    #[arg(long, default_value = "false")]
    describe: bool,

    #[arg(long, default_value = "text")]
    format: String,
}

// Component bytes embedded at compile time
const COMPONENT_BYTES: &[u8] = include_bytes!(env!("MIGRATION_COMPONENT_PATH"));
```

### Task 4.3: Implement Wasmtime runtime

```rust
// crates/tern-migration-runner/src/runtime.rs

pub struct MigrationRuntime {
    engine: Engine,
    component: Component,
}

impl MigrationRuntime {
    pub fn new(component_bytes: &[u8]) -> Result<Self>;
    pub fn describe(&self) -> Result<Metadata>;
    pub fn run_dry(&self) -> Result<()>;
    pub async fn run(&self, client: &Client) -> Result<()>;
}
```

### Task 4.4: Implement host functions

```rust
// crates/tern-migration-runner/src/host.rs

pub struct HostState {
    client: Option<Arc<Mutex<Client>>>,
    dry_run: bool,
}

pub fn add_to_linker(linker: &mut Linker<HostState>) -> Result<()> {
    // database::execute - Execute SQL against PostgreSQL
    // log::log - Emit log messages
}
```

---

## Phase 5: Executable Generation Pipeline

**Goal**: Combine component generation with runner compilation.

### Task 5.1: Implement `ExecutableBuilder`

```rust
// src/db/compile/executable.rs

pub enum Target {
    Native,
    X86_64LinuxGnu,
    X86_64LinuxMusl,
    X86_64MacOS,
    Aarch64MacOS,
    X86_64Windows,
}

pub struct ExecutableBuilder {
    runner_crate_path: PathBuf,
}

impl ExecutableBuilder {
    /// Build a standalone executable from a compiled Wasm component
    pub fn build(
        &self,
        component_bytes: &[u8],
        output_path: &Path,
        target: Target,
    ) -> Result<(), CompileError>;
}
```

### Task 5.2: Create high-level compilation API

```rust
// src/db/compile/mod.rs

pub struct CompileOptions {
    pub description: String,
    pub target: Target,
}

/// Compile a migration from source to target schema
pub fn compile_migration(
    source: &Namespace,
    target: &Namespace,
    output_path: &Path,
    options: CompileOptions,
) -> Result<CompilationResult, CompileError> {
    // 1. Diff schemas
    // 2. Analyze breaking changes
    // 3. Generate migration plan
    // 4. Create Migration record
    // 5. Compile to Wasm component
    // 6. Build standalone executable
}
```

---

## Phase 6: CLI Integration

**Goal**: Add CLI commands for compiling and managing migrations.

### Task 6.1: Add CLI commands

```rust
// src/cli/mod.rs

#[derive(Debug, Parser)]
pub enum CliCommand {
    /// Initialize a new Tern project with state backend
    Init {
        #[arg(long, env = "DATABASE_URL")]
        from: Option<String>,
        #[arg(long, default_value = "public")]
        schema: String,
    },

    /// Show state backend status
    Status,

    /// Compile a migration to a standalone executable
    Compile {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        description: String,
        #[arg(long, default_value = "native")]
        target: String,
        #[arg(long)]
        record: bool,
        #[arg(long)]
        dry_run: bool,
    },

    /// List migration history
    History {
        #[arg(long, default_value = "text")]
        format: String,
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Show details of a specific migration
    Show {
        migration_id: String,
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Record a migration as applied
    Record {
        migration_id: Option<String>,
        #[arg(long)]
        migration_file: Option<PathBuf>,
    },

    /// Inspect a compiled migration
    Inspect {
        path: PathBuf,
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Verify state backend matches database
    Verify {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
}
```

### Task 6.2: Implement command handlers

Create handlers in `src/cli/commands/`:

- `init.rs` - Initialize state backend
- `status.rs` - Show current state
- `compile.rs` - Generate migration executable
- `history.rs` - List migrations
- `show.rs` - Show migration details
- `verify.rs` - Verify state matches database

---

## Phase 7: Testing

**Goal**: Comprehensive tests for the compilation pipeline.

### Task 7.1: Integration tests for compilation

```rust
// tests/compile_integration_tests.rs

#[test]
fn test_compile_simple_migration() {
    let source = NamespaceBuilder::new("public")
        .table("users", |t| t.column("id", "integer").not_null())
        .build();

    let target = NamespaceBuilder::new("public")
        .table("users", |t| {
            t.column("id", "integer").not_null()
             .column("email", "text")
        })
        .build();

    let result = compile_migration(&source, &target, &output, options).unwrap();
    assert!(output.exists());
}
```

### Task 7.2: Integration tests for state reconstruction

```rust
#[tokio::test]
async fn test_state_reconstruction() {
    let backend = LocalFileBackend::new(temp_dir);
    backend.initialize().await.unwrap();

    // Create chain of migrations
    let m1 = Migration::baseline(initial_state);
    backend.save_migration(&m1).await.unwrap();

    let m2 = Migration::new("Add table", ops, ...);
    backend.save_migration(&m2).await.unwrap();

    // Reconstruct state at m1
    let state = backend.get_state_at(&m1.id).await.unwrap();
    assert_eq!(StateHash::from_namespace(&state), m1.resulting_state_hash);
}
```

---

## CLI Usage Examples

### State Backend Initialization

```bash
# Initialize with empty state
tern init

# Initialize from existing database (creates baseline)
tern init --from postgres://localhost/mydb

# Check status
tern status
# Output:
# State Backend: local (.tern/)
# Current State Hash: 3fa55ab02853d2d9...
# Migrations: 1 applied
# Last Migration: Baseline (2024-01-15)
```

### Migration Generation

```bash
# Generate migration by comparing state to database
tern compile \
  --database-url postgres://localhost/mydb \
  --output ./migrations/add_email \
  --description "Add email column to users"

# Generate and record to state backend
tern compile \
  --database-url postgres://localhost/mydb \
  --output ./migrations/add_email \
  --description "Add email column" \
  --record

# Preview without generating
tern compile --database-url postgres://localhost/mydb --dry-run
```

### Running Migrations

```bash
# Run the compiled migration
./migrations/add_email --database-url postgres://localhost/mydb

# Dry run (show what would execute)
./migrations/add_email --database-url postgres://localhost/mydb --dry-run

# Skip warning confirmations
./migrations/add_email --database-url postgres://localhost/mydb --yes

# Get metadata
./migrations/add_email --describe
./migrations/add_email --describe --format json
```

### History Management

```bash
# List migrations
tern history

# Show migration details
tern show 5ac039f6

# Show SQL
tern show 5ac039f6 --format sql

# Verify state matches database
tern verify --database-url postgres://localhost/mydb
```

---

## Dependencies Summary

### Main Tern Crate (existing + new)

```toml
[dependencies]
# Existing...

# State backend (already added)
blake3 = "1"
hex = "0.4"
serde_json = "1.0"
async-trait = "0.1"

# Compilation (to be added)
# (uses tempfile from dev-dependencies)
```

### tern-migration-guest Crate (new)

```toml
[dependencies]
wit-bindgen = "0.36"
```

### tern-migration-runner Crate (implemented)

```toml
[dependencies]
wasmtime = { version = "41", features = ["component-model"] }
clap = { version = "4", features = ["derive", "env"] }
tokio = { version = "1", features = ["full", "sync"] }
tokio-postgres = "0.7"
tokio-postgres-rustls = "0.13"
rustls = "0.23"
webpki-roots = "0.26"
miette = { version = "7", features = ["fancy"] }
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
```

### Build Tools Required

- `cargo-component` - For compiling Wasm components
- Cross-compilation targets as needed

---

## Task Checklist

| Phase | Task | Description | Status |
|-------|------|-------------|--------|
| **1** | 1.1 | Add `get_current_state()` to StateBackend | DONE |
| | 1.2 | Implement `init_from_database()` | DONE |
| | 1.3 | Implement `get_state_at()` for state reconstruction | DONE |
| | 1.4 | Add `record_migration()` method | DONE |
| **2** | 2.1 | Create `tern-migration-wit` crate | DONE |
| | 2.2 | Create `tern-migration-guest` crate | DONE |
| | 2.3 | Update workspace Cargo.toml | DONE |
| **3** | 3.1 | Add `db::compile` module structure | DONE |
| | 3.2 | Implement `MigrationCompiler` | DONE |
| **4** | 4.1 | Create `tern-migration-runner` crate | DONE |
| | 4.2 | Implement CLI entry point | DONE |
| | 4.3 | Implement Wasmtime runtime wrapper | DONE |
| | 4.4 | Implement host functions | DONE |
| **5** | 5.1 | Implement `ExecutableBuilder` | DONE |
| | 5.2 | Create high-level `compile_migration` API | DONE |
| **6** | 6.1 | Add CLI commands | DONE |
| | 6.2 | Implement command handlers | DONE |
| **7** | 7.1 | Integration tests for compilation | DONE |
| | 7.2 | Integration tests for state reconstruction | DONE |

---

## Future Extensions

1. **PostgreSQL Backend**: Store state in `_tern_migrations` schema
2. **S3/Cloud Storage Backend**: Distributed team support
3. **State Branching**: Feature branch support with merge conflict detection
4. **Migration Squashing**: Combine migrations with checkpoints
5. **Drift Detection**: Detect manual schema changes
6. **User code hooks**: `before_run`, `after_run`, `on_error` exports
7. **Rollback execution**: Execute rollback SQL (currently generate-only)
8. **Multi-database support**: MySQL, SQLite, etc.
