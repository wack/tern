# WASI-Based Migration Architecture

## Overview

This document describes the redesigned architecture for Tern's migration compilation and execution system. The new design eliminates runtime dependencies on external tools (like `cargo` or `wasmtime` CLI) by leveraging WebAssembly and WASI, while still producing fully native, standalone executables.

## Goals

1. **Zero runtime dependencies**: Users should not need Rust, Cargo, or Wasmtime installed
2. **Standalone native executables**: Output binaries are fully static, native to the target architecture
3. **No `std::process::Command`**: All operations use Wasmtime's Rust API programmatically
4. **Cross-platform compilation**: Compile once to Wasm, AOT compile for any target
5. **Clean component separation**: Clear interfaces between runner, guest, and migration data

## Current Architecture (Problems)

The current implementation in `src/db/compile/executable.rs` has several issues:

```rust
// PROBLEMATIC: Invokes cargo at runtime
fn run_cargo_build(&self, build_dir: &Path, target: Target) -> Result<PathBuf, CompileError> {
    let mut cmd = Command::new("cargo");  // Requires Rust toolchain on user's machine
    cmd.arg("build");
    // ...
}

// PROBLEMATIC: Invokes rustup at runtime
pub fn verify_target(&self, target: Target) -> Result<(), CompileError> {
    let output = Command::new("rustup")  // Requires rustup on user's machine
        .args(["target", "list", "--installed"])
        // ...
}
```

### Current Flow (Problematic)

```
User runs tern compile
        │
        ▼
Generate Rust source code (define_migration!)
        │
        ▼
Copy tern-migration-runner crate to temp dir
        │
        ▼
Run `cargo build` ← REQUIRES RUST TOOLCHAIN
        │
        ▼
Native executable
```

## New Architecture

### High-Level Component Diagram

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              TERN BINARY                                    │
│                                                                             │
│  Embedded Components (via include_bytes!):                                  │
│  ┌─────────────────────────┐  ┌─────────────────────────────────────────┐   │
│  │ RUNNER_COMPONENT        │  │ GUEST_COMPONENT                         │   │
│  │ (runner.wasm)           │  │ (guest.wasm)                            │   │
│  │                         │  │                                         │   │
│  │ Pre-compiled from       │  │ Pre-compiled from                       │   │
│  │ tern-migration-runner   │  │ tern-migration-guest                    │   │
│  │ Target: wasm32-wasip2   │  │ Target: wasm32-wasip2                   │   │
│  └─────────────────────────┘  └─────────────────────────────────────────┘   │
│                                                                             │
│  Runtime Dependencies (Rust crates):                                        │
│  ┌─────────────────────────┐  ┌─────────────────────────────────────────┐   │
│  │ wasmtime                │  │ wasmtime-wasi                           │   │
│  │ (Component API)         │  │ (WASI implementation)                   │   │
│  └─────────────────────────┘  └─────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Component Responsibilities

#### 1. `tern` (Main Crate)

**Location**: `src/`

**Role**: CLI tool, schema diffing, migration compilation, executable generation

**Key Dependencies**:
- `wasmtime` - For component composition and AOT compilation
- `wasmtime-wasi` - For WASI interface implementation during testing/direct execution

**Embedded Artifacts**:
```rust
// src/db/compile/embedded.rs
/// Pre-compiled migration runner component (wasm32-wasip2)
pub const RUNNER_COMPONENT: &[u8] = include_bytes!(env!("TERN_RUNNER_WASM_PATH"));

/// Pre-compiled migration guest component (wasm32-wasip2)
pub const GUEST_COMPONENT: &[u8] = include_bytes!(env!("TERN_GUEST_WASM_PATH"));
```

**Responsibilities**:
- Parse CLI arguments
- Connect to PostgreSQL and load schemas
- Compute schema diffs
- Generate SQL statements from diffs
- Compose Wasm components (guest + data → migration, runner + migration → final)
- AOT compile to native executable
- Write native executable to disk

#### 2. `tern-migration-runner`

**Location**: `crates/tern-migration-runner/`

**Target**: `wasm32-wasip2`

**Role**: WASI CLI application that orchestrates migration execution

