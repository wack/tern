# Design Document: MCP Server for Programmatic Schema Modifications

## Overview

This document describes the design for an MCP (Model Context Protocol) server that exposes Tern's schema modification capabilities as programmatic tools. This enables AI assistants and other MCP clients to create database migrations through a sequence of structured tool calls, without requiring users to manually edit SQL files.

### Core Concept

The MCP server acts as a bridge between MCP clients (like Claude) and Tern's migration system:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                         MCP-Based Migration Workflow                         │
└─────────────────────────────────────────────────────────────────────────────┘

  MCP Client                    Tern MCP Server                    Tern Core
  (Claude, etc.)
  ────────────                  ───────────────                    ─────────
      │                               │                               │
      │  1. create_table              │                               │
      │  ─────────────────────────►   │                               │
      │                               │  Apply operation to PGLite    │
      │                               │  ─────────────────────────►   │
      │   ◄── success ────────────    │                               │
      │                               │                               │
      │  2. add_column                │                               │
      │  ─────────────────────────►   │                               │
      │                               │  Apply operation to PGLite    │
      │                               │  ─────────────────────────►   │
      │   ◄── success ────────────    │                               │
      │                               │                               │
      │  3. get_migration             │                               │
      │  ─────────────────────────►   │                               │
      │                               │  Diff state.json vs PGLite    │
      │                               │  ─────────────────────────►   │
      │   ◄── migration SQL ──────    │                               │
      │                               │                               │
```

This builds directly on the model-first migrations workflow (Phase 2), replacing manual `schema.sql` editing with structured MCP tool calls.

## Motivation

### The Problem with Manual Schema Editing

The model-first workflow introduced in Phase 2 requires users to:

1. Open `.tern/schema.sql` in an editor
2. Make SQL modifications (add columns, tables, constraints)
3. Run `tern schema migrate` to generate the migration

While this is simpler than the original workflow (which required a running database), it still has friction:

- **SQL syntax knowledge required**: Users must know PostgreSQL DDL syntax
- **Error-prone**: Typos in column types, missing constraints, syntax errors
- **No validation until execution**: Errors only surface when PGLite tries to execute
- **AI assistance is indirect**: AI can suggest SQL, but users must copy-paste

### The MCP Solution

An MCP server enables a conversational workflow:

```
User: "Add a preferences table for users with columns for theme and notifications"

Claude: [calls create_table tool]
        [calls add_foreign_key tool]

Claude: "I've created a preferences table with:
         - id (primary key)
         - user_id (foreign key to users)
         - theme (varchar, default 'light')
         - notifications_enabled (boolean, default true)

         Would you like me to generate the migration?"

User: "Yes"

Claude: [calls get_migration tool]

Claude: "Here's your migration:

         CREATE TABLE preferences (
           id SERIAL PRIMARY KEY,
           user_id INTEGER NOT NULL REFERENCES users(id),
           theme VARCHAR(50) DEFAULT 'light',
           notifications_enabled BOOLEAN DEFAULT true
         );"
```

Benefits:
- **Natural language interface**: Describe changes in English, get migrations
- **Immediate validation**: Each operation is validated by PGLite as it's applied
- **Interactive refinement**: AI can suggest improvements, handle errors, iterate
- **No syntax memorization**: Users don't need to remember DDL syntax

## Architecture

### High-Level Components

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Architecture                                    │
└─────────────────────────────────────────────────────────────────────────────┘

  ┌─────────────────────────────────────────────────────────────────────────┐
  │                          MCP Transport Layer                             │
  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
  │  │   stdio     │  │   HTTP/SSE  │  │  WebSocket  │  │  (future)       │ │
  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                         MCP Server (src/mcp/)                            │
  │  ┌─────────────────────┐  ┌─────────────────────┐  ┌─────────────────┐  │
  │  │   Tool Definitions  │  │   Session Manager   │  │   Tool Router   │  │
  │  │   (schema ops)      │  │   (PGLite state)    │  │   (dispatch)    │  │
  │  └─────────────────────┘  └─────────────────────┘  └─────────────────┘  │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                    Operation Executor (src/mcp/executor.rs)              │
  │  ┌─────────────────────────────────────────────────────────────────────┐ │
  │  │  Converts MCP tool calls → SQL → PGLite execution → Response        │ │
  │  └─────────────────────────────────────────────────────────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
                                      │
                                      ▼
  ┌─────────────────────────────────────────────────────────────────────────┐
  │                       Existing Tern Infrastructure                       │
  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
  │  │  PGLite     │  │    Diff     │  │   Migrate   │  │ State Backend   │ │
  │  │  Runtime    │  │   Engine    │  │   Planner   │  │ (state.json)    │ │
  │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────────┘ │
  └─────────────────────────────────────────────────────────────────────────┘
```

