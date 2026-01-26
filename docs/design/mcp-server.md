# Design Document: MCP Server for Tern

## Overview

This document describes the design for adding a Model Context Protocol (MCP) server to Tern. The MCP server enables AI assistants and other MCP clients to interact with Tern's migration system programmatically, allowing them to:

1. Inspect the current database schema (via MCP resources)
2. Create migrations through a session-based workflow (via MCP tools)
3. Apply schema changes using either structured operations or raw SQL

The server runs as a subcommand (`tern mcp`) and operates entirely locally, using an in-memory PGLite database for schema validation.

## Goals

1. **Schema Inspection**: Expose the current database schema as an MCP resource
2. **Interactive Migration Authoring**: Allow users to create migrations through a session-based workflow
3. **Flexibility**: Support both structured operations and raw SQL for schema changes
4. **Local-First**: Run entirely on the user's machine with an in-memory PostgreSQL database
5. **Safety**: Validate all changes against an in-memory database before writing migrations

## Non-Goals

1. Executing migrations against production databases (use existing `tern up`/`tern down`)
2. Direct database connections to remote PostgreSQL instances
3. Multi-user concurrent access (single-user local tool)
4. Network-based transport (stdio only for initial implementation)

---

## Architecture

### High-Level Components

```
┌─────────────────────────────────────────────────────────────────────┐
│                      MCP Client (AI Assistant)                      │
└─────────────────────────────────┬───────────────────────────────────┘
                                  │ JSON-RPC over stdio
                                  ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        MCP Server (tern mcp)                        │
│                                                                     │
│  ┌─────────────────┐  ┌─────────────────┐  ┌─────────────────────┐ │
│  │    Resources    │  │      Tools      │  │   Session Manager   │ │
│  │  ─────────────  │  │  ────────────   │  │  ─────────────────  │ │
│  │  • schema       │  │  • start_session│  │  • session map      │ │
│  │  • migrations   │  │  • execute_sql  │  │  • PGLite instances │ │
│  │  • migration/id │  │  • apply_op     │  │  • lifecycle mgmt   │ │
│  │                 │  │  • get_diff     │  │                     │ │
│  │                 │  │  • generate     │  │                     │ │
│  │                 │  │  • cancel       │  │                     │ │
│  └─────────────────┘  └─────────────────┘  └─────────────────────┘ │
└─────────────────────────────────┬───────────────────────────────────┘
                                  │
         ┌────────────────────────┼────────────────────────┐
         ▼                        ▼                        ▼
┌─────────────────┐    ┌─────────────────────┐    ┌───────────────────┐
│ LocalFileBackend│    │   Base PGLite       │    │ Session PGLite(s) │
│ (.tern folder)  │    │   (read-only)       │    │ (per-session)     │
│                 │    │                     │    │                   │
│ • state.json    │    │ • Current schema    │    │ • Cloned from     │
│ • migrations/   │    │ • All migrations    │    │   base state      │
│ • schema.sql    │    │   applied           │    │ • Tracks changes  │
└─────────────────┘    └─────────────────────┘    └───────────────────┘
```

### Component Responsibilities

**MCP Server**: Entry point, handles JSON-RPC protocol over stdio, routes requests to handlers

**Resources**: Read-only data exposed to the MCP client
- `tern://schema` — Current schema state after all migrations
- `tern://migrations` — List of all migrations with metadata
- `tern://migration/{id}` — Details of a specific migration

**Tools**: Actions that modify state
- `start_session` — Begin a migration authoring session, returns session ID
- `execute_sql` — Execute arbitrary SQL against session database
- `apply_operation` — Apply a structured schema operation
- `get_session_schema` — Get current schema state within a session
- `get_session_diff` — Get diff between base state and session state
- `generate_migration` — Finalize session and create migration file
- `cancel_session` — Discard session without saving
- `list_sessions` — List active sessions

**Session Manager**: Maintains active editing sessions
- Maps session IDs to PGLite instances
- Tracks schema changes per session
- Handles session lifecycle (create, clone, destroy)

**LocalFileBackend**: Existing Tern state management (reused)
- Reads `.tern/` folder structure
- Loads migration history and current state
- Saves new migrations

**PGLite Instances**:
- **Base instance**: Initialized at server startup with all migrations applied; used for read-only schema resource queries
- **Session instances**: Created per-session; cloned from base state; used for interactive editing

---

## Server Initialization

When `tern mcp` starts, it performs the following initialization sequence:

```
tern mcp
    │
    ├─1─► Locate .tern/ directory
    │     └── Error if not found: "No .tern directory found. Run 'tern init' first."
    │
    ├─2─► Load LocalFileBackend
    │     └── Verify state.json exists and is valid
    │
    ├─3─► Load migration index
    │     └── Parse .tern/migrations/index.json
    │
    ├─4─► Initialize base PGLite instance
    │     ├── Create in-memory PostgreSQL database
    │     └── Apply all migrations in order
    │
    ├─5─► Load base namespace
    │     └── Query PGLite catalog to build Namespace struct
    │
    ├─6─► Create SessionManager
    │     └── Initialize with base state reference
    │
    ├─7─► Initialize MCP Server
    │     ├── Register resources
    │     └── Register tools
    │
    └─8─► Start stdio transport
          └── Begin processing JSON-RPC messages
```

### Startup Errors

The server must fail with clear error messages for these conditions:

| Condition | Error Message |
|-----------|---------------|
| No `.tern/` directory | `"No .tern directory found at {path}. Run 'tern init' to initialize a new project."` |
| Invalid `state.json` | `"Invalid state file: {parse_error}. The .tern/state.json file may be corrupted."` |
| Missing migrations | `"Migration {id} referenced in index but file not found."` |
| Migration apply failure | `"Failed to apply migration {id}: {sql_error}"` |
| PGLite init failure | `"Failed to initialize PGLite database: {error}"` |

---

## PGLite Integration

### Technology Choice

PGLite is a WebAssembly build of PostgreSQL that runs in-process. For Rust integration, we have several options:

1. **pglite-rs** — Native Rust bindings to PGLite (if available)
2. **Wasmtime-based** — Run PGLite WASM in wasmtime (Tern already depends on wasmtime)
3. **pgtemp/embedded_postgres** — Lightweight embedded PostgreSQL alternative
4. **pglite via wasm-component** — Compose PGLite as a WASM component

**Recommended approach**: Evaluate `pglite` Rust bindings first. If unavailable or inadequate, leverage Tern's existing wasmtime infrastructure to run PGLite's WASM build directly.

### Database Lifecycle

```rust
/// Represents a PGLite database instance
pub struct PgLiteInstance {
    /// Connection handle to the in-memory database
    conn: PgLiteConnection,
    /// Schema name being managed (typically "public")
    schema_name: String,
}

impl PgLiteInstance {
    /// Create a new empty PGLite instance
    pub async fn new() -> Result<Self, PgLiteError>;

    /// Execute SQL statement(s)
    pub async fn execute(&self, sql: &str) -> Result<ExecuteResult, PgLiteError>;

    /// Query and return rows
    pub async fn query(&self, sql: &str) -> Result<Vec<Row>, PgLiteError>;

    /// Load schema by applying SQL DDL
    pub async fn apply_ddl(&self, ddl: &str) -> Result<(), PgLiteError>;

    /// Extract current schema as Namespace
    pub async fn get_namespace(&self) -> Result<Namespace, PgLiteError>;
}
```

### Base Instance Initialization

At server startup:

```rust
async fn initialize_base_database(
    backend: &LocalFileBackend,
) -> Result<(PgLiteInstance, Namespace), InitError> {
    // 1. Create empty PGLite instance
    let db = PgLiteInstance::new().await?;

    // 2. Load migration index
    let index = backend.get_migration_index().await?;

    // 3. Apply each migration in order
    for entry in index.migrations {
        let migration = backend.get_migration(&entry.id).await?;

        // Convert operations to SQL and execute
        for operation in &migration.up_operations {
            let sql = render_operation_to_sql(operation)?;
            db.execute(&sql).await.map_err(|e| {
                InitError::MigrationFailed {
                    migration_id: entry.id.clone(),
                    error: e,
                }
            })?;
        }
    }

    // 4. Extract final namespace
    let namespace = db.get_namespace().await?;

    Ok((db, namespace))
}
```

### Session Instance Creation

When a session starts:

```rust
async fn create_session_database(
    base_namespace: &Namespace,
) -> Result<PgLiteInstance, SessionError> {
    // 1. Create new PGLite instance
    let db = PgLiteInstance::new().await?;

    // 2. Generate DDL from base namespace
    let ddl = render_namespace_to_ddl(base_namespace)?;

    // 3. Apply DDL to bring to base state
    db.apply_ddl(&ddl).await?;

    Ok(db)
}
```

---

## MCP Resources

Resources provide read-only access to schema information. They are queried from the base PGLite instance (not session instances).

### Resource: `tern://schema`

Returns the current database schema as structured JSON.

**URI**: `tern://schema`

**Description**: The current database schema after all migrations have been applied.

**MIME Type**: `application/json`

**Content Structure** (mirrors `Namespace` type):

```json
{
  "name": "public",
  "tables": [
    {
      "name": "users",
      "columns": [
        {
          "name": "id",
          "type": "integer",
          "nullable": false,
          "default": null,
          "identity": {
            "type": "ALWAYS",
            "start": 1,
            "increment": 1
          }
        },
        {
          "name": "email",
          "type": "character varying(255)",
          "nullable": false,
          "default": null
        },
        {
          "name": "created_at",
          "type": "timestamp with time zone",
          "nullable": false,
          "default": "now()"
        }
      ],
      "primaryKey": {
        "name": "users_pkey",
        "columns": ["id"]
      },
      "foreignKeys": [],
      "uniqueConstraints": [
        {
          "name": "users_email_key",
          "columns": ["email"]
        }
      ],
      "checkConstraints": [],
      "indexes": []
    }
  ],
  "views": [],
  "materializedViews": [],
  "sequences": [
    {
      "name": "users_id_seq",
      "dataType": "bigint",
      "start": 1,
      "increment": 1,
      "minValue": 1,
      "maxValue": 9223372036854775807,
      "cache": 1,
      "cycle": false
    }
  ],
  "enums": []
}
```

### Resource: `tern://migrations`

Returns the migration history.

**URI**: `tern://migrations`

**Description**: List of all migrations in order of application.

**MIME Type**: `application/json`

**Content Structure**:

```json
{
  "migrations": [
    {
      "id": "a1b2c3d4...",
      "sequenceNumber": 1,
      "description": "Create users table",
      "createdAt": "2024-01-15T10:30:00Z",
      "operationCount": 3,
      "hasBreakingChanges": false,
      "parentStateHash": "0000000000...",
      "resultingStateHash": "e5f6g7h8..."
    },
    {
      "id": "i9j0k1l2...",
      "sequenceNumber": 2,
      "description": "Add posts table",
      "createdAt": "2024-01-16T14:20:00Z",
      "operationCount": 5,
      "hasBreakingChanges": false,
      "parentStateHash": "e5f6g7h8...",
      "resultingStateHash": "m3n4o5p6..."
    }
  ],
  "totalCount": 2,
  "currentStateHash": "m3n4o5p6..."
}
```

### Resource: `tern://migration/{id}`

Returns full details of a specific migration.

**URI Template**: `tern://migration/{id}`

**Description**: Full details of a specific migration including all operations.

**MIME Type**: `application/json`

**Content Structure**:

```json
{
  "id": "a1b2c3d4...",
  "sequenceNumber": 1,
  "description": "Create users table",
  "createdAt": "2024-01-15T10:30:00Z",
  "parentStateHash": "0000000000...",
  "resultingStateHash": "e5f6g7h8...",
  "upOperations": [
    {
      "type": "CreateTable",
      "table": "users",
      "sql": "CREATE TABLE users (...)"
    }
  ],
  "downOperations": [
    {
      "type": "DropTable",
      "table": "users",
      "sql": "DROP TABLE users"
    }
  ],
  "breakingChanges": [],
  "isReversible": true
}
```

---

## MCP Tools

Tools allow the client to perform actions. All session-modifying tools require a `sessionId` parameter (except `start_session` and `list_sessions`).