**Dependencies**:
- WASI interfaces (CLI, filesystem, sockets, environment)
- NO `wasmtime` crate (Wasmtime is the host, not a dependency)

**Imports** (from composed migration component):
```wit
// Imports the migration interface
import migration: interface {
    describe: func() -> metadata;
    get-statements: func() -> list<statement>;
    run: func() -> result<_, string>;
}
```

**Exports**:
```wit
// WASI CLI entry point
export wasi:cli/run@0.2.0;
```

**Responsibilities**:
- Parse CLI arguments (--database-url, --dry-run, --describe, etc.)
- Establish database connection via WASI sockets
- Call imported migration functions (describe, run, get-statements)
- Execute SQL against database
- Report progress and results to stdout/stderr

#### 3. `tern-migration-guest`

**Location**: `crates/tern-migration-guest/`

**Target**: `wasm32-wasip2`

**Role**: Generic migration executor that imports SQL data

**Dependencies**:
- Component model bindings (wit-bindgen)
- NO runtime dependencies (pure Wasm component)

**Imports** (SQL data, provided at composition time):
```wit
// Imports migration data
import migration-data: interface {
    get-id: func() -> string;
    get-description: func() -> string;
    get-source-state-hash: func() -> string;
    get-target-state-hash: func() -> string;
    get-compiled-at: func() -> string;
    get-statement-count: func() -> u32;
    get-statement: func(index: u32) -> statement;
    get-breaking-changes: func() -> list<breaking-change>;
}
```

**Exports** (migration interface):
```wit
// Exports the migration interface
export migration: interface {
    describe: func() -> metadata;
    get-statements: func() -> list<statement>;
    run: func() -> result<_, string>;
}
```

**Responsibilities**:
- Implement the `migration` interface by delegating to imported `migration-data`
- Provide the glue between data imports and migration exports
- Handle the `run` function by iterating statements and calling database execute

#### 4. `tern-migration-wit`

**Location**: `crates/tern-migration-wit/`

**Role**: WIT interface definitions (unchanged in purpose, updated interfaces)

**Contents**: All WIT files defining the component interfaces

### New WIT Interface Definitions

#### `migration-data.wit` (New)

```wit
package tern:migration-data@0.1.0;

/// Data interface for migration content.
/// This is imported by the guest and provided by the data component.
interface migration-data {
    /// Statement with metadata.
    record statement {
        sql: string,
        description: string,
        sequence: u32,
    }

    /// Mitigation strategy for breaking changes.
    enum mitigation-strategy {
        dual-write,
        backfill,
        ratchet,
        destructive,
    }

    /// A breaking change in the migration.
    record breaking-change {
        description: string,
        mitigation: mitigation-strategy,
        affected-sql: list<string>,
    }

    /// Get the migration ID (content-addressable hash).
    get-id: func() -> string;

    /// Get the human-readable description.
    get-description: func() -> string;

    /// Get the source schema state hash.
    get-source-state-hash: func() -> string;

    /// Get the target schema state hash.
    get-target-state-hash: func() -> string;

    /// Get the compilation timestamp (RFC 3339).
    get-compiled-at: func() -> string;

    /// Get the total number of SQL statements.
    get-statement-count: func() -> u32;

    /// Get a statement by index (0-based).
    get-statement: func(index: u32) -> statement;

    /// Get all breaking changes.
    get-breaking-changes: func() -> list<breaking-change>;
}
```

#### `migration.wit` (Updated)

