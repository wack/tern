# Design Document: Model-First Migrations

## Overview

This document describes the design for a "model-first" migration workflow in Tern, allowing users to define their desired database schema declaratively in SQL and have Tern automatically generate migrations by diffing the old and new schema definitions.

## Motivation

### Current Workflow (Problem)

The current Tern workflow requires users to:

1. Apply existing migrations to a live, running database (typically a local development database)
2. Manually modify the database schema to reach the desired state (using `psql`, a GUI tool, or raw SQL)
3. Invoke Tern to capture the diff between the live schema and Tern's saved state
4. Tern generates a migration based on this diff

This workflow is cumbersome because:

- **Redundant manual work**: Users manually perform schema changes that Tern then reverse-engineers into migrations
- **Requires a running database**: Users must maintain a local PostgreSQL instance
- **Error-prone**: Manual schema modifications can introduce inconsistencies
- **Poor developer experience**: The workflow is unintuitive for developers accustomed to Django, Rails, or similar frameworks

### Proposed Workflow (Solution)

Inspired by Django's "model-first" approach:

1. Tern maintains a **schema file** (`.tern/schema.sql`) representing the current database schema as SQL DDL
2. User **edits the schema file** directly to express their desired schema changes
3. Tern **diffs the old schema against the new** to generate a migration
4. The schema file is **automatically regenerated** after migrations are applied

This workflow provides:

- **Declarative schema definition**: Users express "what" they want, not "how" to get there
- **Familiar interface**: SQL DDL is universally understood by database developers
- **No running database required** (for basic operations): Tern uses an embedded PGLite instance
- **Natural version control**: Schema changes are visible as SQL diffs in pull requests

## Architecture

### High-Level Flow

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           Model-First Migration Flow                         │
└─────────────────────────────────────────────────────────────────────────────┘

  ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
  │  Migrations  │         │ schema.sql   │         │ schema.sql   │
  │   (stored)   │         │    (old)     │         │   (edited)   │
  └──────┬───────┘         └──────┬───────┘         └──────┬───────┘
         │                        │                        │
         │ replay                 │ execute                │ execute
         ▼                        ▼                        ▼
  ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
  │  Namespace   │ ──────► │   PGLite     │         │   PGLite     │
  │   (state)    │ render  │  (source)    │         │  (target)    │
  └──────────────┘         └──────┬───────┘         └──────┬───────┘
                                  │                        │
                                  │ introspect             │ introspect
                                  ▼                        ▼
                           ┌──────────────┐         ┌──────────────┐
                           │  Namespace   │         │  Namespace   │
                           │   (source)   │         │  (target)    │
                           └──────┬───────┘         └──────┬───────┘
                                  │                        │
                                  └───────────┬────────────┘
                                              │ diff
                                              ▼
                                       ┌──────────────┐
                                       │  Migration   │
                                       │ (operations) │
                                       └──────────────┘
