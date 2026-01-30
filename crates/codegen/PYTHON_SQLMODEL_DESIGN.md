# Python SQLModel Code Generator Design

## Overview

This document outlines the design for implementing a Python code generator that converts PostgreSQL DDL schema definitions (`Vec<Table>`) into SQLModel and Pydantic models. The generator will implement the `Codegen` trait and use the `genco` crate for Python code generation.

## Goals

1. Generate idiomatic SQLModel models from PostgreSQL table definitions
2. Properly handle all PostgreSQL types with appropriate Python type mappings
3. Support primary keys, foreign keys, unique constraints, and indexes
4. Generate both table models (with `table=True`) and optional Pydantic-only models for validation
5. Produce well-formatted, readable Python code with correct imports
6. Handle edge cases robustly (reserved words, special characters, etc.)

## Architecture

### Module Structure

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
    └── snapshots/      # Snapshot test files (managed by insta)
```

### Core Types

```rust
/// Configuration for Python code generation.
#[derive(Debug, Clone)]
pub struct PythonCodegenConfig {
    /// Whether to generate Pydantic-only base classes for each model.
    /// These are useful for request/response validation without DB coupling.
    pub generate_base_models: bool,

    /// Module name prefix for generated imports (e.g., "app.models").
    pub module_prefix: Option<String>,

    /// Whether to include docstrings from table/column comments.
    pub include_docstrings: bool,

    /// How to handle Python reserved words in identifiers.
    pub reserved_word_strategy: ReservedWordStrategy,