```wit
package tern:migration@0.1.0;

/// Database interface provided by the runner to the migration.
interface database {
    /// Database error information.
    record db-error {
        message: string,
        code: option<string>,
        constraint-name: option<string>,
        table-name: option<string>,
    }

    /// Execute a SQL statement, returning rows affected.
    execute: func(sql: string) -> result<u64, db-error>;

    /// Execute a SQL query, returning results as JSON.
    query: func(sql: string) -> result<string, db-error>;
}

/// Logging interface provided by the runner.
interface log {
    /// Log levels.
    enum level {
        debug,
        info,
        warn,
        error,
    }

    /// Log a message at the given level.
    log: func(level: level, message: string);
}

/// Migration metadata.
record metadata {
    id: string,
    description: string,
    breaking-changes: list<breaking-change>,
    statement-count: u32,
    source-state-hash: string,
    target-state-hash: string,
    compiled-at: string,
}

/// Statement with metadata.
record statement {
    sql: string,
    description: string,
    sequence: u32,
}

/// Mitigation strategy for breaking changes.
enum mitigation-strategy {
    dual-write,
    backfill,
    ratchet,
    destructive,
}

/// A breaking change in the migration.
record breaking-change {
    description: string,
    mitigation: mitigation-strategy,
    affected-sql: list<string>,
}

/// Migration interface exported by the migration component.
interface migration {
    /// Get migration metadata without executing.
    describe: func() -> metadata;

    /// Get all SQL statements without executing.
    get-statements: func() -> list<statement>;

    /// Execute the migration.
    run: func() -> result<_, string>;
}
```

#### `runner.wit` (New)

```wit
package tern:runner@0.1.0;

/// World definition for the migration runner.
/// The runner is a WASI CLI application that imports a migration component.
world tern-runner {
    // WASI imports for CLI functionality
    import wasi:cli/environment@0.2.0;
    import wasi:cli/stdin@0.2.0;
    import wasi:cli/stdout@0.2.0;
    import wasi:cli/stderr@0.2.0;
    import wasi:clocks/wall-clock@0.2.0;
    import wasi:filesystem/types@0.2.0;
    import wasi:filesystem/preopens@0.2.0;
    import wasi:sockets/tcp@0.2.0;
    import wasi:sockets/udp@0.2.0;
    import wasi:sockets/network@0.2.0;
    import wasi:random/random@0.2.0;

    // Import the migration interface (satisfied by composed migration component)
    import tern:migration/migration@0.1.0;

    // Export WASI CLI entry point
    export wasi:cli/run@0.2.0;
}
```

#### `guest.wit` (New)

```wit
package tern:guest@0.1.0;

/// World definition for the migration guest.
/// The guest imports data and exports the migration interface.
world tern-guest {
    // Import migration data (satisfied by data component at composition time)
    import tern:migration-data/migration-data@0.1.0;

    // Import database interface (satisfied by runner at composition time)
    import tern:migration/database@0.1.0;

    // Import logging interface (satisfied by runner at composition time)
    import tern:migration/log@0.1.0;

    // Export the migration interface
    export tern:migration/migration@0.1.0;
}
```

## Build Process

### Phase 1: Build Wasm Components

This happens during `cargo build` for the main `tern` crate, orchestrated by `build.rs`.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              build.rs                                       │
│                                                                             │
│  1. Check if runner.wasm needs rebuild                                      │
│     cargo build -p tern-migration-runner --target wasm32-wasip2 --release   │
│                                                                             │
│  2. Check if guest.wasm needs rebuild                                       │
│     cargo build -p tern-migration-guest --target wasm32-wasip2 --release    │
│                                                                             │
│  3. Set environment variables for include_bytes!                            │
│     TERN_RUNNER_WASM_PATH=/path/to/runner.wasm                              │
│     TERN_GUEST_WASM_PATH=/path/to/guest.wasm                                │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Note**: The `build.rs` script MAY use `std::process::Command` to invoke `cargo` because this runs at **Tern's build time**, not at the user's runtime. The constraint is that users of Tern should not need any external tools.

### Phase 2: Embed in Tern Binary

```rust
// src/db/compile/embedded.rs

use std::path::PathBuf;

/// Pre-compiled migration runner component.
/// Built from tern-migration-runner targeting wasm32-wasip2.
pub const RUNNER_COMPONENT: &[u8] = include_bytes!(env!("TERN_RUNNER_WASM_PATH"));

/// Pre-compiled migration guest component.
/// Built from tern-migration-guest targeting wasm32-wasip2.
pub const GUEST_COMPONENT: &[u8] = include_bytes!(env!("TERN_GUEST_WASM_PATH"));

/// Returns the size of the embedded runner component.
pub fn runner_component_size() -> usize {
    RUNNER_COMPONENT.len()
}

/// Returns the size of the embedded guest component.
pub fn guest_component_size() -> usize {
    GUEST_COMPONENT.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_component_is_valid_wasm() {
        // Verify magic number: \0asm
        assert_eq!(&RUNNER_COMPONENT[0..4], b"\0asm");
    }

    #[test]
    fn guest_component_is_valid_wasm() {
        // Verify magic number: \0asm
        assert_eq!(&GUEST_COMPONENT[0..4], b"\0asm");
    }

    #[test]
    fn components_are_not_empty() {
        assert!(runner_component_size() > 1000, "Runner component suspiciously small");
        assert!(guest_component_size() > 100, "Guest component suspiciously small");
    }
}
```

