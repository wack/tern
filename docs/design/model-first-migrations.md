# Design Document: Model-First Migrations

## Overview

This document describes the design for a "model-first" migration workflow in Tern, allowing users to define their desired database schema declaratively in SQL and have Tern automatically generate migrations by diffing the old and new schema definitions.

### Core Concept

The fundamental insight behind this feature is that a database schema can be represented in two equivalent ways:

1. **As a sequence of changes** (migrations): "Create table X, then add column Y, then add index Z"
2. **As a snapshot** (DDL): "The database has table X with columns A, B, C and index Z"

These representations are mathematically equivalent—applying all migrations to an empty database produces the snapshot, and diffing two snapshots produces the migrations needed to transform one into the other.

Currently, Tern works primarily with the first representation (migrations). This feature adds first-class support for the second representation (a schema file), enabling users to express their intent declaratively rather than imperatively.

### What is the Schema File?

The schema file (`.tern/schema.sql`) is a SQL DDL script that, if executed against an empty database, would produce exactly the schema that results from applying all migrations in sequence. It is:

- **A computed artifact**: Generated from the migration history, not manually maintained as a source of truth
- **Human-readable and editable**: Plain SQL that any database developer can understand and modify
- **The interface for schema changes**: Users edit this file to express desired changes, rather than writing migrations directly

Think of it as analogous to a "compiled" view of the migrations—similar to how a `package-lock.json` is derived from `package.json`, except here the schema file is the user-facing interface and migrations are the derived output.

## Motivation

### Current Workflow (Problem)

The current Tern workflow requires users to:

1. Apply existing migrations to a live, running database (typically a local development database)
2. Manually modify the database schema to reach the desired state (using `psql`, a GUI tool, or raw SQL)
3. Invoke Tern to capture the diff between the live schema and Tern's saved state
4. Tern generates a migration based on this diff

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Current Workflow (Problem)                           │
└─────────────────────────────────────────────────────────────────────────────┘

  Developer                     Local PostgreSQL                    Tern
  ─────────                     ────────────────                    ────
      │                               │                               │
      │  1. Start local database      │                               │
      │  ─────────────────────────►   │                               │
      │                               │                               │
      │  2. Apply existing migrations │                               │
      │  ─────────────────────────────────────────────────────────►   │
      │                               │   ◄──── execute SQL ────────  │
      │                               │                               │
      │  3. Manually alter schema     │                               │
      │  (psql, GUI, raw SQL)         │                               │
      │  ─────────────────────────►   │                               │
      │                               │                               │
      │  4. Capture diff              │                               │
      │  ─────────────────────────────────────────────────────────►   │
      │                               │   ◄──── introspect ─────────  │
      │                               │                               │
      │   ◄──── migration file ───────────────────────────────────    │
      │                               │                               │
```

This workflow is cumbersome because:

- **Redundant manual work**: Users manually perform schema changes (ALTER TABLE, CREATE INDEX, etc.) that Tern then reverse-engineers into migrations. The user is essentially doing Tern's job twice.
- **Requires a running database**: Users must install, configure, and maintain a local PostgreSQL instance just to make schema changes.
- **Error-prone**: Manual schema modifications via ad-hoc SQL commands can introduce inconsistencies, typos, or unintended changes that are then captured in migrations.
- **Poor developer experience**: The workflow is unintuitive for developers accustomed to Django, Rails, or similar frameworks where you edit a model definition and the framework generates migrations.
- **Context switching**: Developers must switch between their editor (for application code), a database client (for schema changes), and the command line (for Tern commands).

### Inspiration: Django's Model-First Approach

Django's migration system exemplifies the "model-first" pattern:

1. Developer edits Python model classes (e.g., adds a field to a Django model)
2. Developer runs `python manage.py makemigrations`
3. Django compares the current model definitions to the previous state
4. Django generates a migration file representing the diff

The key insight is that **the model definition is the source of truth**, and migrations are derived from changes to that definition. Developers think in terms of "what I want the schema to look like," not "what SQL commands to run."

### Proposed Workflow (Solution)

This feature brings Django-style model-first development to Tern:

1. Tern maintains a **schema file** (`.tern/schema.sql`) representing the current database schema as SQL DDL
2. User **edits the schema file** directly to express their desired schema changes
3. Tern **diffs the old schema against the new** to generate a migration
4. The schema file is **automatically regenerated** after migrations are applied

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         Proposed Workflow (Solution)                         │
└─────────────────────────────────────────────────────────────────────────────┘

  Developer                     Tern                           (No external DB!)
  ─────────                     ────                           ────────────────
      │                           │
      │  1. Edit schema.sql       │
      │  (add column, table, etc) │
      │                           │
      │  2. Run: tern schema diff │
      │  ────────────────────►    │
      │                           │  ┌─────────────────────────────────────┐
      │                           │  │  Internally:                        │
      │                           │  │  • Load old schema into PGLite #1   │
      │                           │  │  • Load new schema into PGLite #2   │
      │                           │  │  • Diff the two schemas             │
      │                           │  │  • Generate migration               │
      │                           │  └─────────────────────────────────────┘
      │                           │
      │   ◄── shows diff ─────────│
      │                           │
      │  3. Run: tern schema migrate
      │  ────────────────────►    │
      │                           │
      │   ◄── migration created ──│
      │                           │
```