### Session Management

The MCP server maintains a **session** that tracks the working state:

```rust
pub struct Session {
    /// Unique session identifier
    id: SessionId,

    /// PGLite runtime for this session
    runtime: PgLiteRuntime,

    /// Base namespace loaded from state.json (immutable reference point)
    base_namespace: Namespace,

    /// Operations applied in this session (for undo/history)
    operations: Vec<AppliedOperation>,

    /// Session creation time
    created_at: Instant,

    /// Working directory (where .tern/ lives)
    working_dir: PathBuf,
}

pub struct AppliedOperation {
    /// The operation that was applied
    operation: Operation,

    /// SQL that was executed
    sql: String,

    /// Timestamp when applied
    applied_at: Instant,
}
```

**Session Lifecycle:**

1. **Initialize**: Load `state.json`, start PGLite, execute current schema
2. **Operate**: Apply operations via tool calls, track in history
3. **Generate**: Diff PGLite state against base namespace, produce migration
4. **Commit** (optional): Save migration to `.tern/migrations/`
5. **Cleanup**: Terminate PGLite, release resources

### Tool Categories

The MCP server exposes tools organized into categories:

#### 1. Session Management Tools

| Tool | Description |
|------|-------------|
| `session_start` | Initialize a new session, loading from `.tern/` |
| `session_reset` | Reset session to base state (undo all operations) |
| `session_status` | Get current session state (operations applied, etc.) |

#### 2. Schema Inspection Tools

| Tool | Description |
|------|-------------|
| `list_tables` | List all tables in current schema |
| `describe_table` | Get detailed table definition (columns, constraints, indexes) |
| `list_enums` | List all enum types |
| `list_indexes` | List all indexes |
| `get_schema_ddl` | Get full schema as SQL DDL |

#### 3. Table Operations

| Tool | Description |
|------|-------------|
| `create_table` | Create a new table with columns |
| `drop_table` | Drop an existing table |
| `rename_table` | Rename a table |

#### 4. Column Operations

| Tool | Description |
|------|-------------|
| `add_column` | Add a column to a table |
| `drop_column` | Remove a column from a table |
| `rename_column` | Rename a column |
| `alter_column_type` | Change a column's data type |
| `alter_column_default` | Set or remove default value |
| `alter_column_nullable` | Change nullability constraint |

#### 5. Constraint Operations

| Tool | Description |
|------|-------------|
| `add_primary_key` | Add primary key constraint |
| `add_foreign_key` | Add foreign key reference |
| `add_unique_constraint` | Add unique constraint |
| `add_check_constraint` | Add check constraint |
| `drop_constraint` | Remove a constraint |

#### 6. Index Operations

| Tool | Description |
|------|-------------|
| `create_index` | Create an index |
| `drop_index` | Drop an index |

#### 7. Enum Operations

| Tool | Description |
|------|-------------|
| `create_enum` | Create an enum type |
| `add_enum_value` | Add a value to existing enum |
| `drop_enum` | Drop an enum type |

#### 8. Migration Output Tools

| Tool | Description |
|------|-------------|
| `get_migration_preview` | Preview migration SQL without committing |
| `get_migration_operations` | Get migration as structured operations |
| `commit_migration` | Save migration to `.tern/migrations/` |
| `get_breaking_changes` | List any destructive/breaking changes |

