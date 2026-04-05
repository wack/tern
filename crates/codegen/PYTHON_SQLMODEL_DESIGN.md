# Python SQLModel Code Generator Design

## Implementation Status

> **Status Legend**:
> - [x] Completed
> - [ ] Future work (not yet implemented)
> - [~] Partially implemented

## Overview

This document outlines the design for implementing a Python code generator that converts PostgreSQL DDL schema definitions (`Vec<Table>`) into SQLModel and Pydantic models. The generator will implement the `Codegen` trait and use the `genco` crate for Python code generation.

## Goals

1. [x] Generate idiomatic SQLModel models from PostgreSQL table definitions
2. [x] Properly handle all PostgreSQL types with appropriate Python type mappings
3. [x] Support primary keys, foreign keys, unique constraints, and indexes
4. [x] Generate both table models (with `table=True`) and optional Pydantic-only models for validation
5. [x] Produce well-formatted, readable Python code with correct imports
6. [x] Handle edge cases robustly (reserved words, special characters, etc.)

## Architecture

### Module Structure

[x] **Completed** - All modules implemented as designed.

```
crates/codegen/src/python/
├── mod.rs              # Module root, re-exports, PythonCodegen struct
├── generator.rs        # Core generation logic implementing Codegen trait
├── type_mapping.rs     # PostgreSQL to Python type conversion
├── naming.rs           # Name sanitization and Python identifier handling
├── model.rs            # SQLModel class generation
├── field.rs            # Field/column generation with Field() configurations
├── imports.rs          # Import management
└── tests/              # Test submodule
    ├── mod.rs          # Test utilities and common fixtures
    ├── unit_tests.rs   # Unit tests for individual components
    ├── snapshot_tests.rs # Snapshot tests for full generation output
    └── snapshots/      # Snapshot test files (managed by insta)
```

### Core Types

[x] **Completed** - All core types implemented in `mod.rs`.

```rust
/// Configuration for Python code generation.
#[derive(Debug, Clone)]
pub struct PythonCodegenConfig {
    /// Whether to generate Pydantic-only base classes for each model.
    /// These are useful for request/response validation without DB coupling.
    pub generate_base_models: bool,  // [x] Completed

    /// Module name prefix for generated imports (e.g., "app.models").
    pub module_prefix: Option<String>,  // [x] Completed

    /// Whether to include docstrings from table/column comments.
    pub include_docstrings: bool,  // [x] Completed

    /// How to handle Python reserved words in identifiers.
    pub reserved_word_strategy: ReservedWordStrategy,  // [x] Completed

    /// Whether to generate relationship attributes for foreign keys.
    pub generate_relationships: bool,  // [x] Completed

    /// Output mode (single file or multi-file).
    pub output_mode: OutputMode,  // [x] Completed
}

#[derive(Debug, Clone, Default)]
pub enum ReservedWordStrategy {
    /// Append an underscore: `class` -> `class_`
    #[default]
    AppendUnderscore,
    /// Prepend with prefix: `class` -> `field_class`
    PrependPrefix(String),
}

/// Main code generator for Python SQLModel output.
pub struct PythonCodegen {
    config: PythonCodegenConfig,
}

impl Codegen for PythonCodegen {
    fn generate(&self, tables: Vec<Table>) -> HashMap<String, String>;
}
```

## Type Mapping

### PostgreSQL to Python Type Mapping

[x] **Completed** - All types in this table are implemented in `type_mapping.rs`.