    /// Whether to generate relationship attributes for foreign keys.
    pub generate_relationships: bool,
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

| PostgreSQL Type | Python Type | SQLModel Field | Notes |
|-----------------|-------------|----------------|-------|
| `integer`, `int4` | `int` | `Field()` | |
| `bigint`, `int8` | `int` | `Field()` | Python int handles arbitrary precision |
| `smallint`, `int2` | `int` | `Field()` | |
| `serial`, `serial4` | `int` | `Field(default=None, primary_key=True)` | Auto-increment |
| `bigserial`, `serial8` | `int` | `Field(default=None, primary_key=True)` | |
| `boolean`, `bool` | `bool` | `Field()` | |
| `text` | `str` | `Field()` | |
| `varchar(n)`, `character varying` | `str` | `Field(max_length=n)` | Validate length |
| `char(n)`, `character` | `str` | `Field(min_length=n, max_length=n)` | Fixed length |
| `numeric`, `decimal` | `Decimal` | `Field()` | `from decimal import Decimal` |
| `real`, `float4` | `float` | `Field()` | |
| `double precision`, `float8` | `float` | `Field()` | |
| `date` | `date` | `Field()` | `from datetime import date` |
| `time`, `time without time zone` | `time` | `Field()` | `from datetime import time` |
| `timetz`, `time with time zone` | `time` | `Field()` | |
| `timestamp`, `timestamp without time zone` | `datetime` | `Field()` | `from datetime import datetime` |
| `timestamptz`, `timestamp with time zone` | `datetime` | `Field()` | |
| `interval` | `timedelta` | `Field()` | `from datetime import timedelta` |
| `uuid` | `UUID` | `Field()` | `from uuid import UUID` |
| `json` | `Any` | `Field(sa_type=JSON)` | `from typing import Any` |
| `jsonb` | `Any` | `Field(sa_type=JSON)` | |
| `bytea` | `bytes` | `Field()` | |
| `inet` | `str` | `Field()` | IP address as string |
| `cidr` | `str` | `Field()` | |
| `macaddr` | `str` | `Field()` | |
| `point`, `line`, etc. | `str` | `Field()` | Geometric as string |
| `array` (e.g., `integer[]`) | `list[int]` | `Field(sa_type=ARRAY(Integer))` | |
| User-defined enum | Literal union or Python Enum | `Field()` | See enum handling |

### Handling User-Defined Types

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

```python
# Non-nullable without default
name: str

# Nullable (Optional)
bio: str | None = None

# With default value
status: str = Field(default="active")

# Primary key (nullable with None default for auto-generation)
id: int | None = Field(default=None, primary_key=True)

# With index
email: str = Field(index=True)

# With unique constraint
username: str = Field(unique=True)

# Foreign key
user_id: int | None = Field(default=None, foreign_key="users.id")
```

### Generated/Identity Columns

```python
# Identity column (GENERATED ALWAYS AS IDENTITY)
id: int | None = Field(default=None, primary_key=True)
# Note: SQLModel handles auto-increment through primary_key=True with None default

# Generated column (STORED)
# SQLModel doesn't have native support; use sa_column
full_name: str = Field(
    default=None,
    sa_column_kwargs={"server_default": None, "computed": "first_name || ' ' || last_name"}
)
```

### Constraint Handling

#### Primary Key

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

```python
# Basic FK
user_id: int | None = Field(default=None, foreign_key="users.id")

# With relationship
user_id: int | None = Field(default=None, foreign_key="users.id")
user: "User | None" = Relationship(back_populates="posts")
```

#### Unique Constraint

```python
# Single column
email: str = Field(unique=True)

# Composite - requires __table_args__
__table_args__ = (
    UniqueConstraint("email", "tenant_id", name="uq_users_email_tenant"),
)
```

#### Check Constraint

```python
# Via __table_args__
__table_args__ = (
    CheckConstraint("price > 0", name="products_price_positive"),
)
```

### Index Handling

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

For simpler schemas, generate a single `models.py`:

```
models.py
```

### Multi-File Output

For larger schemas, split by table with a shared types module:

```
models/
├── __init__.py       # Re-exports all models
├── _types.py         # Shared types, enums, base classes
├── user.py           # User model
├── post.py           # Post model
└── comment.py        # Comment model
```

**Configuration**: Let users choose via `OutputMode::SingleFile` or `OutputMode::MultiFile`.

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

```python
"""SQLModel definitions generated by Tern."""

from datetime import datetime
from typing import TYPE_CHECKING

from sqlmodel import Field, SQLModel

if TYPE_CHECKING:
    from .post import Post  # For relationship type hints


class User(SQLModel, table=True):
    """User model."""

    __tablename__ = "users"

    id: int | None = Field(default=None, primary_key=True)
    email: str = Field(unique=True)
    name: str | None = None
    created_at: datetime = Field(default_factory=datetime.now, index=True)

    # Relationships (if generate_relationships=True)
    # posts: list["Post"] = Relationship(back_populates="user")
```

## Edge Cases

### 1. Python Reserved Words

Python reserved words that might appear as column/table names:

```
False, None, True, and, as, assert, async, await, break, class, continue,
def, del, elif, else, except, finally, for, from, global, if, import, in,
is, lambda, nonlocal, not, or, pass, raise, return, try, while, with, yield
```

**Handling**:
```python
# Column named "class"
class_: str = Field(alias="class")
```

Use SQLAlchemy column aliasing to preserve the database column name while using a valid Python identifier.

### 2. Invalid Python Identifiers

- Names starting with numbers: `1column` -> `column_1` or `_1column`
- Names with special characters: `column-name` -> `column_name`
- Names with spaces: `column name` -> `column_name`

```rust
fn sanitize_identifier(name: &str) -> String {
    // 1. Replace invalid characters with underscores
    // 2. Ensure doesn't start with digit
    // 3. Handle reserved words
}
```

### 3. Circular Foreign Key References

Tables may reference each other:

```python
# Forward reference using string annotation
class User(SQLModel, table=True):
    id: int | None = Field(default=None, primary_key=True)
    manager_id: int | None = Field(default=None, foreign_key="users.id")

    # Self-referential relationship
    manager: "User | None" = Relationship(
        back_populates="direct_reports",
        sa_relationship_kwargs={"remote_side": "User.id"}
    )
    direct_reports: list["User"] = Relationship(back_populates="manager")
```

### 4. Schema-Qualified Names

Foreign keys might reference tables in other schemas:

```python
# Reference to other_schema.other_table
other_id: int | None = Field(default=None, foreign_key="other_schema.other_table.id")
```

### 5. Array Types

```python
from sqlalchemy import ARRAY, Integer
from sqlmodel import Field, SQLModel

class Document(SQLModel, table=True):
    tags: list[str] = Field(
        default_factory=list,
        sa_type=ARRAY(String)
    )
```

### 6. JSONB Fields

```python
from sqlalchemy import JSON
from typing import Any

class Settings(SQLModel, table=True):
    config: dict[str, Any] = Field(default_factory=dict, sa_type=JSON)
```

### 7. Composite Primary Keys

```python
class OrderItem(SQLModel, table=True):
    __tablename__ = "order_items"

    order_id: int = Field(primary_key=True)
    product_id: int = Field(primary_key=True)
    quantity: int
```

### 8. Generated Columns

SQLModel doesn't have first-class support for generated columns, but we can use `sa_column`:

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

Not directly supported by SQLModel; emit a warning comment:

```python
# WARNING: Exclusion constraint 'meeting_room_no_overlap' not supported by SQLModel.
# Original: EXCLUDE USING gist (room_id WITH =, tsrange(start_time, end_time) WITH &&)
```

### 10. Empty Tables

Tables with no columns (rare but possible):

```python
class EmptyTable(SQLModel, table=True):
    """Table with no columns (placeholder)."""
    __tablename__ = "empty_table"
    pass  # SQLModel requires at least one field in practice
```

**Emit warning**: Tables with no columns should emit a warning as SQLModel requires at least one field.

## Implementation Plan

### Phase 1: Core Infrastructure

1. Add `genco` dependency to `tern-codegen/Cargo.toml`
2. Create module structure under `src/python/`
3. Implement `PythonCodegenConfig` and `PythonCodegen` struct
4. Implement basic `Codegen` trait with empty generation

### Phase 2: Type Mapping

1. Implement `type_mapping.rs` with PostgreSQL -> Python conversions
2. Handle all scalar types from the mapping table
3. Add array type detection and handling
4. Add tests for type mapping edge cases

### Phase 3: Name Handling

1. Implement `naming.rs` with identifier sanitization
2. Build reserved word detection and handling
3. Implement table/column name conversion to Python conventions
4. Add tests for naming edge cases

### Phase 4: Basic Model Generation

1. Implement simple class generation with genco
2. Generate basic fields (non-nullable, no constraints)
3. Handle nullable fields with `Optional` types
4. Generate proper imports

### Phase 5: Constraint Support

1. Primary key generation
2. Foreign key generation (without relationships)
3. Unique constraint generation (single-column via Field, multi-column via `__table_args__`)
4. Check constraint generation via `__table_args__`
5. Index generation

### Phase 6: Relationship Generation

1. Analyze foreign key graph to determine relationship directions
2. Generate `Relationship()` attributes
3. Handle self-referential relationships
4. Handle circular references with forward declarations

### Phase 7: Advanced Features

1. Generated column support
2. Identity column support
3. Array type support
4. JSONB field support
5. Docstring generation from comments

### Phase 8: Testing

1. Unit tests for each component
2. Snapshot tests for complete model generation
3. Edge case tests (reserved words, special characters, circular refs)
4. Integration tests with complex multi-table schemas

## Dependencies

Add to `crates/codegen/Cargo.toml`:

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

Test individual components in isolation:

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

Use `insta` for snapshot testing of generated code:

```rust
#[test]
fn snapshot_simple_table() {
    let tables = vec![create_users_table()];
    let codegen = PythonCodegen::new(PythonCodegenConfig::default());
    let output = codegen.generate(tables);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}

#[test]
fn snapshot_foreign_key_relationship() {
    let tables = vec![create_users_table(), create_posts_table()];
    let codegen = PythonCodegen::new(PythonCodegenConfig {
        generate_relationships: true,
        ..Default::default()
    });
    let output = codegen.generate(tables);

    insta::assert_snapshot!(output.get("models.py").unwrap());
}
```

### Edge Case Tests

```rust
#[test]
fn test_reserved_word_column() {
    let table = Table {
        name: TableName::try_new("items".to_string()).unwrap(),
        columns: vec![
            column("class", "text"),  // Reserved word
            column("from", "integer"), // Reserved word
        ],
        ..Default::default()
    };
    // Verify generated code uses aliases
}

#[test]
fn test_self_referential_foreign_key() {
    let table = Table {
        name: TableName::try_new("employees".to_string()).unwrap(),
        columns: vec![
            column("id", "integer"),
            column("manager_id", "integer"),
        ],
        constraints: vec![
            Constraint::foreign_key("manager_id", "employees", "id"),
        ],
        ..Default::default()
    };
    // Verify self-referential handling
}

#[test]
fn test_circular_foreign_keys() {
    let user_table = /* ... references posts.featured_user_id */;
    let post_table = /* ... references users.id */;
    // Verify circular reference handling with forward declarations
}
```

## Error Handling

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

## Open Questions

1. **Output Mode**: Should we default to single-file or multi-file output?
   - **Recommendation**: Single file for simplicity, with config option for multi-file.

2. **Relationship Generation**: Should relationships be opt-in or opt-out?
   - **Recommendation**: Opt-in via config flag, as relationships add complexity.

3. **Type Hints Style**: Should we use `Optional[X]` or `X | None`?
   - **Recommendation**: `X | None` (Python 3.10+ syntax) as it's more modern and SQLModel targets newer Python.

4. **Enum Handling**: Literal types vs Python Enum classes?
   - **Recommendation**: This design assumes enums are not passed in the `Vec<Table>` input. If enum support is needed, add a separate `enums` parameter to the generator or include enum definitions in a preprocessing step.

## Future Enhancements

1. **Alembic Migration Generation**: Generate Alembic migration files alongside models
2. **Pydantic V2 Schemas**: Generate pure Pydantic models for API schemas
3. **FastAPI Integration**: Generate FastAPI route stubs for CRUD operations
4. **Custom Validators**: Support for custom Pydantic validators from check constraints
5. **Type Stubs**: Generate `.pyi` stub files for better IDE support