### Tool: `start_session`

Begins a new migration authoring session.

**Description**: Initialize a new session for creating a migration. Returns a session ID that must be used for all subsequent operations.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "description": {
      "type": "string",
      "description": "Description for the migration being created (e.g., 'Add user preferences table')"
    }
  },
  "required": ["description"]
}
```

**Output**:

```json
{
  "sessionId": "sess_7f3k9m2x",
  "description": "Add user preferences table",
  "baseStateHash": "m3n4o5p6...",
  "startedAt": "2024-01-20T09:15:00Z",
  "message": "Session started. Current schema has 2 tables, 0 views, 2 sequences, 0 enums."
}
```

**Behavior**:

1. Generate unique session ID (format: `sess_` + 8 alphanumeric chars)
2. Create new PGLite instance cloned from base state
3. Store session metadata in SessionManager
4. Return session details

**Errors**:

- `SESSION_LIMIT_REACHED`: Too many concurrent sessions (if limit imposed)

---

### Tool: `execute_sql`

Execute arbitrary SQL against the session's database.

**Description**: Execute raw SQL statement(s) against the session's in-memory database. Use this for schema changes that aren't covered by structured operations, or when you prefer writing SQL directly.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    },
    "sql": {
      "type": "string",
      "description": "SQL statement(s) to execute. Multiple statements can be separated by semicolons."
    }
  },
  "required": ["sessionId", "sql"]
}
```

**Output (success)**:

```json
{
  "success": true,
  "rowsAffected": 0,
  "message": "CREATE TABLE executed successfully",
  "warnings": []
}
```

**Output (error)**:

```json
{
  "success": false,
  "error": {
    "message": "relation \"users\" already exists",
    "code": "42P07",
    "detail": null,
    "hint": null,
    "position": 14
  }
}
```

**Behavior**:

1. Validate session exists and is active
2. Execute SQL against session's PGLite instance
3. Track the SQL statement in session history
4. Return result

**Security Note**: Since this runs locally on the user's machine against an ephemeral in-memory database, SQL injection is not a concern. The worst case is the user breaks their own session database, which can be discarded with `cancel_session`.

---

### Tool: `apply_operation`

Apply a structured schema operation.

