# CLAUDE.md

> **IMPORTANT**: This is a production-ready repository, and therefore must adhere to a high bar of professional quality.

## Project Overview

**Tern** is a database migration tool written in Rust. The project is named after the tern bird, known for making the longest migrations of any bird species—a fitting metaphor for a tool that manages database schema migrations.

## Build & Development Commands

### Prerequisites

- Rust toolchain (stable)
- cargo-make: `cargo install cargo-make`
- cargo-nextest: `cargo install cargo-nextest`

### Common Commands

```bash
# Run the full CI pipeline (format check, clippy, build, tests, coverage)
cargo make ci-flow

# Development workflow with formatting
cargo make dev-test-flow

# Run tests only
cargo make test

# Format code
cargo make format

# Check formatting without applying changes
cargo make check-format

# Run clippy linting
cargo make clippy

# Watch mode for tests (rerun on file changes)
cargo make bacon

# Generate CLI reference documentation
cargo make gen-cli-reference
```

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run the CLI
cargo run -- --help
```

## Project Structure

```
src/
├── lib.rs                  # Library root (exports cli and db modules)
├── bin/
│   └── main.rs             # Binary entry point
├── cli/
│   ├── mod.rs              # CLI command definitions using clap
│   └── colors.rs           # Color output handling (Auto/Always/Never)
└── db/
    ├── mod.rs              # Database module root, connection handling
    ├── schema.rs           # Core schema primitives (Oid, name types)
    ├── model/              # Domain model for PostgreSQL schema representation
    │   ├── mod.rs          # Re-exports main types
    │   ├── column.rs       # Column definitions (type, default, identity, generated)
    │   ├── constraint.rs   # Constraints (PK, FK, unique, check, exclusion)
    │   ├── index.rs        # Index definitions with columns, sort order, predicates
    │   ├── namespace.rs    # Schema/namespace with tables, views, sequences, enums
    │   ├── table.rs        # Table definitions aggregating columns/constraints/indexes
    │   └── types.rs        # Shared types (expressions, type info, qualified names)
    ├── query/              # PostgreSQL catalog query module (sans-I/O design)
    │   ├── mod.rs          # Re-exports main types
    │   ├── catalog.rs      # Catalog trait abstracting database queries
    │   ├── postgres.rs     # Real implementation using tokio-postgres
    │   ├── fake.rs         # Test double for unit testing without DB
    │   ├── loader.rs       # Pure business logic for loading schema
    │   ├── sql.rs          # SQL query definitions for catalog queries
    │   └── error.rs        # Query error types
    └── diff/               # Schema comparison and diff generation
        ├── mod.rs          # Re-exports main types
        ├── compare.rs      # Entry point for comparing namespaces
        ├── schema_diff.rs  # Diff types for each schema object
        ├── similarity.rs   # Similarity scoring for rename detection
        ├── types.rs        # Core diff types (Diff, DiffConfig, PotentialRename)
        └── tests.rs        # Comprehensive diff tests
```

## Architecture

### Design Principles

The following principles guide architectural decisions in Tern:

1. **No External Runtime Dependencies**: The Tern binary must be fully self-contained. Users should not need to install any external tools or runtimes to use Tern. The only external dependency permitted at runtime is PostgreSQL (the database being migrated). All functionality—including WebAssembly compilation, component composition, and AOT native code generation—must be implemented using Rust libraries embedded in the binary.

2. **No Subprocess Execution**: Tern must not invoke external processes using `std::process::Command` or similar mechanisms at runtime. This ensures:
   - Predictable behavior across all platforms
   - No hidden dependencies on system-installed tools
   - Consistent error handling and user experience
   - Security (no shell injection vulnerabilities)

   **Exception**: The `build.rs` script may invoke `cargo` during Tern's own compilation (not at user runtime) to build WebAssembly components that are then embedded in the binary.

3. **WebAssembly-Based Compilation**: Migration executables are produced using the WebAssembly component model:
   - Pre-compiled WASI components are embedded in the Tern binary
   - Component composition and AOT compilation use Wasmtime's Rust API
   - The output is a standalone native executable with the Wasmtime runtime embedded
   - Cross-compilation is simplified: compile to Wasm once, then AOT compile for each target platform

### Core Design Patterns

- **Sans-I/O Pattern**: The `db::query` module separates I/O from business logic:
  - `Catalog` trait abstracts database queries
  - `PostgresCatalog` implements real database access
  - `FakeCatalog` provides a test double for unit testing without a database
  - `load_namespace` contains pure business logic that works with any `Catalog`

- **Domain Model**: The `db::model` module provides a complete in-memory representation of PostgreSQL schemas that is:
  - **Serializable**: Can be saved/loaded for migration state tracking
  - **Comparable**: Can diff two schemas to detect changes
  - **Complete**: Captures enough detail to regenerate DDL

- **Schema Diffing**: The `db::diff` module compares schemas to identify:
  - Added, removed, and modified objects
  - Potential renames (detected by structural similarity scoring)

### Key Dependencies

- **CLI Framework**: Uses `clap` with derive macros for argument parsing
- **Async Runtime**: `tokio` with full features
- **Database**: `tokio-postgres` with `rustls` TLS support
- **Error Handling**: `miette` for user-facing diagnostics, `thiserror` for error type definitions
- **Logging**: `tracing` with `tracing-subscriber` (supports text and JSON formats)
- **Serialization**: `serde` for data serialization
- **Type Safety**: `nutype` for newtype wrappers, `bon` for builder patterns

## Code Style & Conventions

- Rust Edition 2024
- Code must pass `rustfmt` formatting checks
- Code must pass `clippy` linting with no warnings
- Use `thiserror` for defining error types
- Use `miette` for rich error diagnostics
- Prefer async/await patterns with tokio

## Commit Message Conventions

This project uses a modified conventional commit format to enable automatic changelog generation with git-cliff.

### Format

**Commit Header (Subject Line):**
- Write a clear, descriptive message explaining what the change does
- Do NOT use conventional commit format in the header
- Good: "Add OCI image output support for migration executables"
- Bad: "FEAT: add oci image output"

**Commit Body:**
- Include a SINGLE LINE in conventional commit format starting with one of these prefixes:
  - `FEATURE:` - New functionality
  - `BUG:` - Bug fixes
  - `BREAKING:` - Breaking changes
  - `SECURITY:` - Security fixes
  - `REFACTOR:` - Code refactoring
  - `DOCS:` - Documentation changes
  - `PERF:` - Performance improvements
  - `TEST:` - Test changes
  - `CI:` - Build/CI changes
  - `CHORE:` - Miscellaneous tasks
- If none of these categories apply, you may omit the conventional commit line
- This line MUST be a single line only

### Examples

Good commit message:
```
Add OCI image output support for migration executables