## Tool Definitions

### Session Management

#### `session_start`

Initializes a new session, loading the base state from the Tern project.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "working_dir": {
      "type": "string",
      "description": "Path to directory containing .tern/. Defaults to current directory."
    },
    "schema_name": {
      "type": "string",
      "description": "PostgreSQL schema name to work with. Defaults to 'public'."
    }
  }
}
```

**Output:**
```json
{
  "session_id": "abc123",
  "base_state": {
    "tables": ["users", "posts"],
    "enums": ["user_status"],
    "sequences": ["users_id_seq", "posts_id_seq"]
  },
  "message": "Session started. Base schema has 2 tables, 1 enum, 2 sequences."
}
```

**Behavior:**
1. Locate `.tern/` directory (error if not found)
2. Load `state.json` to get base namespace
3. Start PGLite runtime
4. Execute base schema SQL in PGLite
5. Return session ID and summary

#### `session_status`

Returns the current session state.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {}
}
```

**Output:**
```json
{
  "session_id": "abc123",
  "operations_applied": 3,
  "pending_changes": {
    "tables_added": ["preferences"],
    "columns_added": ["users.avatar_url"],
    "indexes_created": ["idx_users_email"]
  },
  "has_breaking_changes": false
}
```

#### `session_reset`

Resets the session to the base state.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "confirm": {
      "type": "boolean",
      "description": "Must be true to confirm reset"
    }
  },
  "required": ["confirm"]
}
```

**Output:**
```json
{
  "message": "Session reset. 3 operations discarded.",
  "operations_discarded": 3
}
```

### Schema Inspection

#### `list_tables`

Lists all tables in the current schema.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "include_columns": {
      "type": "boolean",
      "description": "Include column names in output. Default false."
    }
  }
}
```

**Output:**
```json
{
  "tables": [
    {
      "name": "users",
      "columns": ["id", "email", "name", "created_at"]
    },
    {
      "name": "posts",
      "columns": ["id", "user_id", "title", "body"]
    }
  ]
}
```

#### `describe_table`

Returns detailed information about a specific table.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "table_name": {
      "type": "string",
      "description": "Name of the table to describe"
    }
  },
  "required": ["table_name"]
}
```

**Output:**
```json
{
  "name": "users",
  "columns": [
    {
      "name": "id",
      "type": "integer",
      "nullable": false,
      "default": "nextval('users_id_seq')",
      "is_primary_key": true
    },
    {
      "name": "email",
      "type": "character varying(255)",
      "nullable": false,
      "default": null,
      "is_primary_key": false
    }
  ],
  "constraints": [
    {
      "name": "users_pkey",
      "type": "primary_key",
      "columns": ["id"]
    },
    {
      "name": "users_email_key",
      "type": "unique",
      "columns": ["email"]
    }
  ],
  "indexes": [
    {
      "name": "users_pkey",
      "columns": ["id"],
      "is_unique": true,
      "is_primary": true
    }
  ],
  "foreign_keys_outgoing": [],
  "foreign_keys_incoming": [
    {
      "from_table": "posts",
      "from_columns": ["user_id"],
      "to_columns": ["id"],
      "on_delete": "CASCADE"
    }
  ]
}
```

### Table Operations

#### `create_table`

Creates a new table with the specified columns.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "name": {
      "type": "string",
      "description": "Name of the table to create"
    },
    "columns": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "name": { "type": "string" },
          "type": { "type": "string", "description": "PostgreSQL data type" },
          "nullable": { "type": "boolean", "default": true },
          "default": { "type": "string", "description": "Default value expression" },
          "primary_key": { "type": "boolean", "default": false }
        },
        "required": ["name", "type"]
      },
      "description": "List of columns to create"
    },
    "primary_key": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Column names for composite primary key (alternative to per-column)"
    }
  },
  "required": ["name", "columns"]
}
```