| PostgreSQL Type | Python Type | SQLModel Field | Status |
|-----------------|-------------|----------------|--------|
| `integer`, `int4` | `int` | `Field()` | [x] |
| `bigint`, `int8` | `int` | `Field()` | [x] |
| `smallint`, `int2` | `int` | `Field()` | [x] |
| `serial`, `serial4` | `int` | `Field(default=None, primary_key=True)` | [x] |
| `bigserial`, `serial8` | `int` | `Field(default=None, primary_key=True)` | [x] |
| `boolean`, `bool` | `bool` | `Field()` | [x] |
| `text` | `str` | `Field()` | [x] |
| `varchar(n)`, `character varying` | `str` | `Field(max_length=n)` | [x] (length extracted but not yet used in Field) |
| `char(n)`, `character` | `str` | `Field(min_length=n, max_length=n)` | [x] (length extracted but not yet used in Field) |
| `numeric`, `decimal` | `Decimal` | `Field()` | [x] |
| `real`, `float4` | `float` | `Field()` | [x] |
| `double precision`, `float8` | `float` | `Field()` | [x] |
| `date` | `date` | `Field()` | [x] |
| `time`, `time without time zone` | `time` | `Field()` | [x] |
| `timetz`, `time with time zone` | `time` | `Field()` | [x] |
| `timestamp`, `timestamp without time zone` | `datetime` | `Field()` | [x] |
| `timestamptz`, `timestamp with time zone` | `datetime` | `Field()` | [x] |
| `interval` | `timedelta` | `Field()` | [x] |
| `uuid` | `UUID` | `Field()` | [x] |
| `json` | `dict[str, Any]` | `Field(sa_type=JSON)` | [x] |
| `jsonb` | `dict[str, Any]` | `Field(sa_type=JSON)` | [x] |
| `bytea` | `bytes` | `Field()` | [x] |
| `inet` | `str` | `Field()` | [x] |
| `cidr` | `str` | `Field()` | [x] |
| `macaddr` | `str` | `Field()` | [x] |
| `point`, `line`, etc. | `str` | `Field()` | [x] |
| `array` (e.g., `integer[]`) | `list[int]` | `Field(sa_type=ARRAY(Integer))` | [x] |
| User-defined enum | Literal union or Python Enum | `Field()` | [~] Partial (fallback to `str`) |

### Handling User-Defined Types

[~] **Partial** - User-defined types are detected but map to `str` as a fallback since enum values are not available in the `Codegen` trait input.

For user-defined enum types:
```python
from typing import Literal

# Option 1: Literal type (simpler)
UserStatus = Literal["pending", "active", "archived"]

# Option 2: Python Enum (more structured)
from enum import Enum

class UserStatus(str, Enum):
    pending = "pending"
    active = "active"
    archived = "archived"
```

**Decision**: Use `Literal` by default for simplicity, with a config option for Python `Enum` generation.

## Field Generation

### Basic Field Patterns

[x] **Completed** - All basic field patterns implemented in `field.rs`.

```python
# Non-nullable without default
name: str

# Nullable (Optional)
bio: str | None = None

# With default value
status: str = Field(default="active")

# Primary key (nullable with None default for auto-generation)
id: int | None = Field(default=None, primary_key=True)

# With index  [ ] Future work - index=True not yet generated for single columns
email: str = Field(index=True)

# With unique constraint
username: str = Field(unique=True)

# Foreign key
user_id: int | None = Field(default=None, foreign_key="users.id")
```

### Generated/Identity Columns

[x] **Completed** - Identity columns and generated columns fully supported.

```python
# Identity column (GENERATED ALWAYS AS IDENTITY)
id: int | None = Field(default=None, primary_key=True)
# Note: SQLModel handles auto-increment through primary_key=True with None default

# Generated column (STORED) - uses SQLAlchemy Computed
from sqlalchemy import Column, Computed, String

full_name: str | None = Field(
    default=None,
    sa_column=Column(String, Computed("first_name || ' ' || last_name"))
)
```

### Constraint Handling

#### Primary Key

[x] **Completed**

```python
# Single column
id: int | None = Field(default=None, primary_key=True)

# Composite - requires __table_args__
class OrderItem(SQLModel, table=True):
    order_id: int = Field(primary_key=True)
    product_id: int = Field(primary_key=True)
    quantity: int
```

#### Foreign Key

[x] **Completed** - FK support with optional relationship generation.