## Runtime Flow

### Step 1: Generate SQL Statements

```rust
// Existing code in src/db/compile/mod.rs
let diff = diff_namespaces(&source, &target);
let plan = MigrationPlan::from_diff(&diff);
let statements: Vec<CompiledStatement> = /* ... */;
let metadata = MigrationMetadata { /* ... */ };
```

### Step 2: Create Data Component

Tern must dynamically generate a Wasm component that provides the `migration-data` interface. This component contains the SQL statements as embedded data.

```rust
// src/db/compile/data_component.rs

use wasmtime::component::Component;

/// Generates a Wasm component that provides the migration-data interface.
///
/// The generated component embeds the SQL statements and metadata as
/// constant data and exports functions to retrieve them.
pub struct DataComponentGenerator {
    engine: wasmtime::Engine,
}

impl DataComponentGenerator {
    pub fn new(engine: &wasmtime::Engine) -> Self {
        Self { engine: engine.clone() }
    }

    /// Generate a data component from migration metadata and statements.
    pub fn generate(
        &self,
        metadata: &MigrationMetadata,
        statements: &[CompiledStatement],
    ) -> Result<Vec<u8>, CompileError> {
        // Option A: Use wasm-encoder to generate component bytes directly
        // Option B: Use wit-component to create a component from a module
        // Option C: Pre-compile a template and patch in the data

        // Implementation details in "Data Component Generation" section below
        todo!()
    }
}
```

### Step 3: Compose Guest + Data → Migration Component

```rust
// src/db/compile/composer.rs

use wasmtime::component::{Component, Linker};

/// Composes Wasm components using Wasmtime's component model.
pub struct ComponentComposer {
    engine: wasmtime::Engine,
}

impl ComponentComposer {
    pub fn new(engine: &wasmtime::Engine) -> Self {
        Self { engine: engine.clone() }
    }

    /// Compose the guest component with a data component.
    ///
    /// The data component satisfies the guest's `migration-data` import.
    /// The result exports the `migration` interface.
    pub fn compose_migration(
        &self,
        guest_bytes: &[u8],
        data_bytes: &[u8],
    ) -> Result<Vec<u8>, CompileError> {
        // Use wasmtime's component composition/linking
        // This connects data's exports to guest's imports
        todo!()
    }

    /// Compose the runner component with a migration component.
    ///
    /// The migration component satisfies the runner's `migration` import.
    /// The result is a complete WASI CLI application.
    pub fn compose_executable(
        &self,
        runner_bytes: &[u8],
        migration_bytes: &[u8],
    ) -> Result<Vec<u8>, CompileError> {
        // Use wasmtime's component composition/linking
        // This connects migration's exports to runner's imports
        todo!()
    }
}
```

### Step 4: AOT Compile to Native