**Example Input:**
```json
{
  "name": "preferences",
  "columns": [
    { "name": "id", "type": "SERIAL", "primary_key": true },
    { "name": "user_id", "type": "INTEGER", "nullable": false },
    { "name": "theme", "type": "VARCHAR(50)", "default": "'light'" },
    { "name": "notifications", "type": "BOOLEAN", "default": "true" }
  ]
}
```

**Output:**
```json
{
  "success": true,
  "table_name": "preferences",
  "sql_executed": "CREATE TABLE preferences (\n  id SERIAL PRIMARY KEY,\n  user_id INTEGER NOT NULL,\n  theme VARCHAR(50) DEFAULT 'light',\n  notifications BOOLEAN DEFAULT true\n);",
  "message": "Created table 'preferences' with 4 columns"
}
```

**Behavior:**
1. Generate CREATE TABLE SQL from input
2. Execute in PGLite
3. Record operation in session history
4. Return success with executed SQL

#### `drop_table`

Drops an existing table.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "name": {
      "type": "string",
      "description": "Name of the table to drop"
    },
    "cascade": {
      "type": "boolean",
      "default": false,
      "description": "Also drop dependent objects (foreign keys, views)"
    }
  },
  "required": ["name"]
}
```

**Output:**
```json
{
  "success": true,
  "table_name": "old_logs",
  "sql_executed": "DROP TABLE old_logs CASCADE;",
  "warning": "This is a destructive operation that will cause data loss.",
  "dependent_objects_dropped": ["fk_logs_user"]
}
```

### Column Operations

#### `add_column`

Adds a column to an existing table.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "table": {
      "type": "string",
      "description": "Name of the table"
    },
    "column": {
      "type": "object",
      "properties": {
        "name": { "type": "string" },
        "type": { "type": "string" },
        "nullable": { "type": "boolean", "default": true },
        "default": { "type": "string" }
      },
      "required": ["name", "type"]
    }
  },
  "required": ["table", "column"]
}
```

**Example Input:**
```json
{
  "table": "users",
  "column": {
    "name": "avatar_url",
    "type": "TEXT",
    "nullable": true
  }
}
```

**Output:**
```json
{
  "success": true,
  "sql_executed": "ALTER TABLE users ADD COLUMN avatar_url TEXT;",
  "message": "Added column 'avatar_url' to table 'users'"
}
```

#### `alter_column_type`

Changes a column's data type.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "table": { "type": "string" },
    "column": { "type": "string" },
    "new_type": { "type": "string" },
    "using": {
      "type": "string",
      "description": "USING expression for type conversion"
    }
  },
  "required": ["table", "column", "new_type"]
}
```

**Output:**
```json
{
  "success": true,
  "sql_executed": "ALTER TABLE users ALTER COLUMN age TYPE BIGINT;",
  "warning": "Type changes may require data conversion. Verify with get_breaking_changes."
}
```

### Constraint Operations

#### `add_foreign_key`

Adds a foreign key constraint.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "table": {
      "type": "string",
      "description": "Table containing the foreign key column(s)"
    },
    "columns": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Column(s) in the source table"
    },
    "references_table": {
      "type": "string",
      "description": "Table being referenced"
    },
    "references_columns": {
      "type": "array",
      "items": { "type": "string" },
      "description": "Column(s) in the referenced table"
    },
    "on_delete": {
      "type": "string",
      "enum": ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"],
      "default": "NO ACTION"
    },
    "on_update": {
      "type": "string",
      "enum": ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"],
      "default": "NO ACTION"
    },
    "constraint_name": {
      "type": "string",
      "description": "Optional constraint name. Auto-generated if not provided."
    }
  },
  "required": ["table", "columns", "references_table", "references_columns"]
}
```

**Example Input:**
```json
{
  "table": "preferences",
  "columns": ["user_id"],
  "references_table": "users",
  "references_columns": ["id"],
  "on_delete": "CASCADE"
}
```

**Output:**
```json
{
  "success": true,
  "constraint_name": "preferences_user_id_fkey",
  "sql_executed": "ALTER TABLE preferences ADD CONSTRAINT preferences_user_id_fkey FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;",
  "message": "Added foreign key from preferences.user_id to users.id"
}
```