**Description**: Apply a structured schema modification operation. This is an alternative to raw SQL that provides better validation and error messages.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    },
    "operation": {
      "type": "object",
      "description": "The operation to apply",
      "oneOf": [
        { "$ref": "#/$defs/CreateTable" },
        { "$ref": "#/$defs/DropTable" },
        { "$ref": "#/$defs/RenameTable" },
        { "$ref": "#/$defs/AddColumn" },
        { "$ref": "#/$defs/DropColumn" },
        { "$ref": "#/$defs/RenameColumn" },
        { "$ref": "#/$defs/AlterColumnType" },
        { "$ref": "#/$defs/AlterColumnDefault" },
        { "$ref": "#/$defs/AlterColumnNullable" },
        { "$ref": "#/$defs/AddPrimaryKey" },
        { "$ref": "#/$defs/AddForeignKey" },
        { "$ref": "#/$defs/AddUniqueConstraint" },
        { "$ref": "#/$defs/AddCheckConstraint" },
        { "$ref": "#/$defs/DropConstraint" },
        { "$ref": "#/$defs/CreateIndex" },
        { "$ref": "#/$defs/DropIndex" },
        { "$ref": "#/$defs/CreateEnum" },
        { "$ref": "#/$defs/AddEnumValue" },
        { "$ref": "#/$defs/DropEnum" }
      ]
    }
  },
  "required": ["sessionId", "operation"],
  "$defs": {
    "CreateTable": {
      "type": "object",
      "properties": {
        "type": { "const": "create_table" },
        "name": { "type": "string", "description": "Table name" },
        "columns": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "name": { "type": "string" },
              "dataType": { "type": "string", "description": "PostgreSQL data type" },
              "nullable": { "type": "boolean", "default": true },
              "default": { "type": "string", "description": "Default value expression" }
            },
            "required": ["name", "dataType"]
          }
        },
        "primaryKey": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Column names forming the primary key"
        }
      },
      "required": ["type", "name", "columns"]
    },
    "DropTable": {
      "type": "object",
      "properties": {
        "type": { "const": "drop_table" },
        "name": { "type": "string" },
        "cascade": { "type": "boolean", "default": false }
      },
      "required": ["type", "name"]
    },
    "RenameTable": {
      "type": "object",
      "properties": {
        "type": { "const": "rename_table" },
        "from": { "type": "string" },
        "to": { "type": "string" }
      },
      "required": ["type", "from", "to"]
    },
    "AddColumn": {
      "type": "object",
      "properties": {
        "type": { "const": "add_column" },
        "table": { "type": "string" },
        "column": {
          "type": "object",
          "properties": {
            "name": { "type": "string" },
            "dataType": { "type": "string" },
            "nullable": { "type": "boolean", "default": true },
            "default": { "type": "string" }
          },
          "required": ["name", "dataType"]
        }
      },
      "required": ["type", "table", "column"]
    },
    "DropColumn": {
      "type": "object",
      "properties": {
        "type": { "const": "drop_column" },
        "table": { "type": "string" },
        "column": { "type": "string" },
        "cascade": { "type": "boolean", "default": false }
      },
      "required": ["type", "table", "column"]
    },
    "RenameColumn": {
      "type": "object",
      "properties": {
        "type": { "const": "rename_column" },
        "table": { "type": "string" },
        "from": { "type": "string" },
        "to": { "type": "string" }
      },
      "required": ["type", "table", "from", "to"]
    },
    "AlterColumnType": {
      "type": "object",
      "properties": {
        "type": { "const": "alter_column_type" },
        "table": { "type": "string" },
        "column": { "type": "string" },
        "newType": { "type": "string" },
        "using": { "type": "string", "description": "USING expression for conversion" }
      },
      "required": ["type", "table", "column", "newType"]
    },
    "AlterColumnDefault": {
      "type": "object",
      "properties": {
        "type": { "const": "alter_column_default" },
        "table": { "type": "string" },
        "column": { "type": "string" },
        "default": { "type": ["string", "null"], "description": "New default value, or null to drop" }
      },
      "required": ["type", "table", "column", "default"]
    },
    "AlterColumnNullable": {
      "type": "object",
      "properties": {
        "type": { "const": "alter_column_nullable" },
        "table": { "type": "string" },
        "column": { "type": "string" },
        "nullable": { "type": "boolean" }
      },
      "required": ["type", "table", "column", "nullable"]
    },
    "AddPrimaryKey": {
      "type": "object",
      "properties": {
        "type": { "const": "add_primary_key" },
        "table": { "type": "string" },
        "columns": { "type": "array", "items": { "type": "string" } },
        "name": { "type": "string", "description": "Constraint name (auto-generated if omitted)" }
      },
      "required": ["type", "table", "columns"]
    },
    "AddForeignKey": {
      "type": "object",
      "properties": {
        "type": { "const": "add_foreign_key" },
        "table": { "type": "string" },
        "columns": { "type": "array", "items": { "type": "string" } },
        "referencesTable": { "type": "string" },
        "referencesColumns": { "type": "array", "items": { "type": "string" } },
        "onDelete": {
          "type": "string",
          "enum": ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"],
          "default": "NO ACTION"
        },
        "onUpdate": {
          "type": "string",
          "enum": ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"],
          "default": "NO ACTION"
        },
        "name": { "type": "string" }
      },
      "required": ["type", "table", "columns", "referencesTable", "referencesColumns"]
    },
    "AddUniqueConstraint": {
      "type": "object",
      "properties": {
        "type": { "const": "add_unique_constraint" },
        "table": { "type": "string" },
        "columns": { "type": "array", "items": { "type": "string" } },
        "name": { "type": "string" }
      },
      "required": ["type", "table", "columns"]
    },
    "AddCheckConstraint": {
      "type": "object",
      "properties": {
        "type": { "const": "add_check_constraint" },
        "table": { "type": "string" },
        "expression": { "type": "string" },
        "name": { "type": "string" }
      },
      "required": ["type", "table", "expression"]
    },
    "DropConstraint": {
      "type": "object",
      "properties": {
        "type": { "const": "drop_constraint" },
        "table": { "type": "string" },
        "name": { "type": "string" },
        "cascade": { "type": "boolean", "default": false }
      },
      "required": ["type", "table", "name"]
    },
    "CreateIndex": {
      "type": "object",
      "properties": {
        "type": { "const": "create_index" },
        "table": { "type": "string" },
        "columns": {
          "type": "array",
          "items": {
            "oneOf": [
              { "type": "string" },
              {
                "type": "object",
                "properties": {
                  "name": { "type": "string" },
                  "order": { "type": "string", "enum": ["ASC", "DESC"] },
                  "nulls": { "type": "string", "enum": ["FIRST", "LAST"] }
                },
                "required": ["name"]
              }
            ]
          }
        },
        "name": { "type": "string" },
        "unique": { "type": "boolean", "default": false },
        "method": {
          "type": "string",
          "enum": ["btree", "hash", "gin", "gist", "brin"],
          "default": "btree"
        },
        "where": { "type": "string", "description": "Partial index predicate" }
      },
      "required": ["type", "table", "columns"]
    },
    "DropIndex": {
      "type": "object",
      "properties": {
        "type": { "const": "drop_index" },
        "name": { "type": "string" },
        "cascade": { "type": "boolean", "default": false }
      },
      "required": ["type", "name"]
    },
    "CreateEnum": {
      "type": "object",
      "properties": {
        "type": { "const": "create_enum" },
        "name": { "type": "string" },
        "values": { "type": "array", "items": { "type": "string" }, "minItems": 1 }
      },
      "required": ["type", "name", "values"]
    },
    "AddEnumValue": {
      "type": "object",
      "properties": {
        "type": { "const": "add_enum_value" },
        "enumName": { "type": "string" },
        "value": { "type": "string" },
        "before": { "type": "string" },
        "after": { "type": "string" }
      },
      "required": ["type", "enumName", "value"]
    },
    "DropEnum": {
      "type": "object",
      "properties": {
        "type": { "const": "drop_enum" },
        "name": { "type": "string" },
        "cascade": { "type": "boolean", "default": false }
      },
      "required": ["type", "name"]
    }
  }
}
```

**Example Input**:

```json
{
  "sessionId": "sess_7f3k9m2x",
  "operation": {
    "type": "create_table",
    "name": "preferences",
    "columns": [
      { "name": "id", "dataType": "SERIAL" },
      { "name": "user_id", "dataType": "INTEGER", "nullable": false },
      { "name": "theme", "dataType": "VARCHAR(50)", "default": "'light'" },
      { "name": "notifications_enabled", "dataType": "BOOLEAN", "default": "true" }
    ],
    "primaryKey": ["id"]
  }
}
```

**Output**:

```json
{
  "success": true,
  "sqlExecuted": "CREATE TABLE preferences (\n  id SERIAL,\n  user_id INTEGER NOT NULL,\n  theme VARCHAR(50) DEFAULT 'light',\n  notifications_enabled BOOLEAN DEFAULT true,\n  PRIMARY KEY (id)\n);",
  "message": "Created table 'preferences' with 4 columns"
}
```

**Behavior**:

1. Validate session exists
2. Validate operation structure
3. Convert operation to SQL using existing render infrastructure
4. Execute SQL against session's PGLite
5. Store operation in session history
6. Return result with generated SQL

---

### Tool: `get_session_schema`

Get the current schema state within a session.

**Description**: Returns the current schema state in the session's database, reflecting all changes made since the session started.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    }
  },
  "required": ["sessionId"]
}
```

**Output**: Same structure as `tern://schema` resource, but reflecting the session's current state.

---

### Tool: `get_session_diff`

Get the diff between base state and current session state.