```rust
// src/db/compile/aot.rs

use wasmtime::Engine;
use std::path::Path;

/// Target platform for AOT compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AotTarget {
    /// Native platform (current host).
    Native,
    /// Linux x86_64.
    X86_64Linux,
    /// Linux aarch64.
    Aarch64Linux,
    /// macOS x86_64 (Intel).
    X86_64MacOS,
    /// macOS aarch64 (Apple Silicon).
    Aarch64MacOS,
    /// Windows x86_64.
    X86_64Windows,
}

impl AotTarget {
    /// Get the Wasmtime target triple.
    pub fn triple(&self) -> Option<&'static str> {
        match self {
            Self::Native => None,
            Self::X86_64Linux => Some("x86_64-unknown-linux-gnu"),
            Self::Aarch64Linux => Some("aarch64-unknown-linux-gnu"),
            Self::X86_64MacOS => Some("x86_64-apple-darwin"),
            Self::Aarch64MacOS => Some("aarch64-apple-darwin"),
            Self::X86_64Windows => Some("x86_64-pc-windows-msvc"),
        }
    }
}

/// AOT compiler using Wasmtime.
pub struct AotCompiler {
    engine: Engine,
}

impl AotCompiler {
    pub fn new() -> Result<Self, CompileError> {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config)
            .map_err(CompileError::engine_creation)?;

        Ok(Self { engine })
    }

    /// Create a compiler for a specific target.
    pub fn for_target(target: AotTarget) -> Result<Self, CompileError> {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);

        if let Some(triple) = target.triple() {
            config.target(triple)
                .map_err(|e| CompileError::unsupported_target(triple, e))?;
        }

        let engine = Engine::new(&config)
            .map_err(CompileError::engine_creation)?;

        Ok(Self { engine })
    }

    /// Compile a Wasm component to a native executable.
    ///
    /// This produces a standalone binary that embeds the Wasmtime runtime.
    pub fn compile_to_executable(
        &self,
        component_bytes: &[u8],
        output_path: &Path,
    ) -> Result<AotResult, CompileError> {
        // Load the component
        let component = Component::new(&self.engine, component_bytes)
            .map_err(CompileError::component_load)?;

        // Serialize to AOT format
        let serialized = component.serialize()
            .map_err(CompileError::serialization)?;

        // Create the executable
        // Wasmtime provides APIs to create standalone executables
        // This may involve wasmtime_environ or similar
        self.create_executable(&serialized, output_path)?;

        Ok(AotResult {
            output_path: output_path.to_path_buf(),
            component_size: component_bytes.len(),
            executable_size: std::fs::metadata(output_path)
                .map(|m| m.len() as usize)
                .unwrap_or(0),
        })
    }

    /// Create a standalone executable from serialized component.
    fn create_executable(
        &self,
        serialized: &[u8],
        output_path: &Path,
    ) -> Result<(), CompileError> {
        // Wasmtime's approach to standalone executables:
        // 1. Serialize the component (precompiled machine code)
        // 2. Bundle with a minimal runtime loader
        // 3. Write as executable

        // The exact API depends on Wasmtime version
        // May use wasmtime-cli's internal APIs or similar

        todo!("Implement standalone executable creation")
    }
}

/// Result of AOT compilation.
pub struct AotResult {
    /// Path to the output executable.
    pub output_path: std::path::PathBuf,
    /// Size of the input Wasm component.
    pub component_size: usize,
    /// Size of the output executable.
    pub executable_size: usize,
}
```

### Step 5: Complete ExecutableBuilder Rewrite