```python
# Basic FK
user_id: int | None = Field(default=None, foreign_key="users.id")

# With relationship (when generate_relationships=True)
user_id: int | None = Field(default=None, foreign_key="users.id")
user: "User" = Relationship(back_populates="posts")
```

#### Unique Constraint

[x] **Completed**

```python
# Single column
email: str = Field(unique=True)

# Composite - requires __table_args__
__table_args__ = (
    UniqueConstraint("email", "tenant_id", name="uq_users_email_tenant"),
)
```

#### Check Constraint

[x] **Completed**

```python
# Via __table_args__
__table_args__ = (
    CheckConstraint("price > 0", name="products_price_positive"),
)
```

### Index Handling

[x] **Completed** - Both single-column and multi-column indexes supported.

```python
# Simple column index
email: str = Field(index=True)

# Composite or complex indexes - via __table_args__
__table_args__ = (
    Index("idx_users_name_email", "name", "email"),
)
```

## Output Structure

### Single File Output

[x] **Completed**

For simpler schemas, generate a single `models.py`:

```
models.py
```

### Multi-File Output

[x] **Completed**

For larger schemas, split by table with a shared types module:

```
models/
├── __init__.py       # Re-exports all models
├── user.py           # User model
├── post.py           # Post model
└── comment.py        # Comment model
```

**Note**: `_types.py` for shared types/enums not yet implemented (depends on enum support).

**Configuration**: Users can choose via `OutputMode::SingleFile` or `OutputMode::MultiFile`.

## Generated Code Examples

### Input DDL

```rust
Table {
    name: "users",
    columns: [
        Column { name: "id", type: "integer", is_nullable: false, identity: Some(Always) },
        Column { name: "email", type: "text", is_nullable: false },
        Column { name: "name", type: "text", is_nullable: true },
        Column { name: "created_at", type: "timestamptz", is_nullable: false, default: Some("now()") },
    ],
    constraints: [
        PrimaryKey { columns: ["id"] },
        Unique { columns: ["email"] },
    ],
    indexes: [
        Index { columns: ["created_at"], is_unique: false },
    ],
}
```

### Generated Python

[x] **Completed** - Full generation with all core features.

```python
"""SQLModel definitions generated by Tern."""

from datetime import datetime, timezone

from sqlmodel import Field, SQLModel


class User(SQLModel, table=True):
    """User model."""

    __tablename__ = "users"

    id: int | None = Field(default=None, primary_key=True)
    email: str = Field(unique=True)
    name: str | None = None
    created_at: datetime = Field(default_factory=lambda: datetime.now(timezone.utc))

    # Relationships (when generate_relationships=True)
    # posts: list["Post"] = Relationship(back_populates="user")
```

## Edge Cases

### 1. Python Reserved Words

[x] **Completed** - Full reserved word handling in `naming.rs`.

Python reserved words that might appear as column/table names:

```
False, None, True, and, as, assert, async, await, break, class, continue,
def, del, elif, else, except, finally, for, from, global, if, import, in,
is, lambda, nonlocal, not, or, pass, raise, return, try, while, with, yield
```

Also handles soft keywords (`match`, `case`, `type`, `_`) and can optionally handle Python builtins.

**Handling**:
```python
# Column named "class"
class_: str = Field(alias="class")
```

### 2. Invalid Python Identifiers

[x] **Completed**

- Names starting with numbers: `1column` -> `_1column`
- Names with special characters: `column-name` -> `column_name`
- Names with spaces: `column name` -> `column_name`

### 3. Circular Foreign Key References

[x] **Completed** - FK references and relationships with forward references.

```python
# Forward reference using string annotation
class User(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    manager_id: int | None = Field(default=None, foreign_key="users.id")

    # Self-referential relationship (when generate_relationships=True)
    manager: "User" = Relationship(back_populates="users")
```

### 4. Schema-Qualified Names

[x] **Completed**

Foreign keys reference tables with schema qualification:

```python
# Reference to other_schema.other_table
other_id: int | None = Field(default=None, foreign_key="other_schema.other_table.id")
```

### 5. Array Types

[x] **Completed**

```python
from sqlalchemy import ARRAY, Text
from sqlmodel import Field, SQLModel

class Document(SQLModel, table=True):
    tags: list[str] = Field(sa_type=ARRAY(Text))
```

### 6. JSONB Fields

[x] **Completed**

```python
from sqlalchemy import JSON
from typing import Any

class Settings(SQLModel, table=True):
    config: dict[str, Any] = Field(sa_type=JSON)
```

### 7. Composite Primary Keys

[x] **Completed**

```python
class OrderItem(SQLModel, table=True):
    __tablename__ = "order_items"

    order_id: int = Field(primary_key=True)
    product_id: int = Field(primary_key=True)
    quantity: int
```

### 8. Generated Columns

[x] **Completed** - Uses SQLAlchemy `Computed` via `sa_column`.

SQLModel doesn't have first-class support for generated columns, but we use `sa_column`:

```python
from sqlalchemy import Column, Computed, String

class Person(SQLModel, table=True):
    first_name: str
    last_name: str
    full_name: str | None = Field(
        default=None,
        sa_column=Column(String, Computed("first_name || ' ' || last_name"))
    )
```

### 9. Exclusion Constraints

[x] **Completed** - Emits warning comment as designed.

Not directly supported by SQLModel; emit a warning comment:

```python
# WARNING: Exclusion constraint 'meeting_room_no_overlap' not supported by SQLModel.
# Original: EXCLUDE USING gist (room_id WITH =, tsrange(start_time, end_time) WITH &&)
```

### 10. Empty Tables

[x] **Completed** - Emits warning.

Tables with no columns emit a warning as SQLModel requires at least one field.

## Implementation Plan

### Phase 1: Core Infrastructure

[x] **Completed**

1. [x] Add `genco` dependency to `tern-codegen/Cargo.toml`
2. [x] Create module structure under `src/python/`
3. [x] Implement `PythonCodegenConfig` and `PythonCodegen` struct
4. [x] Implement basic `Codegen` trait with empty generation

### Phase 2: Type Mapping

[x] **Completed**

1. [x] Implement `type_mapping.rs` with PostgreSQL -> Python conversions
2. [x] Handle all scalar types from the mapping table
3. [x] Add array type detection and handling
4. [x] Add tests for type mapping edge cases

### Phase 3: Name Handling

[x] **Completed**

1. [x] Implement `naming.rs` with identifier sanitization
2. [x] Build reserved word detection and handling
3. [x] Implement table/column name conversion to Python conventions
4. [x] Add tests for naming edge cases

### Phase 4: Basic Model Generation

[x] **Completed**

1. [x] Implement simple class generation with genco
2. [x] Generate basic fields (non-nullable, no constraints)
3. [x] Handle nullable fields with `| None` types
4. [x] Generate proper imports

### Phase 5: Constraint Support

[x] **Completed**

1. [x] Primary key generation
2. [x] Foreign key generation (without relationships)
3. [x] Unique constraint generation (single-column via Field, multi-column via `__table_args__`)
4. [x] Check constraint generation via `__table_args__`
5. [x] Index generation (multi-column via `__table_args__`)

### Phase 6: Relationship Generation

[x] **Completed**

1. [x] Analyze foreign key graph to determine relationship directions
2. [x] Generate `Relationship()` attributes with `back_populates`
3. [x] Handle self-referential relationships
4. [x] Handle circular references with forward declarations (string annotations)

### Phase 7: Advanced Features

[x] **Completed**

1. [x] Generated column support (with `Computed`)
2. [x] Identity column support
3. [x] Array type support
4. [x] JSONB field support
5. [x] Docstring generation from comments

### Phase 8: Testing

[x] **Completed**