FEATURE: Add OCI image packaging format for Kubernetes deployments

Implements a new output format that packages migration executables
as OCI images, allowing users to run migrations in Kubernetes clusters
without additional container image creation steps.
```

Good commit message for a bug fix:
```
Fix Rust toolchain action name in bump-tag workflow

BUG: Fix incorrect action name causing CI failures
```

Good commit message for a breaking change:
```
Remove deprecated execute-sql command

BREAKING: Remove execute-sql command, use run-migration instead

The execute-sql command has been deprecated since v0.8 and is now
removed. Users should migrate to the run-migration command.
```

## Agent Instructions

**IMPORTANT**: Before completing any task, the agent must:

1. **Always run the formatter**: Execute `cargo make format` to ensure all code adheres to Rust formatting standards
2. **Always run the linter**: Execute `cargo make clippy` to check for linting warnings and ensure code quality

These steps are mandatory and must be performed on all modifications before marking a task as complete. Do not skip these checks even if the changes appear minor.

**All lint violations must be resolved**, including any that are unrelated to the current changes. Before marking a task complete, ensure that `cargo make clippy` produces no warnings whatsoever.

## Testing

- Tests are run using `cargo-nextest` (faster than default cargo test)
- Run tests with: `cargo make test`
- Use `pretty_assertions` for enhanced assertion output in tests
- Use `static_assertions` for compile-time checks

## CI/CD

GitHub Actions workflow runs on all pushes (except `trunk` branch):
1. Format check
2. Clippy linting
3. Build
4. Tests
5. Coverage reporting

All CI checks must pass before merging.

## Future Improvements

This section documents features that have been discussed and designed but not yet implemented. These represent planned enhancements that should be considered when extending the codebase.

### Partition Tracking

PostgreSQL supports table partitioning (range, list, hash) for managing large tables. The current schema model does not capture partition information.

**What would need to be added:**

1. **Partition key representation**: Store the partition strategy (RANGE, LIST, HASH) and the partition key expression (e.g., `PARTITION BY RANGE (created_at)`).

2. **Parent-child relationships**: Track which tables are partitions of which parent tables. This requires:
   - A `parent_table: Option<QualifiedTableName>` field on `Table`
   - Or a separate `Partition` type that references its parent
   - Tracking of partition bounds (e.g., `FOR VALUES FROM ('2024-01-01') TO ('2024-02-01')`)

3. **Partition-specific constraints**: Partitions inherit constraints from parents but can have additional partition-specific constraints.

4. **Diff considerations**: Partition changes are complex:
   - Adding/removing partitions
   - Changing partition bounds
   - Attaching/detaching partitions
   - Converting between partitioned and non-partitioned tables

**Catalog queries needed:**
- `pg_partitioned_table`: Partition strategy and key
- `pg_inherits`: Parent-child relationships
- `pg_class.relispartition`: Whether a table is a partition

### Advanced Similarity Scoring for Rename Detection

**Status: Implemented** in `src/db/diff/similarity.rs`

The rename detection system now includes configurable weighted similarity scoring via `SimilarityConfig`:

- **Weighted column matching**: PK and FK columns receive bonus multipliers
- **Constraint similarity**: Compares PK, FK, and unique constraint structures
- **Index structure comparison**: Similar index definitions contribute to similarity score
- **Name similarity heuristics**: Levenshtein distance for detecting common prefixes/suffixes

**Potential future enhancements:**

1. **Constraint graph analysis**: Tables with similar FK relationships to other tables could strengthen rename detection.

2. **Historical tracking**: Store previous schema snapshots to detect rename patterns over time.

3. **Machine learning approach**: Train a model on known rename operations to predict renames based on structural features.

### Foreign Tables and Foreign Data Wrappers

PostgreSQL supports foreign tables through the Foreign Data Wrapper (FDW) system, allowing access to external data sources.

**What would need to be added:**

1. **FDW representation**: Track installed foreign data wrappers and their options.

2. **Foreign server representation**: Servers configured for each FDW with connection options.

3. **Foreign table representation**: Tables that reference foreign servers with column mappings and options.

4. **User mappings**: Per-user credentials for foreign servers.

**Catalog queries needed:**
- `pg_foreign_data_wrapper`: FDW definitions
- `pg_foreign_server`: Server configurations
- `pg_foreign_table`: Foreign table metadata
- `pg_user_mapping`: User credentials

### Extension Tracking

PostgreSQL extensions add functionality (PostGIS, pg_trgm, etc.) and may create schema objects.

**What would need to be added:**

1. **Extension inventory**: Track which extensions are installed and their versions.

2. **Extension-owned objects**: Many extensions create functions, types, and operators. These should be marked as extension-owned to avoid including them in migration diffs.

3. **Extension dependencies**: Some extensions depend on others.

4. **Version tracking**: Extension upgrades may require specific migration steps.

**Catalog queries needed:**
- `pg_extension`: Installed extensions
- `pg_depend` with `deptype = 'e'`: Extension-owned objects

### Function and Procedure Tracking

PostgreSQL functions and procedures are important schema objects not currently tracked.

**What would need to be added:**

1. **Function representation**:
   - Name and schema
   - Argument types and names
   - Return type
   - Language (SQL, PL/pgSQL, etc.)
   - Function body/definition
   - Volatility (IMMUTABLE, STABLE, VOLATILE)
   - Security definer vs invoker
   - Parallel safety

2. **Procedure representation**: Similar to functions but for procedures (no return value, can use transactions).

3. **Aggregate functions**: Custom aggregates with state transition functions.

4. **Trigger functions**: Functions used by triggers.

5. **Overload handling**: Functions can be overloaded by argument types.

**Diff considerations:**
- Function signature changes require DROP + CREATE
- Body-only changes can use CREATE OR REPLACE
- Dependent objects (triggers, views) may need recreation

### Trigger Tracking

Database triggers execute functions in response to table events.

**What would need to be added:**

1. **Trigger representation**:
   - Name and table
   - Timing (BEFORE, AFTER, INSTEAD OF)
   - Events (INSERT, UPDATE, DELETE, TRUNCATE)
   - Row-level vs statement-level
   - Trigger function reference
   - WHEN condition
   - Enabled/disabled state

2. **Trigger ordering**: PostgreSQL allows specifying trigger execution order.

3. **Constraint triggers**: Special triggers used for constraint enforcement.

**Catalog queries needed:**
- `pg_trigger`: Trigger definitions
- Cross-reference with `pg_proc` for trigger functions

### Row-Level Security (RLS) Policies

PostgreSQL RLS allows fine-grained access control at the row level.

**What would need to be added:**

1. **RLS status**: Whether RLS is enabled/forced on a table.

2. **Policy representation**:
   - Policy name
   - Target table
   - Command type (ALL, SELECT, INSERT, UPDATE, DELETE)
   - Roles the policy applies to
   - USING expression (for existing rows)
   - WITH CHECK expression (for new/modified rows)
   - Permissive vs restrictive

**Catalog queries needed:**
- `pg_class.relrowsecurity` and `relforcerowsecurity`: RLS status
- `pg_policy`: Policy definitions

### Domain Types

PostgreSQL domains are user-defined types based on existing types with optional constraints.

**What would need to be added:**

1. **Domain representation**:
   - Name and schema
   - Base type
   - Default value
   - NOT NULL constraint
   - Check constraints

2. **Domain usage tracking**: Which columns use the domain.

**Catalog queries needed:**
- `pg_type` where `typtype = 'd'`: Domain types
- `pg_constraint` for domain constraints

### Composite Types

User-defined composite types (beyond table row types).

**What would need to be added:**

1. **Composite type representation**:
   - Name and schema
   - Attributes (like columns but for types)

**Catalog queries needed:**
- `pg_type` where `typtype = 'c'`: Composite types
- `pg_attribute` for type attributes

### Tablespaces

PostgreSQL tablespaces control physical storage location.

**What would need to be added:**

1. **Tablespace tracking**: Which tablespace tables and indexes use.

2. **Default tablespace**: Database and schema defaults.

**Note**: Tablespace management often requires superuser privileges and may be environment-specific (dev vs prod).