```rust
// src/db/compile/executable.rs (rewritten)

use std::path::Path;
use crate::db::compile::{
    aot::{AotCompiler, AotTarget},
    composer::ComponentComposer,
    data_component::DataComponentGenerator,
    embedded::{GUEST_COMPONENT, RUNNER_COMPONENT},
    CompileError, CompiledStatement, MigrationMetadata,
};

/// Builder for standalone migration executables.
///
/// Takes migration metadata and SQL statements, and produces a
/// platform-native executable without invoking any external tools.
pub struct ExecutableBuilder {
    engine: wasmtime::Engine,
    target: AotTarget,
}

impl ExecutableBuilder {
    /// Create a new builder for the native platform.
    pub fn new() -> Result<Self, CompileError> {
        Self::for_target(AotTarget::Native)
    }

    /// Create a builder for a specific target platform.
    pub fn for_target(target: AotTarget) -> Result<Self, CompileError> {
        let mut config = wasmtime::Config::new();
        config.wasm_component_model(true);

        if let Some(triple) = target.triple() {
            config.target(triple)
                .map_err(|e| CompileError::unsupported_target(triple, e))?;
        }

        let engine = wasmtime::Engine::new(&config)
            .map_err(CompileError::engine_creation)?;

        Ok(Self { engine, target })
    }

    /// Build a standalone executable from migration data.
    ///
    /// # Process
    ///
    /// 1. Generate a data component containing the SQL statements
    /// 2. Compose: guest + data → migration component
    /// 3. Compose: runner + migration → complete CLI component
    /// 4. AOT compile → native executable
    ///
    /// No external tools are invoked. Everything uses Wasmtime's Rust API.
    pub fn build(
        &self,
        metadata: &MigrationMetadata,
        statements: &[CompiledStatement],
        output_path: &Path,
    ) -> Result<BuildResult, CompileError> {
        // Step 1: Generate data component
        let data_generator = DataComponentGenerator::new(&self.engine);
        let data_component = data_generator.generate(metadata, statements)?;

        // Step 2: Compose guest + data
        let composer = ComponentComposer::new(&self.engine);
        let migration_component = composer.compose_migration(
            GUEST_COMPONENT,
            &data_component,
        )?;

        // Step 3: Compose runner + migration
        let complete_component = composer.compose_executable(
            RUNNER_COMPONENT,
            &migration_component,
        )?;

        // Step 4: AOT compile to native executable
        let aot = AotCompiler::for_target(self.target)?;
        let result = aot.compile_to_executable(&complete_component, output_path)?;

        Ok(BuildResult {
            output_path: result.output_path,
            target: self.target,
            component_size: result.component_size,
            executable_size: result.executable_size,
        })
    }
}

impl Default for ExecutableBuilder {
    fn default() -> Self {
        Self::new().expect("Failed to create default ExecutableBuilder")
    }
}

/// Result of building a standalone executable.
#[derive(Debug)]
pub struct BuildResult {
    /// Path to the output executable.
    pub output_path: std::path::PathBuf,
    /// Target platform.
    pub target: AotTarget,
    /// Size of the composed Wasm component in bytes.
    pub component_size: usize,
    /// Size of the output executable in bytes.
    pub executable_size: usize,
}
```

## Data Component Generation

The most complex part of this design is generating the data component at runtime. There are several approaches:

### Option A: Direct Wasm Generation (Recommended)

Use `wasm-encoder` and `wit-component` to generate component bytes directly:

```rust
use wasm_encoder::{
    ComponentBuilder, ComponentExportKind, ComponentTypeEncoder,
    CoreTypeEncoder, Module, ModuleSection,
};
use wit_component::ComponentEncoder;

/// Generate a Wasm component that exports the migration-data interface.
pub fn generate_data_component(
    metadata: &MigrationMetadata,
    statements: &[CompiledStatement],
) -> Result<Vec<u8>, CompileError> {
    // 1. Create a core Wasm module with:
    //    - Data section containing serialized metadata and statements
    //    - Exported functions that read from the data section

    // 2. Wrap in a component that exports the migration-data interface

    // This requires careful construction of the component binary format
    // but avoids any compilation step

    todo!()
}
```

### Option B: Template Patching

Pre-compile a template data component with placeholder data, then patch:

```rust
/// Pre-compiled template with placeholder data.
const DATA_TEMPLATE: &[u8] = include_bytes!("data_template.wasm");

/// Markers in the template for patching.
const METADATA_MARKER: &[u8] = b"__TERN_METADATA_PLACEHOLDER__";
const STATEMENTS_MARKER: &[u8] = b"__TERN_STATEMENTS_PLACEHOLDER__";

pub fn generate_data_component(
    metadata: &MigrationMetadata,
    statements: &[CompiledStatement],
) -> Result<Vec<u8>, CompileError> {
    let mut component = DATA_TEMPLATE.to_vec();

    // Serialize data
    let metadata_bytes = serialize_metadata(metadata);
    let statements_bytes = serialize_statements(statements);

    // Find and replace markers
    patch_bytes(&mut component, METADATA_MARKER, &metadata_bytes)?;
    patch_bytes(&mut component, STATEMENTS_MARKER, &statements_bytes)?;

    Ok(component)
}
```

### Option C: Wit-Bindgen Code Generation

Generate Rust source and compile to Wasm at Tern build time:

```rust
/// This approach generates a data component at Tern build time
/// with all possible statement counts pre-compiled.
///
/// At runtime, select the appropriate pre-compiled component
/// and patch in the actual data.
///
/// Less flexible but simpler implementation.
```

