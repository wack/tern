# Standalone Migration Executables via WebAssembly

## Overview

This document describes the design for compiling database migrations into standalone, platform-native executables using WebAssembly. When run, these executables connect to a PostgreSQL database and apply the migration.

### Goals

1. **Self-contained executables**: A single binary that contains all migration logic
2. **Platform independence**: Generate executables for any supported target platform
3. **No runtime dependencies**: Users don't need Tern installed to run migrations
4. **Breaking change warnings**: Warn users about potentially dangerous operations
5. **Inspectable artifacts**: Extract metadata and SQL without executing

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
    └── migration_compile_tests.rs
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

#### Task 5.1: Add compile command

Update `src/cli/mod.rs` to add new commands:

```rust
// src/cli/mod.rs (additions)

#[derive(Debug, Parser)]
pub enum CliCommand {
    // ... existing commands ...

    /// Compile a migration to a standalone executable
    #[command(name = "compile")]
    Compile {
        /// Source database URL (current state)
        #[arg(long, env = "SOURCE_DATABASE_URL")]
        from: String,

        /// Target schema file or database URL
        #[arg(long)]
        to: String,

        /// Output path for the executable
        #[arg(short, long)]
        output: PathBuf,

        /// Migration identifier
        #[arg(long)]
        id: Option<String>,

        /// Migration description
        #[arg(long)]
        description: Option<String>,

        /// Target platform
        #[arg(long, default_value = "native")]
        target: String,
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
}
```

#### Task 5.2: Implement command handlers

**Files to create:**
- `src/cli/commands/compile.rs`
- `src/cli/commands/inspect.rs`

```rust
// src/cli/commands/compile.rs

use crate::db::compile::{compile_migration, CompileOptions, Target};
use crate::db::query::{load_namespace, PostgresCatalog};
use miette::Result;
use std::path::PathBuf;

pub async fn run_compile(
    from: &str,
    to: &str,
    output: &PathBuf,
    id: Option<String>,
    description: Option<String>,
    target_str: &str,
) -> Result<()> {
    // Parse target
    let target = parse_target(target_str)?;

    // Load source schema
    eprintln!("Loading source schema from {}...", from);
    let source = load_schema_from_url(from).await?;

    // Load target schema
    eprintln!("Loading target schema from {}...", to);
    let target_schema = load_schema(to).await?;

    // Generate ID if not provided
    let id = id.unwrap_or_else(|| {
        chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string()
    });

    // Generate description if not provided
    let description = description.unwrap_or_else(|| {
        format!("Migration {}", id)
    });

    eprintln!("Compiling migration {}...", id);

    let result = compile_migration(
        &source,
        &target_schema,
        output,
        CompileOptions {
            id: id.clone(),
            description,
            target,
        },
    )?;

    eprintln!("Compiled migration to {}", output.display());
    eprintln!("  Statements: {}", result.statement_count);

    if !result.warnings.is_empty() {
        eprintln!("  Warnings:");
        for warning in &result.warnings {
            eprintln!("    - {}", warning);
        }
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

async fn load_schema_from_url(url: &str) -> Result<Namespace> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .into_diagnostic()?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("Connection error: {}", e);
        }
    });

    let catalog = PostgresCatalog::new(&client);
    load_namespace(&catalog, "public").await.into_diagnostic()
}

async fn load_schema(path_or_url: &str) -> Result<Namespace> {
    if path_or_url.starts_with("postgres://") || path_or_url.starts_with("postgresql://") {
        load_schema_from_url(path_or_url).await
    } else {
        // Load from SQL file
        // TODO: Implement SQL file parsing
        Err(miette::miette!("SQL file loading not yet implemented"))
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

```bash
# Compile a migration from database diff
tern compile \
  --from postgres://localhost/mydb_dev \
  --to postgres://localhost/mydb_staging \
  --output ./migrations/20240115_schema_update \
  --id "20240115_schema_update" \
  --description "Add user preferences table"

# Compile for a specific platform
tern compile \
  --from postgres://localhost/mydb \
  --to ./schema/target.sql \
  --output ./dist/migration-linux \
  --target x86_64-linux-musl

# Inspect a compiled migration
tern inspect ./migrations/20240115_schema_update
# Output:
# Migration: 20240115_schema_update
# Description: Add user preferences table
# Statements: 5
# Warnings:
#   - Column 'users.email' will be made NOT NULL

# Extract SQL from compiled migration
tern inspect ./migrations/20240115_schema_update --format sql
# Output:
# -- Migration: 20240115_schema_update
# -- Description: Add user preferences table
# BEGIN;
# ALTER TABLE users ADD COLUMN preferences JSONB;
# ...
# COMMIT;

# Run the compiled migration
./migrations/20240115_schema_update --database-url postgres://localhost/mydb

# Run with dry-run mode
./migrations/20240115_schema_update --database-url postgres://localhost/mydb --dry-run

# Skip warning confirmation
./migrations/20240115_schema_update --database-url postgres://localhost/mydb --yes

# Get migration info
./migrations/20240115_schema_update --describe
./migrations/20240115_schema_update --describe --format json
```

---

## Dependencies Summary

### Main Tern Crate

```toml
[dependencies]
# Existing dependencies...

# New dependencies for compilation
tempfile = "3"
chrono = "0.4"
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
| **5** | 5.1 | Add `compile` and `inspect` CLI commands |
| | 5.2 | Implement command handlers |
| **6** | 6.1 | Unit tests for code generation |
| | 6.2 | Integration tests for full pipeline |

---

## Future Extensions

This design intentionally leaves room for future enhancements:

1. **User code hooks**: Add optional exports (`before_run`, `after_run`, `on_error`) that user-provided components can implement

2. **Rollback execution**: Currently rollback SQL is generated but not executed; could add `--rollback` flag

3. **Multi-database support**: Abstract the `database` interface to support MySQL, SQLite, etc.

4. **Checkpoints**: For long-running migrations, add checkpoint/resume capability

5. **Progress reporting**: Add progress callbacks for migrations with many statements

6. **Remote execution**: Allow migrations to execute against remote databases through a secure tunnel