This workflow provides:

- **Declarative schema definition**: Users express "what" they want, not "how" to get there. Instead of writing `ALTER TABLE users ADD COLUMN email VARCHAR(255)`, they simply add `email VARCHAR(255)` to the table definition in schema.sql.
- **Familiar interface**: SQL DDL is universally understood by database developers. There's no new DSL to learn—if you know PostgreSQL, you know how to use this feature.
- **No running database required** (for basic operations): Tern uses an embedded PGLite instance to execute and introspect schemas. Users don't need to install or manage PostgreSQL locally.
- **Natural version control**: While the schema file itself isn't committed (it's regenerated from migrations), the migrations it produces show clear, reviewable diffs in pull requests.
- **Single-tool workflow**: Developers stay in their editor. Edit schema.sql, run a Tern command, done.

### Example: Adding a Column

**Current workflow:**
```bash
# 1. Make sure local Postgres is running
docker start my-postgres

# 2. Apply existing migrations
tern apply

# 3. Manually alter the table
psql -d mydb -c "ALTER TABLE users ADD COLUMN email VARCHAR(255) NOT NULL"

# 4. Capture the change
tern compile --description "Add email to users"
```

**Proposed workflow:**
```bash
# 1. Edit .tern/schema.sql - find the users table and add the column:
#    CREATE TABLE users (
#      id SERIAL PRIMARY KEY,
#      name VARCHAR(100),
#      email VARCHAR(255) NOT NULL  -- ← add this line
#    );

# 2. Generate migration
tern schema migrate --description "Add email to users"
```

The proposed workflow is simpler, requires no external database, and keeps the developer in their editor.

## Architecture

### Conceptual Model

The architecture is built around a key abstraction: the **Namespace**. A `Namespace` is Tern's in-memory representation of a PostgreSQL schema, containing all tables, columns, constraints, indexes, views, sequences, and enums. Namespaces are:

- **Serializable**: Can be saved to JSON and loaded back (this is how `state.json` works)
- **Diffable**: Two Namespaces can be compared to produce a set of operations (migrations)
- **Renderable**: A Namespace can be converted to SQL DDL

The model-first workflow leverages these properties:

```
                    ┌─────────────────────────────────────────┐
                    │              Namespace                   │
                    │  (Tern's in-memory schema model)         │
                    └─────────────────────────────────────────┘
                           ▲                      │
                           │                      │
              ┌────────────┴────────────┐         │
              │                         │         │
        introspect                 deserialize    │  serialize
        (from DB)                  (from JSON)    │  (to JSON)
              │                         │         │
              ▲                         ▲         ▼
     ┌────────────────┐        ┌──────────────┐  ┌──────────────┐
     │   PostgreSQL   │        │  state.json  │  │  state.json  │
     │   (live DB)    │        │   (input)    │  │  (output)    │
     └────────────────┘        └──────────────┘  └──────────────┘

                                      │
                                      │  render (to SQL)
                                      ▼
                              ┌──────────────┐
                              │  schema.sql  │
                              │  (DDL text)  │
                              └──────────────┘
```

**The challenge**: While we can easily convert a Namespace to SQL (rendering), we cannot easily convert SQL back to a Namespace without executing it. SQL is a complex language with many syntactic variations, and parsing it reliably would require a full PostgreSQL-compatible parser.

**The solution**: Execute the SQL in an embedded PostgreSQL instance (PGLite), then introspect the resulting schema using the same catalog queries we use for live databases. This reuses existing infrastructure and guarantees compatibility with any valid PostgreSQL DDL.

### How Migration Generation Works