### Recommendation

**Option A (Direct Wasm Generation)** is recommended because:

1. Most flexible - handles any number of statements
2. No template maintenance
3. Smallest output size (no placeholders)
4. Full control over the component structure

The implementation requires understanding the component model binary format, but libraries like `wasm-encoder` and `wit-component` provide the necessary primitives.

## Crate Changes Summary

### `tern` (Main Crate)

**Cargo.toml additions:**
```toml
[dependencies]
wasmtime = { version = "28", features = ["component-model", "cranelift"] }
wasmtime-wasi = { version = "28" }
wasm-encoder = "0.219"
wit-component = "0.219"

[build-dependencies]
# For compiling runner and guest at build time
```

**New files:**
- `src/db/compile/embedded.rs` - Embedded Wasm components
- `src/db/compile/data_component.rs` - Data component generation
- `src/db/compile/composer.rs` - Component composition
- `src/db/compile/aot.rs` - AOT compilation

**Modified files:**
- `src/db/compile/executable.rs` - Complete rewrite
- `src/db/compile/mod.rs` - Updated exports
- `build.rs` - Build runner and guest components

### `tern-migration-runner`

**Cargo.toml changes:**
```toml
[package]
name = "tern-migration-runner"

[lib]
crate-type = ["cdylib"]  # For Wasm component output

[dependencies]
# Remove wasmtime dependency entirely
wit-bindgen = "0.35"

# WASI support
# (wit-bindgen generates WASI bindings)

[dependencies.tokio]
# May need async runtime that works in WASI
# Or use blocking I/O with WASI sockets
```

**Target:**
```bash
cargo build --target wasm32-wasip2 --release
```

**Major changes:**
- Remove all `wasmtime` usage
- Use `wit-bindgen` to generate bindings for WIT interfaces
- Implement CLI using WASI interfaces
- Import `migration` interface instead of loading it dynamically

### `tern-migration-guest`

**Cargo.toml changes:**
```toml
[package]
name = "tern-migration-guest"

[lib]
crate-type = ["cdylib"]  # For Wasm component output

[dependencies]
wit-bindgen = "0.35"
```

**Target:**
```bash
cargo build --target wasm32-wasip2 --release
```

**Major changes:**
- Remove `define_migration!` macro (no longer needed)
- Import `migration-data` interface
- Export `migration` interface
- Pure delegation from exports to imports

### `tern-migration-wit`

**Changes:**
- Add `migration-data.wit`
- Add `runner.wit`
- Add `guest.wit`
- Update `migration.wit` as needed

## Migration Path

### Phase 1: Infrastructure

1. Add Wasmtime dependencies to `tern`
2. Create `build.rs` for compiling Wasm components
3. Implement embedded component loading
4. Add basic tests for component validity

### Phase 2: Data Component Generation

1. Implement `DataComponentGenerator` using `wasm-encoder`
2. Test data component generation with sample migrations
3. Verify generated components satisfy the interface

### Phase 3: Component Composition

1. Implement `ComponentComposer`
2. Test guest + data composition
3. Test runner + migration composition
4. Verify composed components work correctly

### Phase 4: Runner Rewrite

1. Rewrite `tern-migration-runner` for WASI target
2. Remove Wasmtime dependency
3. Implement CLI using WASI
4. Implement database connectivity using WASI sockets
5. Test in Wasmtime CLI first

### Phase 5: Guest Rewrite

1. Rewrite `tern-migration-guest` as pure delegation
2. Remove `define_migration!` macro
3. Test with data component

### Phase 6: AOT Compilation

1. Implement `AotCompiler`
2. Test native executable generation
3. Test cross-compilation targets
4. Benchmark executable sizes and performance

### Phase 7: Integration