### Index Operations

#### `create_index`

Creates an index on a table.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
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
    "name": {
      "type": "string",
      "description": "Index name. Auto-generated if not provided."
    },
    "unique": { "type": "boolean", "default": false },
    "method": {
      "type": "string",
      "enum": ["btree", "hash", "gin", "gist", "brin"],
      "default": "btree"
    },
    "where": {
      "type": "string",
      "description": "Partial index predicate"
    },
    "concurrent": {
      "type": "boolean",
      "default": false,
      "description": "Create index concurrently (not supported in PGLite)"
    }
  },
  "required": ["table", "columns"]
}
```

**Example Input:**
```json
{
  "table": "posts",
  "columns": ["user_id", { "name": "created_at", "order": "DESC" }],
  "name": "idx_posts_user_recent"
}
```

**Output:**
```json
{
  "success": true,
  "index_name": "idx_posts_user_recent",
  "sql_executed": "CREATE INDEX idx_posts_user_recent ON posts (user_id, created_at DESC);"
}
```

### Enum Operations

#### `create_enum`

Creates a new enum type.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "name": { "type": "string" },
    "values": {
      "type": "array",
      "items": { "type": "string" },
      "minItems": 1
    }
  },
  "required": ["name", "values"]
}
```

**Example Input:**
```json
{
  "name": "order_status",
  "values": ["pending", "processing", "shipped", "delivered", "cancelled"]
}
```

**Output:**
```json
{
  "success": true,
  "sql_executed": "CREATE TYPE order_status AS ENUM ('pending', 'processing', 'shipped', 'delivered', 'cancelled');"
}
```

#### `add_enum_value`

Adds a value to an existing enum.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "enum_name": { "type": "string" },
    "value": { "type": "string" },
    "before": { "type": "string", "description": "Insert before this value" },
    "after": { "type": "string", "description": "Insert after this value" }
  },
  "required": ["enum_name", "value"]
}
```

**Output:**
```json
{
  "success": true,
  "sql_executed": "ALTER TYPE order_status ADD VALUE 'returned' AFTER 'delivered';"
}
```

### Migration Output

#### `get_migration_preview`

Returns the migration SQL without committing.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "include_transaction": {
      "type": "boolean",
      "default": true,
      "description": "Wrap in BEGIN/COMMIT"
    },
    "include_comments": {
      "type": "boolean",
      "default": true,
      "description": "Include explanatory comments"
    }
  }
}
```

**Output:**
```json
{
  "sql": "BEGIN;\n\n-- Create table: preferences\nCREATE TABLE preferences (\n  id SERIAL PRIMARY KEY,\n  ...\n);\n\n-- Add column: users.avatar_url\nALTER TABLE users ADD COLUMN avatar_url TEXT;\n\nCOMMIT;",
  "operation_count": 2,
  "has_breaking_changes": false,
  "summary": "Creates 1 table, adds 1 column"
}
```

#### `get_breaking_changes`