**Description**: Returns a summary of all changes made in the session compared to the base schema.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    }
  },
  "required": ["sessionId"]
}
```

**Output**:

```json
{
  "hasChanges": true,
  "summary": {
    "tablesAdded": 1,
    "tablesRemoved": 0,
    "tablesModified": 1,
    "columnsAdded": 2,
    "columnsRemoved": 0,
    "columnsModified": 0,
    "indexesAdded": 1,
    "indexesRemoved": 0,
    "constraintsAdded": 1,
    "constraintsRemoved": 0,
    "enumsAdded": 0,
    "enumsRemoved": 0
  },
  "details": {
    "tablesAdded": ["preferences"],
    "tablesModified": [
      {
        "table": "users",
        "columnsAdded": ["last_login", "avatar_url"],
        "columnsRemoved": [],
        "columnsModified": []
      }
    ],
    "indexesAdded": [
      {
        "name": "idx_users_email",
        "table": "users",
        "columns": ["email"]
      }
    ],
    "foreignKeysAdded": [
      {
        "name": "preferences_user_id_fkey",
        "table": "preferences",
        "columns": ["user_id"],
        "referencesTable": "users",
        "referencesColumns": ["id"]
      }
    ]
  },
  "breakingChanges": [],
  "hasBreakingChanges": false
}
```

---

### Tool: `generate_migration`

Finalize the session and create a migration file.

**Description**: Generates a migration from all changes made in the session and saves it to the .tern/migrations directory. This ends the session.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    },
    "description": {
      "type": "string",
      "description": "Override the migration description (uses session description if not provided)"
    },
    "force": {
      "type": "boolean",
      "default": false,
      "description": "Generate migration even if it contains breaking changes"
    }
  },
  "required": ["sessionId"]
}
```

**Output**:

```json
{
  "success": true,
  "migrationId": "x7y8z9...",
  "sequenceNumber": 3,
  "description": "Add user preferences table",
  "filePath": ".tern/migrations/00003.json",
  "operationCount": 5,
  "hasBreakingChanges": false,
  "breakingChanges": [],
  "isReversible": true,
  "sessionEnded": true,
  "message": "Migration 00003 created: 'Add user preferences table'"
}
```

**Behavior**:

1. Validate session exists
2. Load current schema from session PGLite
3. Diff against base state using existing diff infrastructure
4. Check for breaking changes; require `force: true` if present
5. Generate migration operations using existing diff→operation pipeline
6. Compute inverse operations for down migration
7. Create Migration struct with proper hashing
8. Save to `.tern/migrations/` using LocalFileBackend
9. Update `state.json` with new schema
10. Regenerate `schema.sql`
11. Clean up session (destroy PGLite instance, remove from SessionManager)
12. Return migration details

**Errors**:

- `NO_CHANGES`: Session has no schema changes
- `BREAKING_CHANGES_DETECTED`: Has breaking changes and `force` is false
- `SESSION_NOT_FOUND`: Invalid session ID
- `SAVE_FAILED`: Could not write migration file

---

### Tool: `cancel_session`

Discard a session without creating a migration.

**Description**: Cancels an active session and discards all changes. No migration is created.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {
    "sessionId": {
      "type": "string",
      "description": "Session ID from start_session"
    }
  },
  "required": ["sessionId"]
}
```

**Output**:

```json
{
  "success": true,
  "sessionId": "sess_7f3k9m2x",
  "changesDiscarded": 5,
  "message": "Session cancelled. 5 operations discarded, no migration created."
}
```

---

### Tool: `list_sessions`

List all active sessions.

**Description**: Returns a list of all currently active sessions. Useful for recovery if a session ID is lost.

**Input Schema**:

```json
{
  "type": "object",
  "properties": {}
}
```

**Output**:

```json
{
  "sessions": [
    {
      "sessionId": "sess_7f3k9m2x",
      "description": "Add user preferences table",
      "startedAt": "2024-01-20T09:15:00Z",
      "operationCount": 5,
      "hasChanges": true
    }
  ],
  "totalCount": 1
}
```

---

## Session Management

### Session State

```rust
/// Unique session identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    /// Generate a new random session ID
    pub fn generate() -> Self {
        Self(format!("sess_{}", nanoid::nanoid!(8)))
    }
}

/// An active migration authoring session
pub struct Session {
    /// Unique identifier
    pub id: SessionId,

    /// User-provided description for the migration
    pub description: String,

    /// When the session was started
    pub started_at: chrono::DateTime<chrono::Utc>,

    /// PGLite database instance for this session
    pub database: PgLiteInstance,

    /// Schema state at session start (immutable, for diffing)
    pub base_state: Namespace,

    /// History of SQL statements executed (for debugging/audit)
    pub sql_history: Vec<String>,

    /// History of structured operations applied
    pub operation_history: Vec<OperationRecord>,
}

/// Record of an operation applied in a session
pub struct OperationRecord {
    /// The operation (if structured)
    pub operation: Option<Operation>,

    /// SQL that was executed
    pub sql: String,

    /// When it was applied
    pub applied_at: chrono::DateTime<chrono::Utc>,
}

/// Manages all active sessions
pub struct SessionManager {
    /// Active sessions indexed by ID
    sessions: HashMap<SessionId, Session>,

    /// Base schema state (shared, immutable)
    base_state: Arc<Namespace>,

    /// State backend for saving migrations
    backend: Arc<LocalFileBackend>,

    /// Maximum concurrent sessions (optional limit)
    max_sessions: Option<usize>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(
        base_state: Namespace,
        backend: LocalFileBackend,
    ) -> Self;

    /// Start a new session
    pub async fn start_session(
        &mut self,
        description: String,
    ) -> Result<SessionId, SessionError>;

    /// Get a session by ID
    pub fn get_session(&self, id: &SessionId) -> Option<&Session>;

    /// Get a mutable session by ID
    pub fn get_session_mut(&mut self, id: &SessionId) -> Option<&mut Session>;

    /// Cancel and remove a session
    pub async fn cancel_session(&mut self, id: &SessionId) -> Result<(), SessionError>;

    /// Generate migration from session and remove it
    pub async fn generate_migration(
        &mut self,
        id: &SessionId,
        description_override: Option<String>,
        force: bool,
    ) -> Result<Migration, GenerateError>;