When a user edits the schema file and runs `tern schema migrate`, the following process occurs:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    Migration Generation Process                              │
└─────────────────────────────────────────────────────────────────────────────┘

  Step 1: Determine Source State
  ──────────────────────────────
  The "source" is the current schema state before the user's edits.
  This is already stored in state.json (or can be reconstructed from migrations).

       ┌──────────────┐                    ┌──────────────┐
       │  state.json  │  ───deserialize──► │  Namespace   │
       │              │                    │  (source)    │
       └──────────────┘                    └──────────────┘


  Step 2: Determine Target State
  ──────────────────────────────
  The "target" is the desired schema state after the user's edits.
  We obtain this by executing the edited schema.sql in PGLite and introspecting.

       ┌──────────────┐                    ┌──────────────┐
       │  schema.sql  │  ───execute──────► │   PGLite     │
       │  (edited)    │                    │  (in-memory) │
       └──────────────┘                    └──────┬───────┘
                                                  │
                                                  │ introspect
                                                  ▼
                                           ┌──────────────┐
                                           │  Namespace   │
                                           │  (target)    │
                                           └──────────────┘


  Step 3: Generate Migration
  ──────────────────────────
  Diff the source and target Namespaces to produce operations.

       ┌──────────────┐         ┌──────────────┐
       │  Namespace   │         │  Namespace   │
       │  (source)    │         │  (target)    │
       └──────┬───────┘         └──────┬───────┘
              │                        │
              └───────────┬────────────┘
                          │ diff_namespaces()
                          ▼
                   ┌──────────────┐
                   │ NamespaceDiff │
                   │ • added       │
                   │ • removed     │
                   │ • modified    │
                   └──────┬───────┘
                          │ MigrationPlan::from_diff()
                          ▼
                   ┌──────────────┐
                   │  Operations  │
                   │ • CreateTable│
                   │ • AddColumn  │
                   │ • DropIndex  │
                   │ • ...        │
                   └──────┬───────┘
                          │ PostgresRenderer
                          ▼
                   ┌──────────────┐
                   │  Migration   │
                   │  (SQL + ops) │
                   └──────────────┘