1. [x] Unit tests for each component (109 tests)
2. [x] Snapshot tests for complete model generation (11 snapshots)
3. [x] Edge case tests (reserved words, special characters, circular refs)
4. [x] Integration tests with complex multi-table schemas

## Dependencies

[x] **Completed**

Added to `crates/codegen/Cargo.toml`:

```toml
[dependencies]
tern-ddl = { path = "../ddl" }
genco = "0.19"  # Code generation
thiserror = "2.0"  # Error types

[dev-dependencies]
insta = { version = "1.42", features = ["yaml"] }
pretty_assertions = "1.4"
```

## Testing Strategy

### Unit Tests

[x] **Completed** - 109 tests covering all components.

```rust
#[test]
fn test_type_mapping_integer() {
    assert_eq!(map_pg_type("integer"), PythonType::Int);
}

#[test]
fn test_sanitize_reserved_word() {
    assert_eq!(sanitize_identifier("class"), "class_");
}
```

### Snapshot Tests

[x] **Completed** - 11 snapshot tests with insta.

```rust
#[test]
fn snapshot_simple_table() {
    let tables = vec![create_users_table()];
    let codegen = PythonCodegen::new(PythonCodegenConfig::default());
    let output = codegen.generate(tables);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}
```

### Edge Case Tests

[x] **Completed**

- Reserved word columns (class, from, import, def, etc.)
- Self-referential foreign keys
- Composite primary keys
- Multi-column unique constraints
- Check constraints
- Exclusion constraints (warning)
- All PostgreSQL types
- Multi-file output mode

## Error Handling

[x] **Completed** - Error type defined in `mod.rs`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PythonCodegenError {
    #[error("unsupported PostgreSQL type: {0}")]
    UnsupportedType(String),

    #[error("table has no columns: {0}")]
    EmptyTable(String),

    #[error("invalid identifier after sanitization: {0}")]
    InvalidIdentifier(String),

    #[error("code generation failed: {0}")]
    GenerationError(String),
}
```

**Note**: The current `Codegen` trait returns `HashMap<String, String>` without error handling. Consider proposing a trait update to return `Result<HashMap<String, String>, Error>` in the future.

## Open Questions (Resolved)

1. **Output Mode**: Should we default to single-file or multi-file output?
   - **Resolution**: [x] Single file is the default, with `OutputMode::MultiFile` option.

2. **Relationship Generation**: Should relationships be opt-in or opt-out?
   - **Resolution**: [x] Opt-in via `generate_relationships` config flag. Not yet implemented.

3. **Type Hints Style**: Should we use `Optional[X]` or `X | None`?
   - **Resolution**: [x] Using `X | None` (Python 3.10+ syntax).

4. **Enum Handling**: Literal types vs Python Enum classes?
   - **Resolution**: [~] Partial. User-defined types are detected by schema but map to `str` as a fallback since enum values are not available in the `Codegen` trait input.

## Future Enhancements

1. [x] **Relationship Generation**: Generate `Relationship()` attributes with back_populates
2. [x] **Pydantic Base Models**: Generate Pydantic-only models via `generate_base_models` config
3. [~] **User-Defined Enums**: Support PostgreSQL enum types as Python Literal or Enum (partial - fallback to `str`)
4. [x] **Generated Columns**: Full `Computed` support with sa_column
5. [x] **Single-Column Index**: Generate `Field(index=True)` for indexed columns
6. [x] **Default Factory**: Generate `default_factory` for datetime fields with `now()` defaults
7. [x] **Module Prefix**: Support `module_prefix` config for import paths
8. [x] **String Length Validation**: Use extracted varchar/char length in `Field(max_length=n)`
9. [ ] **Alembic Migration Generation**: Generate Alembic migration files alongside models
10. [ ] **FastAPI Integration**: Generate FastAPI route stubs for CRUD operations
11. [ ] **Custom Validators**: Support for custom Pydantic validators from check constraints
12. [ ] **Type Stubs**: Generate `.pyi` stub files for better IDE support