    /// List all active sessions
    pub fn list_sessions(&self) -> Vec<SessionSummary>;
}
```

### Session Lifecycle Diagram

```
┌──────────────────────────────────────────────────────────────────────────┐
│                           Session Lifecycle                               │
└──────────────────────────────────────────────────────────────────────────┘

          start_session(description)
                    │
                    ▼
            ┌───────────────┐
            │    ACTIVE     │◄────────────────────────────────────┐
            │               │                                     │
            │ • PGLite up   │     execute_sql(sql)               │
            │ • Tracks ops  │     apply_operation(op)            │
            │ • Can diff    │     get_session_schema()           │
            │               │     get_session_diff()             │
            └───────┬───────┴─────────────────────────────────────┘
                    │
        ┌───────────┴───────────┐
        │                       │
        ▼                       ▼
┌───────────────┐       ┌───────────────┐
│cancel_session │       │generate_      │
│               │       │migration      │
│ • Discard     │       │               │
│   changes     │       │ • Diff state  │
│ • No file     │       │ • Save file   │
│   written     │       │ • Update      │
│               │       │   state.json  │
└───────┬───────┘       └───────┬───────┘
        │                       │
        ▼                       ▼
┌──────────────────────────────────────┐
│              TERMINATED              │
│                                      │
│ • PGLite destroyed                   │
│ • Session removed from manager       │
│ • Cannot be resumed                  │
└──────────────────────────────────────┘
```

### Concurrency Considerations

- Sessions are independent and isolated
- Each session has its own PGLite instance
- No cross-session operations permitted
- SessionManager uses `RwLock` or similar for thread-safe access
- The MCP server is single-threaded (stdio), simplifying concurrency
- Future HTTP transport would require proper synchronization

---

## CLI Integration

### Command Structure

```rust
// In src/cli/mod.rs
#[derive(Debug, Clone, clap::Subcommand)]
pub enum CliCommand {
    // ... existing commands
    /// Start the MCP server for AI-assisted migration authoring
    Mcp(mcp::Mcp),
}

// In src/cli/mcp/mod.rs
/// MCP server for AI-assisted database migration authoring
#[derive(Debug, Clone, clap::Args)]
pub struct Mcp {
    /// Path to .tern directory (defaults to searching current and parent directories)
    #[arg(long, short = 'p')]
    pub path: Option<PathBuf>,

    /// Log level for MCP server (logs go to stderr, not interfering with stdio transport)
    #[arg(long, default_value = "warn")]
    pub log_level: tracing::level_filters::LevelFilter,
}

impl Mcp {
    pub async fn dispatch(self) -> miette::Result<()> {
        // 1. Find and load backend
        let backend = match &self.path {
            Some(p) => LocalFileBackend::at_path(p),
            None => LocalFileBackend::find_root()
                .ok_or_else(|| miette::miette!(
                    "No .tern directory found.\n\n\
                     Run 'tern init' to initialize a new Tern project."
                ))?,
        };

        // 2. Verify backend is initialized
        ensure_backend_initialized(&backend).await?;

        // 3. Initialize and run server
        let server = McpServer::initialize(backend).await?;
        server.run_stdio().await
    }
}
```

### Usage

```bash
# Start MCP server (searches for .tern in current/parent directories)
tern mcp

# Start MCP server with explicit path
tern mcp --path /path/to/project

# Start with debug logging (logs to stderr)
tern mcp --log-level debug
```

---

## File Structure

### New Files

```
src/
├── cli/
│   └── mcp/
│       └── mod.rs                # CLI subcommand definition
├── mcp/
│   ├── mod.rs                    # Module root, public exports
│   ├── server.rs                 # MCP server implementation
│   ├── transport.rs              # stdio transport handling
│   ├── protocol.rs               # MCP protocol types and JSON-RPC handling
│   ├── resources/
│   │   ├── mod.rs                # Resource registry
│   │   ├── schema.rs             # tern://schema resource
│   │   └── migrations.rs         # tern://migrations resource
│   ├── tools/
│   │   ├── mod.rs                # Tool registry
│   │   ├── session.rs            # start_session, cancel_session, list_sessions
│   │   ├── execute.rs            # execute_sql, apply_operation
│   │   ├── inspect.rs            # get_session_schema, get_session_diff
│   │   └── generate.rs           # generate_migration
│   ├── session.rs                # Session, SessionId, SessionManager
│   ├── pglite.rs                 # PGLite wrapper/abstraction
│   └── error.rs                  # MCP-specific error types
```

### Dependencies to Add

```toml
# Cargo.toml additions

[dependencies]
# Session ID generation
nanoid = "0.4"

# JSON-RPC protocol handling
serde_json = "1"  # Already present

# Timestamps for session tracking
chrono = { version = "0.4", features = ["serde"] }

# PGLite integration (evaluate options)
# Option A: If pglite-rs exists and is maintained
# pglite = "0.x"

# Option B: Embedded PostgreSQL alternative
# pgtemp = "0.2"

# Option C: Use existing wasmtime for PGLite WASM
# (no additional deps needed)
```

---

## Error Handling

### Error Types

```rust
/// MCP server errors
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    // Protocol errors (JSON-RPC standard codes)
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Method not found: {0}")]
    MethodNotFound(String),

    #[error("Invalid params: {0}")]
    InvalidParams(String),

    #[error("Internal error: {0}")]
    InternalError(String),

    // Session errors
    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("No active session. Call start_session first.")]
    NoActiveSession,

    #[error("Session limit reached. Cancel an existing session first.")]
    SessionLimitReached,

    // SQL execution errors
    #[error("SQL execution failed: {0}")]
    SqlExecutionFailed(String),

    // Migration generation errors
    #[error("No changes in session")]
    NoChanges,

    #[error("Breaking changes detected. Use force=true to proceed.")]
    BreakingChangesDetected(Vec<BreakingChange>),

    #[error("Failed to save migration: {0}")]
    SaveFailed(String),

    // Backend errors
    #[error("Backend not initialized: {0}")]
    BackendNotInitialized(String),

    // PGLite errors
    #[error("PGLite error: {0}")]
    PgLiteError(String),
}