Returns details about any destructive or breaking changes.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {}
}
```

**Output:**
```json
{
  "has_breaking_changes": true,
  "changes": [
    {
      "type": "column_dropped",
      "table": "users",
      "column": "legacy_field",
      "mitigation": "Destructive",
      "message": "Dropping column will permanently delete data"
    },
    {
      "type": "column_type_changed",
      "table": "users",
      "column": "age",
      "from_type": "INTEGER",
      "to_type": "SMALLINT",
      "mitigation": "DualWrite",
      "message": "Type narrowing may cause data truncation"
    }
  ]
}
```

#### `commit_migration`

Saves the migration to `.tern/migrations/`.

**Input Schema:**
```json
{
  "type": "object",
  "properties": {
    "description": {
      "type": "string",
      "description": "Human-readable description of the migration"
    },
    "force": {
      "type": "boolean",
      "default": false,
      "description": "Commit even with breaking changes"
    }
  },
  "required": ["description"]
}
```

**Output:**
```json
{
  "success": true,
  "migration_id": "00005",
  "migration_path": ".tern/migrations/00005.json",
  "sql_preview": "CREATE TABLE preferences (...);",
  "message": "Migration 00005 committed: 'Add user preferences table'"
}
```

## Error Handling

### Error Response Format

All tools return errors in a consistent format:

```json
{
  "error": {
    "code": "TABLE_NOT_FOUND",
    "message": "Table 'nonexistent' does not exist",
    "details": {
      "available_tables": ["users", "posts"]
    },
    "suggestion": "Did you mean 'users'?"
  }
}
```

### Error Categories

| Code | Description | Example |
|------|-------------|---------|
| `NO_SESSION` | No active session | Call `session_start` first |
| `TABLE_NOT_FOUND` | Referenced table doesn't exist | Table 'foo' not found |
| `COLUMN_NOT_FOUND` | Referenced column doesn't exist | Column 'bar' not in table |
| `DUPLICATE_OBJECT` | Object already exists | Table 'users' already exists |
| `INVALID_TYPE` | Invalid PostgreSQL type | Unknown type 'INTT' |
| `CONSTRAINT_VIOLATION` | Constraint prevents operation | FK references non-existent table |
| `CIRCULAR_DEPENDENCY` | Circular reference detected | Tables reference each other |
| `PGLITE_ERROR` | PGLite execution failed | Syntax error in generated SQL |
| `STATE_ERROR` | Problem with .tern/ state | state.json not found |

### Validation and Suggestions

The server provides helpful suggestions for common errors:

```json
{
  "error": {
    "code": "INVALID_TYPE",
    "message": "Unknown type 'VARCHAR'",
    "suggestion": "Did you mean 'CHARACTER VARYING' or 'VARCHAR(n)'? VARCHAR requires a length."
  }
}
```

## CLI Integration

### Starting the MCP Server

```bash
# Start MCP server on stdio (for local MCP clients)
tern mcp serve

# Start MCP server on HTTP (for remote clients)
tern mcp serve --transport http --port 3000

# Start with specific working directory
tern mcp serve --working-dir /path/to/project
```

### Server Configuration

In `.tern/config.toml`:

```toml
[mcp]
# Enable MCP server
enabled = true

# Default transport
transport = "stdio"

# HTTP transport settings (if using HTTP)
[mcp.http]
port = 3000
host = "127.0.0.1"

# Session settings
[mcp.session]
# Auto-timeout sessions after inactivity
timeout_minutes = 30

# Maximum operations before requiring commit
max_uncommitted_operations = 100
```

## Implementation Plan

### Phase 1: Core MCP Infrastructure

**Goal**: Basic MCP server with session management.

**Tasks**:
1. Add MCP server crate dependencies (`mcp-server` or implement protocol)
2. Implement `McpServer` with stdio transport
3. Implement `SessionManager` with single-session support
4. Add `session_start`, `session_status`, `session_reset` tools
5. Add CLI command `tern mcp serve`

**New files**:
```
src/mcp/
├── mod.rs              # Module root, server setup
├── server.rs           # MCP protocol handling
├── session.rs          # Session state management
└── tools/
    └── session.rs      # Session management tools