```

### High-Level Flow Diagram

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

**Note on the diagram**: In practice, we may not need to execute the "old" schema.sql in PGLite—we can simply load the source Namespace directly from `state.json`. The diagram shows the conceptual flow; the implementation optimizes by reusing cached state.

### Key Components

#### 1. Schema File Management

The schema file represents the complete database DDL—the instructions to build the entire database schema from scratch. It can exist in two forms:

| Mode | Path | Description |
|------|------|-------------|
| Single file | `.tern/schema.sql` | All DDL in one file |
| Multi-file | `.tern/schema/*.sql` | DDL split across multiple files |

**Detection logic**: If `.tern/schema.sql` exists, use single-file mode. Otherwise, if `.tern/schema/` directory exists, use multi-file mode.

**Single-file mode** is simpler and suitable for most projects. The entire schema is in one file, making it easy to search and navigate.

**Multi-file mode** is useful for large databases where a single file becomes unwieldy. Users can organize their schema logically (e.g., one file per table, or grouped by domain). The trade-off is increased complexity in dependency management.

**Example single-file schema:**
```sql
-- .tern/schema.sql

-- Enum types
CREATE TYPE user_status AS ENUM ('active', 'inactive', 'suspended');

-- Tables
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email VARCHAR(255) NOT NULL UNIQUE,
    status user_status NOT NULL DEFAULT 'active',
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

CREATE TABLE posts (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    body TEXT,
    published_at TIMESTAMP WITH TIME ZONE
);

-- Indexes
CREATE INDEX idx_posts_user_id ON posts(user_id);
CREATE INDEX idx_posts_published_at ON posts(published_at) WHERE published_at IS NOT NULL;
```

**Example multi-file schema:**
```
.tern/schema/
├── 00_types.sql      # Enum types and domains
├── 01_users.sql      # Users table
├── 02_posts.sql      # Posts table (references users)
└── 03_indexes.sql    # Additional indexes
```

#### 2. Schema Exporter

The Schema Exporter converts a `Namespace` (Tern's in-memory schema representation) to SQL DDL. This is how we generate the schema.sql file from the current state.

**The key insight**: Exporting a schema to DDL is equivalent to asking "what SQL would I need to run to create this schema from scratch?" This is exactly what a migration from an empty database to the current state would look like.

```
Namespace → NamespaceDiff (vs empty) → MigrationPlan → PostgresRenderer → SQL
```

This reuses existing infrastructure:
- `diff_namespaces(&Namespace::empty(), &current)` produces a diff where everything is "added"
- `MigrationPlan::from_diff()` converts the diff to semantic operations
- `PostgresRenderer` renders operations as SQL DDL

**Output ordering**: The exporter produces DDL in dependency order:
1. Enum types and domains (no dependencies)
2. Sequences (no dependencies)
3. Tables (may reference enums, sequences)
4. Foreign key constraints (reference other tables)
5. Indexes (reference tables)
6. Views (may reference tables, other views)
7. Comments (reference any object)

This ordering ensures the schema.sql can be executed top-to-bottom without dependency errors.

#### 3. PGLite Integration

PGLite is PostgreSQL compiled to WebAssembly, allowing us to run a real PostgreSQL instance entirely in-memory without any external dependencies. This is the cornerstone of the model-first workflow.

**Why PGLite is necessary:**

The challenge with using SQL as the schema format is that we need to convert SQL back into a `Namespace` for diffing. There are two approaches:

1. **Parse the SQL directly**: Build or use a SQL parser to extract schema information from DDL statements. This is complex because PostgreSQL's SQL syntax is vast and has many variations. A custom parser would need to handle all CREATE TABLE variants, column constraints, expressions, etc.

2. **Execute the SQL and introspect**: Run the SQL in a real PostgreSQL instance, then query the system catalogs (`pg_class`, `pg_attribute`, `pg_constraint`, etc.) to extract schema information. This is what Tern already does for live databases.

PGLite enables option 2 without requiring users to manage an external database. It provides:

- **Full PostgreSQL compatibility**: PGLite runs actual PostgreSQL code, so any valid PostgreSQL DDL will work
- **Sandboxed execution**: Each schema load runs in a fresh, isolated instance—no risk of conflicting with user data
- **No installation required**: PGLite is embedded in Tern; users don't need to install PostgreSQL locally
- **Fast startup**: In-memory instances start in milliseconds

**Responsibilities:**
- Execute SQL DDL statements from schema files
- Provide a queryable catalog for schema introspection via the existing `Catalog` trait
- Validate that user-edited SQL is syntactically and semantically correct

**Fallback**: Users can configure a real PostgreSQL connection for full compatibility when PGLite's limitations are encountered (e.g., when using extensions like PostGIS that aren't available in PGLite).

#### 4. Worklist Executor

When using multi-file mode, SQL files may have dependencies on each other. For example, a `posts.sql` file that creates a table with a foreign key to `users` cannot be executed until `users.sql` has been executed.

**The problem**: We need to determine the correct execution order for SQL files, but we don't want to parse SQL to detect dependencies.

**The solution**: Use a worklist (retry queue) algorithm that leverages PostgreSQL's own error messages to detect missing dependencies. The algorithm attempts to execute each file; if it fails due to a missing dependency, the file is moved to the back of the queue to be retried later.

**Why this approach?**

1. **No SQL parsing required**: We don't need to understand SQL syntax to detect dependencies
2. **Handles all dependency types**: Foreign keys, sequences, functions, types, triggers—all handled uniformly
3. **PostgreSQL is the authority**: PostgreSQL itself tells us when dependencies are missing, so we can't miss any
4. **Correctly detects circular dependencies**: If we complete a full rotation through the queue without making progress, there's a genuine circular dependency that no ordering can resolve

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

After executing DDL in PGLite, we need to extract the resulting schema as a `Namespace`. This is done using Tern's existing schema introspection infrastructure.

Tern already has a `Catalog` trait that abstracts database queries for schema introspection. The `PostgresCatalog` implementation queries PostgreSQL's system catalogs (`pg_class`, `pg_attribute`, `pg_constraint`, etc.) to load complete schema information.

For PGLite integration, we implement `PgLiteCatalog` with the same interface. Since PGLite runs real PostgreSQL, the same SQL queries work unchanged.

```rust
// Existing infrastructure, reused with a new Catalog implementation
let catalog = PgLiteCatalog::new(pglite_connection);
let namespace = load_namespace(&catalog, "public").await?;
```

**Key benefit**: By reusing the existing `Catalog` trait and `load_namespace()` function, we guarantee that schema introspection from PGLite produces exactly the same `Namespace` structure as introspection from a real PostgreSQL database. This ensures the diff and migration generation work correctly regardless of the source.

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