impl McpError {
    /// Convert to JSON-RPC error code
    pub fn error_code(&self) -> i32 {
        match self {
            Self::ParseError(_) => -32700,
            Self::InvalidRequest(_) => -32600,
            Self::MethodNotFound(_) => -32601,
            Self::InvalidParams(_) => -32602,
            Self::InternalError(_) => -32603,
            Self::SessionNotFound(_) => -32100,
            Self::NoActiveSession => -32101,
            Self::SessionLimitReached => -32102,
            Self::SqlExecutionFailed(_) => -32103,
            Self::NoChanges => -32104,
            Self::BreakingChangesDetected(_) => -32105,
            Self::SaveFailed(_) => -32106,
            Self::BackendNotInitialized(_) => -32107,
            Self::PgLiteError(_) => -32108,
        }
    }
}
```

### Error Response Format

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32100,
    "message": "Session not found: sess_invalid",
    "data": {
      "sessionId": "sess_invalid",
      "activeSessions": ["sess_7f3k9m2x"],
      "hint": "Use list_sessions to see active sessions, or start_session to create a new one."
    }
  }
}
```

---

## Testing Strategy

### Unit Tests

1. **Session Manager**
   - `test_start_session` — Creates session with unique ID
   - `test_get_session` — Retrieves session by ID
   - `test_cancel_session` — Removes session and cleans up
   - `test_session_not_found` — Returns error for invalid ID
   - `test_max_sessions_limit` — Enforces session limit if configured

2. **Tool Handlers**
   - `test_execute_sql_success` — Executes valid SQL
   - `test_execute_sql_error` — Returns SQL errors properly
   - `test_apply_operation_create_table` — Creates table from operation
   - `test_apply_operation_add_column` — Adds column correctly
   - `test_get_session_diff` — Returns accurate diff

3. **Resource Handlers**
   - `test_schema_resource` — Returns correct schema structure
   - `test_migrations_resource` — Lists migrations correctly
   - `test_migration_detail_resource` — Returns migration details

4. **PGLite Integration**
   - `test_pglite_create_instance` — Creates in-memory database
   - `test_pglite_execute_ddl` — Executes DDL statements
   - `test_pglite_get_namespace` — Extracts schema correctly

### Integration Tests

```rust
#[tokio::test]
async fn test_full_migration_workflow() {
    // 1. Set up test .tern directory with base state
    let temp_dir = tempfile::tempdir().unwrap();
    setup_test_tern_directory(&temp_dir).await;

    // 2. Create MCP server
    let backend = LocalFileBackend::at_path(temp_dir.path());
    let server = McpServer::initialize(backend).await.unwrap();

    // 3. Start session
    let response = server.call_tool("start_session", json!({
        "description": "Add preferences table"
    })).await.unwrap();
    let session_id = response["sessionId"].as_str().unwrap();

    // 4. Apply operations
    server.call_tool("apply_operation", json!({
        "sessionId": session_id,
        "operation": {
            "type": "create_table",
            "name": "preferences",
            "columns": [
                { "name": "id", "dataType": "SERIAL" },
                { "name": "user_id", "dataType": "INTEGER", "nullable": false }
            ],
            "primaryKey": ["id"]
        }
    })).await.unwrap();

    // 5. Check diff
    let diff = server.call_tool("get_session_diff", json!({
        "sessionId": session_id
    })).await.unwrap();
    assert!(diff["hasChanges"].as_bool().unwrap());
    assert_eq!(diff["summary"]["tablesAdded"].as_i64().unwrap(), 1);

    // 6. Generate migration
    let result = server.call_tool("generate_migration", json!({
        "sessionId": session_id
    })).await.unwrap();
    assert!(result["success"].as_bool().unwrap());

    // 7. Verify migration file exists
    let migration_path = temp_dir.path()
        .join(".tern/migrations/00001.json");
    assert!(migration_path.exists());
}
```

### Test Infrastructure

```rust
/// Test helper for MCP server testing
pub struct TestMcpServer {
    server: McpServer,
    temp_dir: tempfile::TempDir,
}

impl TestMcpServer {
    /// Create a test server with an empty .tern directory
    pub async fn empty() -> Self;

    /// Create a test server with predefined base state
    pub async fn with_state(namespace: Namespace) -> Self;

    /// Call a tool and return the result
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value, McpError>;

    /// Read a resource
    pub async fn read_resource(&self, uri: &str) -> Result<Value, McpError>;
}
```

---

## Implementation Phases

### Phase 1: Foundation (Core Infrastructure)

**Goal**: Basic MCP server that can start and respond to protocol messages.

**Tasks**:
1. Create `src/mcp/` module structure
2. Implement JSON-RPC message parsing and routing
3. Implement stdio transport
4. Add `tern mcp` CLI subcommand
5. Implement MCP `initialize` and `ping` handlers
6. Implement resource and tool listing

**Deliverables**:
- Server starts and responds to MCP protocol messages
- `tern mcp` command works

### Phase 2: Resources (Schema Inspection)

**Goal**: Expose schema information via MCP resources.

**Tasks**:
1. Implement `tern://schema` resource
2. Implement `tern://migrations` resource
3. Implement `tern://migration/{id}` resource
4. Add resource discovery handler

**Deliverables**:
- Clients can read current schema and migration history

### Phase 3: PGLite Integration

**Goal**: In-memory PostgreSQL for schema validation.

**Tasks**:
1. Evaluate and select PGLite implementation approach
2. Implement `PgLiteInstance` wrapper
3. Implement base database initialization (apply migrations)
4. Implement namespace extraction from PGLite

**Deliverables**:
- Server initializes PGLite with current schema on startup

### Phase 4: Session Management

**Goal**: Session-based workflow for migration authoring.

**Tasks**:
1. Implement `Session` and `SessionManager`
2. Implement `start_session` tool
3. Implement `cancel_session` tool
4. Implement `list_sessions` tool
5. Implement session PGLite instance creation

**Deliverables**:
- Users can start and manage editing sessions

### Phase 5: Session Operations

**Goal**: Schema modification within sessions.

**Tasks**:
1. Implement `execute_sql` tool
2. Implement `apply_operation` tool with all operation types
3. Implement `get_session_schema` tool
4. Implement `get_session_diff` tool
5. Integrate with existing SQL rendering infrastructure

**Deliverables**:
- Users can modify schema via SQL or structured operations

### Phase 6: Migration Generation