```

### Phase 2: Schema Inspection Tools

**Goal**: Tools for examining current schema.

**Tasks**:
1. Implement `list_tables`, `describe_table`
2. Implement `list_enums`, `list_indexes`
3. Implement `get_schema_ddl`

**New files**:
```
src/mcp/tools/
├── mod.rs
├── session.rs
└── inspect.rs          # Inspection tools
```

### Phase 3: Schema Modification Tools

**Goal**: Tools for modifying schema.

**Tasks**:
1. Implement table operations (`create_table`, `drop_table`, `rename_table`)
2. Implement column operations (add, drop, rename, alter)
3. Implement constraint operations (FK, PK, unique, check)
4. Implement index operations
5. Implement enum operations

**New files**:
```
src/mcp/tools/
├── tables.rs           # Table operations
├── columns.rs          # Column operations
├── constraints.rs      # Constraint operations
├── indexes.rs          # Index operations
└── enums.rs            # Enum operations
```

### Phase 4: Migration Output Tools

**Goal**: Tools for generating and committing migrations.

**Tasks**:
1. Implement `get_migration_preview`
2. Implement `get_migration_operations`
3. Implement `get_breaking_changes`
4. Implement `commit_migration`

**New files**:
```
src/mcp/tools/
└── migrate.rs          # Migration output tools
```

### Phase 5: Advanced Features

**Goal**: Polish and advanced capabilities.

**Tasks**:
1. Add HTTP/SSE transport option
2. Implement multi-session support
3. Add undo/redo within session
4. Add operation batching
5. Add schema validation tool

## Appendix: MCP Protocol Considerations

### Protocol Basics

MCP (Model Context Protocol) defines a JSON-RPC based protocol for tool interaction:

```json
// Tool call request
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "create_table",
    "arguments": {
      "name": "preferences",
      "columns": [...]
    }
  }
}

// Tool call response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "{\"success\": true, ...}"
      }
    ]
  }
}
```

### Tool Registration

Tools are registered with the server at startup:

```rust
impl McpServer {
    fn register_tools(&mut self) {
        self.register_tool(Tool {
            name: "create_table",
            description: "Creates a new table with specified columns",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "columns": { "type": "array", ... }
                },
                "required": ["name", "columns"]
            }),
        });
        // ... more tools
    }
}
```

### Rust MCP Libraries

Options for implementing MCP in Rust:

1. **`mcp-rust-sdk`**: Official/community MCP SDK for Rust (if available)
2. **Custom implementation**: Implement JSON-RPC protocol directly
3. **`jsonrpc-core`**: Generic JSON-RPC library as foundation

Recommended approach: Start with `jsonrpc-core` for protocol handling, implement MCP-specific extensions on top.

## Appendix: Example Session

A complete example of using the MCP server:

```
[User starts Claude with Tern MCP server]

User: "I need to add a comments feature. Users should be able to comment on posts."

Claude: Let me check your current schema first.

[Claude calls list_tables]

Claude: You currently have `users` and `posts` tables. I'll create a `comments` table
that references both. Let me:

1. Create the comments table
2. Add appropriate indexes for querying

[Claude calls create_table with:
  name: "comments"
  columns:
    - id SERIAL PRIMARY KEY
    - post_id INTEGER NOT NULL
    - user_id INTEGER NOT NULL
    - body TEXT NOT NULL
    - created_at TIMESTAMPTZ DEFAULT NOW()
]

[Claude calls add_foreign_key: comments.post_id -> posts.id CASCADE]
[Claude calls add_foreign_key: comments.user_id -> users.id CASCADE]
[Claude calls create_index: comments(post_id, created_at DESC)]

Claude: I've created the comments table with:
- Primary key (id)
- Foreign keys to posts and users (both CASCADE on delete)
- An index for efficiently fetching comments by post

Would you like me to generate the migration?

User: "Yes, generate it"

[Claude calls get_migration_preview]

Claude: Here's your migration:

```sql
BEGIN;

CREATE TABLE comments (
  id SERIAL PRIMARY KEY,
  post_id INTEGER NOT NULL,
  user_id INTEGER NOT NULL,
  body TEXT NOT NULL,
  created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

ALTER TABLE comments
  ADD CONSTRAINT comments_post_id_fkey
  FOREIGN KEY (post_id) REFERENCES posts(id) ON DELETE CASCADE;

ALTER TABLE comments
  ADD CONSTRAINT comments_user_id_fkey
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE;

CREATE INDEX idx_comments_post_created
  ON comments (post_id, created_at DESC);

COMMIT;
```

Should I commit this migration?

User: "Yes, commit it with description 'Add comments feature'"

[Claude calls commit_migration with description: "Add comments feature"]

Claude: Migration committed as `00005.json`. You can now run `tern apply`
to apply it to your database.
```