```

### Key Components

#### 1. Schema File Management

The schema file represents the complete database DDL and can exist in two forms:

| Mode | Path | Description |
|------|------|-------------|
| Single file | `.tern/schema.sql` | All DDL in one file |
| Multi-file | `.tern/schema/*.sql` | DDL split across multiple files |

**Detection logic**: If `.tern/schema.sql` exists, use single-file mode. Otherwise, if `.tern/schema/` directory exists, use multi-file mode.

#### 2. Schema Exporter

Converts a `Namespace` (Tern's in-memory schema representation) to SQL DDL.

```
Namespace → NamespaceDiff (vs empty) → MigrationPlan → PostgresRenderer → SQL
```

This reuses existing infrastructure:
- `diff_namespaces(&Namespace::empty(), &current)` produces a diff where everything is "added"
- `MigrationPlan::from_diff()` converts to operations
- `PostgresRenderer` renders operations as SQL DDL

#### 3. PGLite Integration

An embedded PostgreSQL instance (via PGLite/WASM) for executing DDL without requiring an external database.

**Responsibilities:**
- Execute SQL DDL statements
- Provide a queryable catalog for schema introspection
- Support the existing `Catalog` trait interface

**Fallback**: Users can configure a real PostgreSQL connection for full compatibility when PGLite's limitations are encountered.

#### 4. Worklist Executor

Handles dependency ordering when executing multiple SQL files (multi-file mode).

**Algorithm:**
```
function execute_schema_files(files: Vec<SqlFile>) -> Result<(), Error>:
    queue = Queue::from(files)  // arbitrary initial order
    items_visited_since_last_removal = 0

    while !queue.is_empty():
        file = queue.pop_front()

        match execute(file):
            Ok(_) =>
                items_visited_since_last_removal = 0
                // file successfully executed, don't re-add

            Err(e) if is_dependency_error(e) =>
                items_visited_since_last_removal += 1
                if items_visited_since_last_removal == queue.len() + 1:
                    return Err("Circular dependency detected")
                queue.push_back(file)  // retry later

            Err(e) =>
                return Err(e)  // genuine error, abort

    Ok(())
```

**Error categorization:**

| PostgreSQL Error Code | Meaning | Action |
|----------------------|---------|--------|
| `42P01` | undefined_table | Retry (dependency not yet created) |
| `42P06` | duplicate_schema | Skip (already exists) |
| `42P07` | duplicate_table | Skip (already exists) |
| `42883` | undefined_function | Retry |
| `42704` | undefined_object | Retry |
| `42710` | duplicate_object | Skip |
| Other | Syntax error, constraint violation, etc. | Abort with error |

#### 5. Schema Introspector

After executing DDL in PGLite, introspect the resulting schema using the existing `Catalog` trait and `load_namespace()` function.

```rust
// Existing infrastructure, reused
let catalog = PgLiteCatalog::new(pglite_connection);
let namespace = load_namespace(&catalog, "public").await?;
```

### Integration with Existing Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Tern Architecture                               │
└─────────────────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────────────┐
  │                            CLI Layer (src/cli/)                          │
  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌─────────────────────┐ │
  │  │  init   │ │ status  │ │ compile │ │ history │ │ schema export/diff  │ │
  │  └─────────┘ └─────────┘ └─────────┘ └─────────┘ └─────────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                         State Backend (src/db/state/)                    │
  │  ┌───────────────────┐  ┌───────────────────┐  ┌─────────────────────┐  │
  │  │ LocalFileBackend  │  │  SchemaExporter   │  │  SchemaLoader       │  │
  │  │ (migrations)      │  │  (Namespace→SQL)  │  │  (SQL→PGLite→NS)    │  │
  │  └───────────────────┘  └───────────────────┘  └─────────────────────┘  │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                          Database Layer (src/db/)                        │
  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
  │  │    model/   │  │    diff/    │  │   migrate/  │  │     query/      │ │
  │  │ (Namespace) │  │ (diffing)   │  │ (operations)│  │ (Catalog trait) │ │
  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                         Execution Layer (new)                            │
  │  ┌───────────────────────────────┐  ┌─────────────────────────────────┐ │
  │  │          PGLite               │  │      PostgresCatalog            │ │
  │  │   (embedded PostgreSQL)       │  │   (real PostgreSQL fallback)    │ │
  │  └───────────────────────────────┘  └─────────────────────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Schema Export

**Goal**: Generate `.tern/schema.sql` from the current state.

**Tasks**:
1. Add `SchemaExporter` that converts `Namespace` → SQL DDL
2. Extend `LocalFileBackend` with `export_schema()` method
3. Auto-generate schema file when migrations are applied
4. Add CLI command `tern schema export` for manual regeneration

**Key code paths**:
```rust
// In state backend
impl LocalFileBackend {
    pub async fn export_schema(&self) -> Result<(), Error> {
        let namespace = self.load_state().await?;
        let sql = SchemaExporter::export(&namespace)?;
        fs::write(".tern/schema.sql", sql)?;
        Ok(())
    }
}

// Schema exporter
pub struct SchemaExporter;

impl SchemaExporter {
    pub fn export(namespace: &Namespace) -> Result<String, Error> {
        let empty = Namespace::empty(namespace.name.clone());
        let diff = diff_namespaces(&empty, namespace);
        let plan = MigrationPlan::from_diff(&diff);
        let renderer = PostgresRenderer::new(RenderConfig::default());
        Ok(plan.render(&renderer).to_sql())
    }
}
```

### Phase 2: PGLite Integration

**Goal**: Execute SQL DDL in an embedded PostgreSQL environment.

**Tasks**:
1. Research PGLite Rust/WASM integration options
2. Implement `PgLiteCatalog` implementing the `Catalog` trait
3. Add worklist executor for multi-file DDL execution
4. Implement error categorization for retry logic

**Key code paths**:
```rust
// PGLite catalog implementation
pub struct PgLiteCatalog {
    connection: PgLiteConnection,
}

#[async_trait]
impl Catalog for PgLiteCatalog {
    async fn get_namespace(&self, name: &str) -> Result<NamespaceInfo, Error> {
        // Query pg_namespace via PGLite
    }

    async fn get_tables(&self, namespace_oid: Oid) -> Result<Vec<TableInfo>, Error> {
        // Query pg_class via PGLite
    }

    // ... other Catalog methods
}

// Worklist executor
pub struct WorklistExecutor {
    catalog: PgLiteCatalog,
}

impl WorklistExecutor {
    pub async fn execute_files(&self, files: Vec<PathBuf>) -> Result<(), Error> {
        let mut queue: VecDeque<_> = files.into();
        let mut visits_since_removal = 0;

        while let Some(file) = queue.pop_front() {
            let sql = fs::read_to_string(&file)?;

            match self.catalog.execute(&sql).await {
                Ok(_) => {
                    visits_since_removal = 0;
                }
                Err(e) if Self::is_dependency_error(&e) => {
                    visits_since_removal += 1;
                    if visits_since_removal > queue.len() {
                        return Err(Error::CircularDependency);
                    }
                    queue.push_back(file);
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }

    fn is_dependency_error(e: &PgError) -> bool {
        matches!(
            e.code(),
            Some("42P01") | // undefined_table
            Some("42883") | // undefined_function
            Some("42704")   // undefined_object
        )
    }
}
```

### Phase 3: Model-First Migration Generation

**Goal**: Generate migrations by diffing old and new schema files.

**Tasks**:
1. Add `SchemaLoader` that executes SQL in PGLite and introspects result
2. Implement full migration generation workflow
3. Add CLI command `tern schema diff` or extend `tern compile`
4. Handle destructive change warnings

**Key code paths**:
```rust
// Schema loader
pub struct SchemaLoader {
    executor: WorklistExecutor,
}

impl SchemaLoader {
    pub async fn load(&self, schema_path: &Path) -> Result<Namespace, Error> {
        // Create fresh PGLite instance
        let pglite = PgLite::new_in_memory()?;

        // Execute schema DDL
        if schema_path.is_file() {
            let sql = fs::read_to_string(schema_path)?;
            pglite.execute(&sql).await?;
        } else if schema_path.is_dir() {
            let files = glob::glob(&format!("{}/*.sql", schema_path.display()))?;
            self.executor.execute_files(files.collect()).await?;
        }

        // Introspect resulting schema
        let catalog = PgLiteCatalog::new(pglite);
        load_namespace(&catalog, "public").await
    }
}

// Migration generation
pub async fn generate_migration(
    backend: &LocalFileBackend,
    edited_schema_path: &Path,
) -> Result<Migration, Error> {
    // Load source (current state from migrations)
    let source = backend.load_state().await?;

    // Load target (edited schema file)
    let loader = SchemaLoader::new();
    let target = loader.load(edited_schema_path).await?;

    // Generate diff and migration
    let diff = diff_namespaces(&source, &target);
    let plan = MigrationPlan::from_diff(&diff);

    // Warn about destructive changes
    if !plan.breaking_changes.is_empty() {
        warn_destructive_changes(&plan.breaking_changes)?;
    }

    Ok(Migration::new(
        "Auto-generated from schema changes",
        plan.operations,
        source.hash(),
        target.hash(),
        plan.breaking_changes,
    ))
}
```

### Phase 4: Real PostgreSQL Fallback

**Goal**: Allow users to use a real PostgreSQL instance when PGLite is insufficient.

**Tasks**:
1. Add configuration option for PostgreSQL connection
2. Implement execution backend abstraction
3. Document when fallback is needed (extensions, advanced features)

**Configuration** (in `.tern/config.toml`):
```toml
[schema]
# Use embedded PGLite (default)
executor = "pglite"

# Or use external PostgreSQL
# executor = "postgres"
# connection = "postgresql://localhost/tern_sandbox"
```

## File Structure

### Single-File Mode

```
.tern/
├── config.toml           # Optional configuration
├── state.json            # Current Namespace (internal)
├── schema.sql            # User-editable schema DDL ← NEW
└── migrations/
    ├── index.json
    ├── 00001.json
    └── ...
```

### Multi-File Mode

```
.tern/
├── config.toml
├── state.json
├── schema/               # User-editable schema DDL ← NEW
│   ├── enums.sql         # Enum type definitions
│   ├── users.sql         # Users table
│   ├── orders.sql        # Orders table (FK to users)
│   └── ...
└── migrations/
    ├── index.json
    └── ...
```

## CLI Commands

### New Commands

```bash
# Export current state as schema.sql (manual regeneration)
tern schema export

# Show diff between current state and edited schema.sql
tern schema diff

# Generate migration from schema changes (alternative to compile)
tern schema migrate --description "Add user preferences"
```

### Modified Commands

```bash
# After applying migrations, schema.sql is automatically regenerated
tern apply
# → Applies migrations
# → Regenerates .tern/schema.sql
```

## Design Decisions

### Decision 1: SQL Format (Not JSON or DSL)

**Choice**: The schema file is plain SQL DDL.

**Rationale**:
- SQL is universally understood by database developers
- No learning curve for new users
- Standard tooling (formatters, linters, syntax highlighting) works out of the box
- Lower barrier to adoption compared to custom DSLs

**Trade-off**: Requires executing SQL to parse it back into a `Namespace`, rather than simple deserialization. This necessitates PGLite integration.

### Decision 2: PGLite for SQL Execution

**Choice**: Use PGLite (embedded PostgreSQL via WASM) as the primary execution environment.

**Rationale**:
- No external database required for basic operations
- Sandboxed execution prevents conflicts with user's local databases
- Tern already has Wasmtime as a dependency, reducing integration friction

**Trade-off**: PGLite may have PostgreSQL compatibility gaps. Mitigated by offering real PostgreSQL as a fallback.

### Decision 3: Worklist Algorithm for Dependency Resolution

**Choice**: Use a retry-based worklist algorithm rather than parsing SQL for dependencies.

**Rationale**:
- No SQL parsing required (beyond statement execution)
- Handles all dependency types uniformly (FKs, functions, types, sequences)
- PostgreSQL itself determines what's a valid execution order
- Correctly detects genuinely circular dependencies

**Trade-off**: O(n²) worst-case complexity for linear dependency chains. Acceptable because n (number of schema objects) is typically small.

### Decision 4: File-Level Execution Granularity

**Choice**: In multi-file mode, each `.sql` file is one execution unit.

**Rationale**:
- Avoids the complexity of parsing SQL to split statements
- Semicolons inside dollar-quoted strings (functions, procedures) make naive splitting unreliable
- Proper statement splitting would require a full SQL lexer or external dependency

**Trade-off**: Users must organize files such that each file can be executed atomically. Document as best practice: "one logical unit per file."

### Decision 5: Schema File Not Committed to Version Control

**Choice**: `.tern/schema.sql` should be in `.gitignore`.

**Rationale**:
- Can be deterministically regenerated from migrations
- Avoids merge conflicts when multiple developers create migrations
- Migrations remain the source of truth

**Trade-off**: Schema changes aren't directly visible in PRs. Mitigated by good migration descriptions and the ability to run `tern schema diff` locally.

### Decision 6: Automatic Regeneration on Migration Apply

**Choice**: Schema file is regenerated automatically when migrations are applied.

**Rationale**:
- Keeps schema file in sync with migration history
- No manual step required
- Ensures schema file always reflects current state

**Future consideration**: For remote backends, a `tern schema refresh` command may be needed.

## Edge Cases and Error Handling

### Circular Foreign Key References

PostgreSQL supports circular FK references, but they require either deferrable constraints or two-phase creation (CREATE TABLE without FK, then ALTER TABLE ADD CONSTRAINT).

If a user writes inline circular FKs, the worklist algorithm will detect this as unsatisfiable and report:

```
Error: Circular dependency detected among schema files.

The following files could not be executed in any order:
  - schema/orders.sql (requires: users)
  - schema/users.sql (requires: orders)

Hint: For circular foreign key references, use ALTER TABLE ADD CONSTRAINT
in a separate file that runs after both tables are created.
```

### PGLite Compatibility Gaps

When PGLite doesn't support a PostgreSQL feature (e.g., certain extensions):

```
Error: PGLite execution failed: extension "postgis" is not available

Hint: Configure a real PostgreSQL connection for full compatibility:

  In .tern/config.toml:
  [schema]
  executor = "postgres"
  connection = "postgresql://localhost/tern_sandbox"
```

### Destructive Changes

When the schema diff includes destructive operations (DROP TABLE, DROP COLUMN):

```
Warning: The following destructive changes were detected:

  - DROP COLUMN users.legacy_field
  - DROP TABLE deprecated_logs

These changes will result in data loss. Continue? [y/N]
```

## Future Enhancements

### Schema Validation

Add `tern schema validate` to check schema.sql for:
- SQL syntax errors
- Tern model compatibility
- Best practice violations

### Schema Formatting

Add `tern schema fmt` to normalize schema.sql formatting:
- Consistent indentation
- Alphabetized columns/constraints
- Standardized quoting

### Interactive Schema Editor

A TUI or web-based editor for schema modifications with:
- Autocomplete for types and references
- Real-time validation
- Visual diff preview

### Migration Squashing

Combine multiple migrations into one, updating schema.sql to match:
```bash
tern migrate squash --from 00001 --to 00010
```

## Appendix: PGLite Integration Research

### What is PGLite?

PGLite is PostgreSQL compiled to WebAssembly, runnable in browsers and Node.js. Key characteristics:

- **Full PostgreSQL**: Not an emulator; actual PostgreSQL compiled to WASM
- **In-memory or persistent**: Can use in-memory storage or IndexedDB/filesystem
- **Extensions**: Limited extension support (some built-in, no native extensions)

### Rust Integration Options

1. **pglite-rs**: Direct Rust bindings (if available)
2. **wasm-bindgen**: Call PGLite's JavaScript API from Rust/WASM
3. **wasmtime**: Run PGLite's WASM module directly (Tern already uses wasmtime)

### Recommended Approach

Use wasmtime to instantiate PGLite's WASM module, providing:
- Pure Rust solution (no JavaScript runtime)
- Leverages existing wasmtime dependency
- Full control over memory and execution

### PGLite Limitations

| Feature | PGLite Support |
|---------|---------------|
| Basic DDL (CREATE TABLE, etc.) | ✅ Full |
| Foreign keys, constraints | ✅ Full |
| Indexes | ✅ Full |
| Functions (PL/pgSQL) | ✅ Full |
| Triggers | ✅ Full |
| Extensions (PostGIS, etc.) | ❌ Limited |
| Full-text search | ✅ Built-in |
| Large objects | ⚠️ Partial |

For unsupported features, users should configure the real PostgreSQL fallback.