**Goal**: Complete workflow from session to saved migration.

**Tasks**:
1. Implement `generate_migration` tool
2. Integrate with existing diff→operation pipeline
3. Integrate with existing inverse operation generation
4. Integrate with LocalFileBackend for saving
5. Handle breaking changes detection

**Deliverables**:
- Full end-to-end workflow: start session → make changes → generate migration

### Phase 7: Polish and Documentation

**Goal**: Production-ready quality.

**Tasks**:
1. Improve error messages with suggestions
2. Add comprehensive logging
3. Write user documentation
4. Add example usage in docs
5. Performance optimization if needed

---

## Example Session

A complete example of using the MCP server:

```
┌────────────────────────────────────────────────────────────────────────────┐
│  User: "I need to add a comments feature. Users should be able to          │
│         comment on posts."                                                 │
└────────────────────────────────────────────────────────────────────────────┘

[AI reads tern://schema resource to understand current schema]

┌────────────────────────────────────────────────────────────────────────────┐
│  AI: "I see you have `users` and `posts` tables. I'll create a `comments` │
│       table that references both. Let me start a migration session."       │
└────────────────────────────────────────────────────────────────────────────┘

[AI calls start_session]
Request:  { "description": "Add comments feature" }
Response: { "sessionId": "sess_abc123", "baseStateHash": "...", ... }

[AI calls apply_operation to create table]
Request:  {
  "sessionId": "sess_abc123",
  "operation": {
    "type": "create_table",
    "name": "comments",
    "columns": [
      { "name": "id", "dataType": "SERIAL" },
      { "name": "post_id", "dataType": "INTEGER", "nullable": false },
      { "name": "user_id", "dataType": "INTEGER", "nullable": false },
      { "name": "body", "dataType": "TEXT", "nullable": false },
      { "name": "created_at", "dataType": "TIMESTAMPTZ", "default": "now()" }
    ],
    "primaryKey": ["id"]
  }
}
Response: { "success": true, "sqlExecuted": "CREATE TABLE comments (...)", ... }

[AI calls apply_operation to add foreign keys]
Request:  {
  "sessionId": "sess_abc123",
  "operation": {
    "type": "add_foreign_key",
    "table": "comments",
    "columns": ["post_id"],
    "referencesTable": "posts",
    "referencesColumns": ["id"],
    "onDelete": "CASCADE"
  }
}
Response: { "success": true, ... }

[AI calls apply_operation for second FK]
Request:  {
  "sessionId": "sess_abc123",
  "operation": {
    "type": "add_foreign_key",
    "table": "comments",
    "columns": ["user_id"],
    "referencesTable": "users",
    "referencesColumns": ["id"],
    "onDelete": "CASCADE"
  }
}
Response: { "success": true, ... }

[AI calls apply_operation to create index]
Request:  {
  "sessionId": "sess_abc123",
  "operation": {
    "type": "create_index",
    "table": "comments",
    "name": "idx_comments_post_created",
    "columns": [
      "post_id",
      { "name": "created_at", "order": "DESC" }
    ]
  }
}
Response: { "success": true, ... }

[AI calls get_session_diff to summarize]
Request:  { "sessionId": "sess_abc123" }
Response: {
  "hasChanges": true,
  "summary": { "tablesAdded": 1, "indexesAdded": 1, "constraintsAdded": 2, ... },
  "details": { ... }
}

┌────────────────────────────────────────────────────────────────────────────┐
│  AI: "I've created the comments table with:                                │
│       • Primary key (id)                                                   │
│       • Foreign keys to posts and users (both CASCADE on delete)           │
│       • An index for efficiently fetching comments by post                 │
│                                                                            │
│       Would you like me to generate the migration?"                        │
└────────────────────────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────────────────────────┐
│  User: "Yes, generate it"                                                  │
└────────────────────────────────────────────────────────────────────────────┘

[AI calls generate_migration]
Request:  { "sessionId": "sess_abc123" }
Response: {
  "success": true,
  "migrationId": "...",
  "sequenceNumber": 3,
  "description": "Add comments feature",
  "filePath": ".tern/migrations/00003.json",
  "operationCount": 4,
  "hasBreakingChanges": false,
  "isReversible": true,
  "message": "Migration 00003 created: 'Add comments feature'"
}

┌────────────────────────────────────────────────────────────────────────────┐
│  AI: "Migration created! You can find it at .tern/migrations/00003.json   │
│                                                                            │
│       To apply it to your database, run:                                   │
│       $ tern up                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## Open Questions

1. **PGLite Technology Choice**: Which PGLite implementation to use?
   - Native Rust bindings (if available and maintained)
   - WASM-based using existing wasmtime infrastructure
   - Alternative embedded PostgreSQL (pgtemp, embedded_postgres)
   - **Recommendation**: Start with investigation spike to evaluate options

2. **Session Persistence**: Should sessions survive server restarts?
   - **Pro**: Allows recovery from crashes
   - **Con**: Adds complexity, stale session cleanup needed
   - **Recommendation**: Start without persistence; add if user feedback indicates need

3. **Session Limits**: Should we limit concurrent sessions?
   - Each session uses memory for PGLite instance
   - **Recommendation**: Default limit of 5 sessions, configurable

4. **Schema Scope**: Support multiple schemas beyond "public"?
   - Current Tern focuses on single schema
   - **Recommendation**: Match existing behavior; add multi-schema later if needed

5. **Operation Subset**: Which operations to support initially?
   - **Recommendation**: Start with core operations (create/drop table, add/drop column, add FK/PK/unique, create/drop index, create/add enum)
   - Add more based on usage patterns

---

## Security Considerations

1. **Local-Only**: Server only accepts stdio connections, not network
2. **No Production DB Access**: Only touches in-memory PGLite and local files
3. **SQL Execution**: Safe because it's against ephemeral in-memory database
4. **File System Access**: Limited to `.tern/` directory
5. **No Secrets**: No credentials stored or transmitted

---

## References

- [Model Context Protocol Specification](https://modelcontextprotocol.io/)
- [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)
- [PGLite](https://pglite.dev/)
- [Tern Project CLAUDE.md](../../CLAUDE.md)