1. Rewrite `ExecutableBuilder` to use new pipeline
2. Remove old `Command::new("cargo")` code
3. End-to-end testing
4. Documentation updates

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_components_are_valid() {
        // Verify magic bytes
        assert_eq!(&RUNNER_COMPONENT[0..4], b"\0asm");
        assert_eq!(&GUEST_COMPONENT[0..4], b"\0asm");
    }

    #[test]
    fn can_load_runner_component() {
        let engine = Engine::new(&Config::new().wasm_component_model(true)).unwrap();
        let component = Component::new(&engine, RUNNER_COMPONENT).unwrap();
        // Verify expected imports/exports
    }

    #[test]
    fn can_generate_data_component() {
        let metadata = test_metadata();
        let statements = vec![test_statement()];

        let generator = DataComponentGenerator::new(&engine);
        let bytes = generator.generate(&metadata, &statements).unwrap();

        // Verify valid component
        let component = Component::new(&engine, &bytes).unwrap();
    }

    #[test]
    fn can_compose_migration() {
        let data = generate_test_data_component();
        let composer = ComponentComposer::new(&engine);
        let migration = composer.compose_migration(GUEST_COMPONENT, &data).unwrap();

        // Verify exports migration interface
    }

    #[test]
    fn can_build_executable() {
        let builder = ExecutableBuilder::new().unwrap();
        let result = builder.build(
            &test_metadata(),
            &[test_statement()],
            Path::new("/tmp/test-migration"),
        ).unwrap();

        // Verify executable exists and is executable
        assert!(result.output_path.exists());

        // On Unix, verify executable bit
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&result.output_path).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0);
        }
    }
}
```

### Integration Tests

```rust
#[test]
fn end_to_end_migration_executable() {
    // 1. Create source and target schemas
    let source = Namespace::empty("public");
    let target = namespace_with_users_table();

    // 2. Compile migration
    let result = compile_migration(&source, &target, CompileOptions::new("Add users")).unwrap();

    // 3. Build executable
    let builder = ExecutableBuilder::new().unwrap();
    let exe_result = builder.build(
        &result.migration.metadata(),
        &result.compilation.statements,
        &temp_dir.path().join("migration"),
    ).unwrap();

    // 4. Run executable with --dry-run
    let output = std::process::Command::new(&exe_result.output_path)
        .arg("--dry-run")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("CREATE TABLE"));
}
```

## Open Questions

### 1. WASI Sockets for Database Connectivity

WASI sockets (`wasi:sockets`) provide TCP/UDP networking. Need to verify:
- Can we establish TLS connections? (Required for PostgreSQL)
- Is there a PostgreSQL client that works in WASI?
- May need to implement a minimal PostgreSQL protocol client

**Alternatives:**
- Use `wasi:http` if available and proxy through an HTTP-to-PostgreSQL gateway
- Implement custom host function for database access (less portable)

### 2. Async Runtime in WASI

The current runner uses Tokio. Options for WASI:
- Use synchronous/blocking I/O (simpler, WASI sockets support this)
- Use a WASI-compatible async runtime
- Implement custom async primitives

**Recommendation:** Start with synchronous I/O for simplicity.

### 3. Wasmtime Standalone Executable API

Wasmtime's API for creating standalone executables needs investigation:
- `wasmtime compile` CLI produces `.cwasm` (precompiled module)
- Need runtime stub that loads and runs `.cwasm`
- May need to vendor parts of `wasmtime-cli`

**Alternatives:**
- Ship `runner.cwasm` + minimal loader as separate files
- Embed precompiled code in a generated C stub and compile with system CC

### 4. Component Composition API

Wasmtime's component composition API:
- `wasmtime::component::Linker` can satisfy imports
- May need `wasm-compose` tool functionality as a library
- Investigate `wit-component` crate's composition features

### 5. Cross-Compilation Targets

Verify Wasmtime supports AOT compilation for all targets:
- x86_64-unknown-linux-gnu ✓
- aarch64-unknown-linux-gnu ✓
- x86_64-apple-darwin ✓
- aarch64-apple-darwin ✓
- x86_64-pc-windows-msvc ?

## References

- [Wasmtime Book](https://docs.wasmtime.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [WASI Preview 2](https://github.com/WebAssembly/WASI/blob/main/preview2/README.md)
- [wit-bindgen](https://github.com/bytecodealliance/wit-bindgen)
- [wasm-tools](https://github.com/bytecodealliance/wasm-tools) (includes wasm-encoder, wit-component)
- [wasmtime Rust API](https://docs.rs/wasmtime/latest/wasmtime/)
