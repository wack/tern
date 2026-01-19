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
├── lib.rs              # Library root (exports cli module)
├── bin/
│   └── main.rs         # Binary entry point
└── cli/
    ├── mod.rs          # CLI command definitions using clap
    └── colors.rs       # Color output handling (Auto/Always/Never)
```

## Architecture

- **CLI Framework**: Uses `clap` with derive macros for argument parsing
- **Async Runtime**: `tokio` with full features
- **Error Handling**: `miette` for user-facing diagnostics, `thiserror` for error type definitions
- **Logging**: `tracing` with `tracing-subscriber` (supports text and JSON formats)
- **Serialization**: `serde` for data serialization

## Code Style & Conventions

- Rust Edition 2024
- Code must pass `rustfmt` formatting checks
- Code must pass `clippy` linting with no warnings
- Use `thiserror` for defining error types
- Use `miette` for rich error diagnostics
- Prefer async/await patterns with tokio

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

The current rename detection uses basic structural similarity (column name overlap, type matches). More sophisticated algorithms could improve accuracy.

**Potential enhancements:**

1. **Weighted column matching**: Consider column importance:
   - Primary key columns should weigh more heavily
   - Columns referenced by foreign keys are more significant
   - Columns with unique constraints indicate structural importance

2. **Constraint graph analysis**: Tables with similar foreign key relationships to other tables are more likely to be renames:
   - If `users` references `accounts` and a new table `members` also references `accounts` with similar FK structure, that's a strong signal

3. **Index structure comparison**: Similar index definitions (same columns, same method) indicate structural similarity beyond just columns.

4. **Historical tracking**: Store previous schema snapshots to detect rename patterns over time (e.g., if a table was renamed before, similar patterns might indicate another rename).

5. **Name similarity heuristics**: Use string similarity metrics (Levenshtein distance, common prefixes/suffixes) as a tie-breaker:
   - `user_accounts` → `accounts` (common suffix)
   - `tbl_users` → `users` (common base name)

6. **Machine learning approach**: Train a model on known rename operations to predict renames based on structural features.

**Configuration options to add:**
```rust
pub struct RenameDetectionConfig {
    /// Minimum structural similarity (0.0-1.0)
    pub similarity_threshold: f64,
    /// Weight for column name overlap
    pub column_name_weight: f64,
    /// Weight for type matches
    pub type_match_weight: f64,
    /// Weight for constraint similarity
    pub constraint_weight: f64,
    /// Weight for name similarity
    pub name_similarity_weight: f64,
    /// Whether to consider position when matching columns
    pub consider_position: bool,
}
```

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
