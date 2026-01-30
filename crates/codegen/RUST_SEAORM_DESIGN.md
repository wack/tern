# Rust SeaORM Code Generator Design

## Overview

This document outlines the design for implementing a Rust code generator that converts PostgreSQL DDL schema definitions (`Vec<Table>`) into SeaORM entity modules. The generator will implement the `Codegen` trait and use the `genco` crate for Rust code generation.

## Goals

1. Generate idiomatic SeaORM entities from PostgreSQL table definitions
2. Properly handle all PostgreSQL types with appropriate Rust/SeaORM type mappings
3. Support primary keys (single and composite), foreign keys, unique constraints, and indexes
4. Generate both compact (DeriveEntityModel) and expanded entity formats
5. Produce well-formatted, readable Rust code with correct imports
6. Handle edge cases robustly (reserved words, special characters, self-referential FKs, etc.)
7. Leverage SeaORM's derive macros for minimal boilerplate

## Architecture

### Module Structure

```
crates/codegen/src/rust/
├── mod.rs                  # Module root, re-exports, RustSeaOrmCodegen struct
├── generator.rs            # Core generation logic implementing Codegen trait
├── type_mapping.rs         # PostgreSQL to Rust/SeaORM type conversion
├── naming.rs               # Name sanitization and Rust identifier handling
├── entity.rs               # Entity struct and trait generation
├── column.rs               # Column enum and field generation
├── primary_key.rs          # PrimaryKey enum generation
├── relation.rs             # Relation enum and Related trait generation
├── active_enum.rs          # ActiveEnum generation for PostgreSQL enums
├── imports.rs              # Import management using genco
└── tests/                  # Test submodule
    ├── mod.rs              # Test utilities and common fixtures
    ├── unit_tests.rs       # Unit tests for individual components
    └── snapshots/          # Snapshot test files (managed by insta)
```

### Core Types

```rust
/// Configuration for Rust SeaORM code generation.
#[derive(Debug, Clone)]
pub struct RustSeaOrmCodegenConfig {
    /// Whether to generate compact format (DeriveEntityModel) or expanded format.
    /// Compact is recommended and is the default.
    pub entity_format: EntityFormat,

    /// Whether to generate relationship attributes and Related trait implementations.
    pub generate_relations: bool,

    /// Whether to include doc comments from table/column comments.
    pub include_doc_comments: bool,

    /// How to handle Rust reserved words in identifiers.
    pub reserved_word_strategy: ReservedWordStrategy,

    /// Module name for the generated entities (used for cross-module references).
    pub module_name: Option<String>,

    /// Whether to generate ActiveEnum types for PostgreSQL enums.
    /// Note: Requires enum definitions to be passed separately or inferred.
    pub generate_active_enums: bool,

    /// The schema name to use (if not public).
    pub schema_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub enum EntityFormat {
    /// Uses DeriveEntityModel macro - recommended, less boilerplate.
    #[default]
    Compact,
    /// Generates explicit Column enum, PrimaryKey enum, and trait implementations.
    Expanded,
}

#[derive(Debug, Clone, Default)]
pub enum ReservedWordStrategy {
    /// Append an underscore: `type` -> `type_` with column_name attribute.
    #[default]
    AppendUnderscore,
    /// Prepend with prefix: `type` -> `r#type` (raw identifier).
    RawIdentifier,
    /// Prepend with custom prefix: `type` -> `field_type`.
    PrependPrefix(String),
}

/// Main code generator for Rust SeaORM output.
pub struct RustSeaOrmCodegen {
    config: RustSeaOrmCodegenConfig,
}

impl Codegen for RustSeaOrmCodegen {
    fn generate(&self, tables: Vec<Table>) -> HashMap<String, String>;
}
```

## Type Mapping

### PostgreSQL to Rust/SeaORM Type Mapping

| PostgreSQL Type | Rust Type | SeaORM ColumnType | Notes |
|-----------------|-----------|-------------------|-------|
| `integer`, `int4` | `i32` | `ColumnType::Integer` | |
| `bigint`, `int8` | `i64` | `ColumnType::BigInteger` | |
| `smallint`, `int2` | `i16` | `ColumnType::SmallInteger` | |
| `serial`, `serial4` | `i32` | `ColumnType::Integer` | Auto-increment via `#[sea_orm(primary_key)]` |
| `bigserial`, `serial8` | `i64` | `ColumnType::BigInteger` | |
| `smallserial`, `serial2` | `i16` | `ColumnType::SmallInteger` | |
| `boolean`, `bool` | `bool` | `ColumnType::Boolean` | |
| `text` | `String` | `ColumnType::Text` | |
| `varchar(n)`, `character varying` | `String` | `ColumnType::String(StringLen::N(n))` | |
| `char(n)`, `character` | `String` | `ColumnType::Char(Some(n))` | Fixed length |
| `numeric`, `decimal` | `rust_decimal::Decimal` | `ColumnType::Decimal(Some((p, s)))` | Feature: `with-rust_decimal` |
| `real`, `float4` | `f32` | `ColumnType::Float` | |
| `double precision`, `float8` | `f64` | `ColumnType::Double` | |
| `date` | `chrono::NaiveDate` | `ColumnType::Date` | Feature: `with-chrono` |
| `time`, `time without time zone` | `chrono::NaiveTime` | `ColumnType::Time` | Feature: `with-chrono` |
| `timetz`, `time with time zone` | `chrono::NaiveTime` | `ColumnType::Time` | Tz info lost in SeaORM |
| `timestamp`, `timestamp without time zone` | `chrono::NaiveDateTime` | `ColumnType::DateTime` | Feature: `with-chrono` |
| `timestamptz`, `timestamp with time zone` | `chrono::DateTime<FixedOffset>` | `ColumnType::TimestampWithTimeZone` | Feature: `with-chrono` |
| `interval` | `String` | `ColumnType::Interval(None, None)` | No native Rust equivalent |
| `uuid` | `uuid::Uuid` | `ColumnType::Uuid` | Feature: `with-uuid` |
| `json` | `serde_json::Value` | `ColumnType::Json` | Feature: `with-json` |
| `jsonb` | `serde_json::Value` | `ColumnType::JsonBinary` | Feature: `with-json` |
| `bytea` | `Vec<u8>` | `ColumnType::Binary(BlobSize::Blob(None))` | |
| `inet` | `ipnetwork::IpNetwork` | `ColumnType::Inet` | Feature: `with-ipnetwork` |
| `cidr` | `ipnetwork::IpNetwork` | `ColumnType::Cidr` | Feature: `with-ipnetwork` |
| `macaddr` | `mac_address::MacAddress` | `ColumnType::MacAddr` | Feature: `with-mac_address` |
| `point` | `String` | `ColumnType::Custom("point")` | Geometric as string |
| `line`, `lseg`, `box`, `path`, `polygon`, `circle` | `String` | `ColumnType::Custom(...)` | Geometric types |
| `integer[]` | `Vec<i32>` | `ColumnType::Array(RcOrArc::new(ColumnType::Integer))` | PostgreSQL-only |
| `text[]` | `Vec<String>` | `ColumnType::Array(RcOrArc::new(ColumnType::Text))` | |
| User-defined enum | Custom `ActiveEnum` | `ColumnType::Enum { ... }` | See enum handling |
| `money` | `rust_decimal::Decimal` | `ColumnType::Money(Some((19, 2)))` | Feature: `with-rust_decimal` |
| `bit(n)`, `bit varying(n)` | `String` | `ColumnType::Bit(Some(n))` | |
| `xml` | `String` | `ColumnType::Custom("xml")` | |
| `tsvector` | `String` | `ColumnType::Custom("tsvector")` | Full-text search |
| `tsquery` | `String` | `ColumnType::Custom("tsquery")` | |

### Type Parsing Strategy

The `TypeInfo` struct from `tern-ddl` provides:
- `name`: The base type name (e.g., `int4`, `varchar`)
- `schema`: The schema containing the type (e.g., `pg_catalog`)
- `formatted`: The full formatted type with modifiers (e.g., `character varying(255)`)
- `is_array`: Whether this is an array type

```rust
/// Maps a PostgreSQL TypeInfo to a Rust type and SeaORM column type.
pub fn map_pg_type(type_info: &TypeInfo) -> TypeMapping {
    // Parse formatted string to extract precision/scale for numeric types
    // Handle array types by wrapping base type
    // Return both Rust type string and SeaORM ColumnType
}

pub struct TypeMapping {
    /// The Rust type as a string (e.g., "i32", "Option<String>")
    pub rust_type: String,
    /// The SeaORM ColumnType expression (for expanded format)
    pub column_type: String,
    /// Required feature flags for Cargo.toml
    pub required_features: Vec<&'static str>,
    /// Required imports (module path, type name)
    pub imports: Vec<(&'static str, &'static str)>,
}
```

### Handling Nullability

```rust
// Non-nullable column
pub id: i32,

// Nullable column - wrap in Option
pub description: Option<String>,
```

In SeaORM:
- Non-nullable fields use the raw type
- Nullable fields use `Option<T>`
- The `nullable` attribute can be explicit: `#[sea_orm(nullable)]`

## Entity Generation

### Compact Format (Default)

The compact format uses `DeriveEntityModel` which automatically generates the Column enum, PrimaryKey enum, and necessary trait implementations.

```rust
use sea_orm::entity::prelude::*;

/// User accounts table.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub email: String,
    pub name: Option<String>,
    #[sea_orm(column_type = "TimestampWithTimeZone")]
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}

impl Related<super::post::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Posts.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

### Expanded Format

The expanded format provides explicit control over all generated code, useful for complex customizations.

```rust
use sea_orm::entity::prelude::*;

#[derive(Copy, Clone, Default, Debug, DeriveEntity)]
pub struct Entity;

impl EntityName for Entity {
    fn schema_name(&self) -> Option<&str> {
        None // or Some("my_schema")
    }

    fn table_name(&self) -> &str {
        "users"
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveColumn)]
pub enum Column {
    Id,
    Email,
    Name,
    CreatedAt,
}

impl ColumnTrait for Column {
    type EntityName = Entity;

    fn def(&self) -> ColumnDef {
        match self {
            Self::Id => ColumnType::Integer.def(),
            Self::Email => ColumnType::String(StringLen::None).def().unique(),
            Self::Name => ColumnType::String(StringLen::None).def().nullable(),
            Self::CreatedAt => ColumnType::TimestampWithTimeZone.def(),
        }
    }
}

#[derive(Copy, Clone, Debug, EnumIter, DerivePrimaryKey)]
pub enum PrimaryKey {
    Id,
}

impl PrimaryKeyTrait for PrimaryKey {
    type ValueType = i32;

    fn auto_increment() -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveModel, DeriveActiveModel)]
pub struct Model {
    pub id: i32,
    pub email: String,
    pub name: Option<String>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}

impl Related<super::post::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Posts.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

## Column Generation

### Basic Column Patterns

```rust
// Non-nullable, no special attributes
pub name: String,

// Nullable
pub bio: Option<String>,

// Primary key (auto-increment)
#[sea_orm(primary_key)]
pub id: i32,

// Primary key (not auto-increment)
#[sea_orm(primary_key, auto_increment = false)]
pub uuid: uuid::Uuid,

// Unique constraint
#[sea_orm(unique)]
pub email: String,

// Indexed column
#[sea_orm(indexed)]
pub username: String,

// Column name override (for reserved words or non-snake_case names)
#[sea_orm(column_name = "type")]
pub type_: String,

// Custom column type
#[sea_orm(column_type = "Text")]
pub long_text: String,

// JSON column
#[sea_orm(column_type = "JsonBinary")]
pub metadata: serde_json::Value,

// Array column (PostgreSQL only)
pub tags: Vec<String>,
```

### Column Attribute Combinations

```rust
/// Determines which #[sea_orm(...)] attributes to generate for a column.
pub fn generate_column_attributes(
    column: &Column,
    constraints: &[Constraint],
    indexes: &[Index],
) -> Vec<String> {
    let mut attrs = Vec::new();

    // Check if column is part of primary key
    if is_primary_key_column(column, constraints) {
        attrs.push("primary_key".to_string());
        if !is_auto_increment(column) {
            attrs.push("auto_increment = false".to_string());
        }
    }

    // Check for unique constraint (single-column only)
    if has_unique_constraint(column, constraints) {
        attrs.push("unique".to_string());
    }

    // Check for index (single-column only)
    if has_index(column, indexes) {
        attrs.push("indexed".to_string());
    }

    // Column name override if needed
    if needs_column_name_override(column) {
        attrs.push(format!("column_name = \"{}\"", column.name.as_ref()));
    }

    // Custom column type if not inferable
    if let Some(ct) = custom_column_type(column) {
        attrs.push(format!("column_type = \"{}\"", ct));
    }

    attrs
}
```

### Generated and Identity Columns

SeaORM handles generated columns through the `Computed` attribute in the expanded format:

```rust
// Generated column (STORED)
// Note: SeaORM doesn't have first-class support for generated columns
// We emit the column as read-only with a comment
/// Generated column: first_name || ' ' || last_name
#[sea_orm(ignore)]
pub full_name: String,
```

Identity columns map to primary keys with auto-increment:

```rust
// GENERATED ALWAYS AS IDENTITY
#[sea_orm(primary_key)]
pub id: i32,

// GENERATED BY DEFAULT AS IDENTITY
#[sea_orm(primary_key)]
pub id: i32,
```

## Primary Key Handling

### Single Column Primary Key

```rust
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    // ...
}
```

### Composite Primary Key

```rust
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "order_items")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub order_id: i32,
    #[sea_orm(primary_key, auto_increment = false)]
    pub product_id: i32,
    pub quantity: i32,
}
```

### Non-Auto-Increment Primary Keys

For UUID or other non-auto-increment primary keys:

```rust
#[sea_orm(primary_key, auto_increment = false)]
pub id: uuid::Uuid,
```

## Relation Generation

### Determining Relation Direction

Foreign keys define the relationship direction:
- The table with the FK has `belongs_to` (many-to-one)
- The referenced table has `has_many` or `has_one` (one-to-many or one-to-one)

```rust
/// Analyzes foreign keys to determine relation types.
pub fn analyze_relations(
    tables: &[Table],
) -> HashMap<TableName, Vec<RelationInfo>> {
    let mut relations: HashMap<TableName, Vec<RelationInfo>> = HashMap::new();

    for table in tables {
        for constraint in &table.constraints {
            if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
                // Table with FK: belongs_to
                relations
                    .entry(table.name.clone())
                    .or_default()
                    .push(RelationInfo {
                        kind: RelationKind::BelongsTo,
                        target_table: fk.referenced_table.clone(),
                        from_columns: fk.columns.clone(),
                        to_columns: fk.referenced_columns.clone(),
                    });

                // Referenced table: has_many (or has_one if FK is unique)
                let ref_kind = if is_one_to_one(fk, &table.constraints) {
                    RelationKind::HasOne
                } else {
                    RelationKind::HasMany
                };

                relations
                    .entry(fk.referenced_table.table.clone())
                    .or_default()
                    .push(RelationInfo {
                        kind: ref_kind,
                        target_table: QualifiedTableName::new(
                            table.name.clone(),
                            // schema handling...
                        ),
                        from_columns: fk.referenced_columns.clone(),
                        to_columns: fk.columns.clone(),
                    });
            }
        }
    }

    relations
}
```

### Generating Relation Enum

```rust
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    // has_many: This table is referenced by post.user_id
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,

    // belongs_to: This table has a foreign key to another table
    #[sea_orm(
        belongs_to = "super::team::Entity",
        from = "Column::TeamId",
        to = "super::team::Column::Id"
    )]
    Team,
}
```

### Generating Related Trait

```rust
impl Related<super::post::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Posts.def()
    }
}

impl Related<super::team::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Team.def()
    }
}
```

### Many-to-Many Relations

For junction tables, SeaORM uses the `via()` method:

```rust
// In cake entity, relating to filling through cake_filling
impl Related<super::filling::Entity> for Entity {
    fn to() -> RelationDef {
        super::cake_filling::Relation::Filling.def()
    }

    fn via() -> Option<RelationDef> {
        Some(super::cake_filling::Relation::Cake.def().rev())
    }
}
```

**Detection heuristic**: A table is likely a junction table if:
1. It has exactly two foreign keys
2. Those foreign keys together form the primary key
3. It has few or no other non-FK columns

### Composite Foreign Keys

```rust
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::composite_pk::Entity",
        from = "(Column::LeftId, Column::RightId)",
        to = "(super::composite_pk::Column::LeftId, super::composite_pk::Column::RightId)"
    )]
    CompositePk,
}
```

### Self-Referential Relations

```rust
// Employee with manager_id referencing the same table
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "Entity",
        from = "Column::ManagerId",
        to = "Column::Id"
    )]
    Manager,
}

impl Related<Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Manager.def()
    }
}
```

## Constraint Handling

### Unique Constraints

Single-column unique constraints use the `unique` attribute:

```rust
#[sea_orm(unique)]
pub email: String,
```

Multi-column unique constraints require `__table_args__` equivalent, which in SeaORM is handled via the expanded format's `ColumnTrait`:

```rust
impl ColumnTrait for Column {
    fn def(&self) -> ColumnDef {
        match self {
            Self::Email => ColumnType::String(StringLen::None).def().unique(),
            // ...
        }
    }
}
```

For composite unique constraints, a comment is generated:

```rust
// Composite unique constraint: uq_users_email_tenant (email, tenant_id)
// Note: Composite unique constraints are enforced at database level
```

### Check Constraints

SeaORM doesn't have native check constraint support. Generate comments:

```rust
// Check constraint: products_price_positive (price > 0)
// Note: Check constraints are enforced at database level
```

### Exclusion Constraints

Exclusion constraints are PostgreSQL-specific and not supported by SeaORM:

```rust
// WARNING: Exclusion constraint 'meeting_room_no_overlap' not supported by SeaORM.
// Original: EXCLUDE USING gist (room_id WITH =, tsrange(start_time, end_time) WITH &&)
```

## Index Handling

### Single-Column Indexes

```rust
#[sea_orm(indexed)]
pub username: String,
```

### Multi-Column and Complex Indexes

Generated as comments since SeaORM doesn't support declarative index creation:

```rust
// Index: idx_users_name_email (name, email)
// Index: idx_users_created_at_desc (created_at DESC)
// Partial index: idx_active_users (email) WHERE active = true
```

## Enum Handling

### ActiveEnum Generation

For PostgreSQL enum types, generate `ActiveEnum`:

```rust
use sea_orm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "user_status")]
pub enum UserStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "archived")]
    Archived,
}
```

Using the enum in a model:

```rust
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub status: UserStatus,
}
```

### Enum Type Detection

Since enums aren't part of `Vec<Table>`, we have two options:

1. **Infer from column types**: If `type_info.schema` is not `pg_catalog`, it might be a user-defined enum
2. **Accept enum definitions separately**: Add an optional `enums: Vec<EnumType>` parameter or configuration

```rust
/// Configuration includes optional enum definitions.
pub struct RustSeaOrmCodegenConfig {
    /// User-defined PostgreSQL enums to generate as ActiveEnum.
    pub enums: Vec<EnumDefinition>,
    // ...
}

pub struct EnumDefinition {
    pub name: String,
    pub schema: Option<String>,
    pub values: Vec<String>,
}
```

## Output Structure

### Single File Output

For simpler schemas, generate a single `entities.rs`:

```
entities.rs
```

### Multi-File Output (Default)

For larger schemas, generate one file per entity with a `mod.rs`:

```
entities/
├── mod.rs                  # Re-exports all entities
├── prelude.rs              # Common imports (Entity, Model, ActiveModel, Column)
├── user.rs                 # User entity
├── post.rs                 # Post entity
├── comment.rs              # Comment entity
└── enums/                  # ActiveEnum definitions (if any)
    ├── mod.rs
    └── user_status.rs
```

**mod.rs example:**

```rust
//! SeaORM entity definitions generated by Tern.

pub mod prelude;

pub mod user;
pub mod post;
pub mod comment;

pub mod enums;
```

**prelude.rs example:**

```rust
//! Re-exports commonly used types.

pub use super::user::Entity as User;
pub use super::post::Entity as Post;
pub use super::comment::Entity as Comment;
```

## Edge Cases

### 1. Rust Reserved Words

Rust reserved words and keywords that might appear as column/table names:

```
as, async, await, break, const, continue, crate, dyn, else, enum, extern,
false, fn, for, if, impl, in, let, loop, match, mod, move, mut, pub, ref,
return, self, Self, static, struct, super, trait, true, type, unsafe, use,
where, while, abstract, become, box, do, final, macro, override, priv,
typeof, unsized, virtual, yield, try, union
```

**Handling:**

```rust
// Column named "type" - use raw identifier
#[sea_orm(column_name = "type")]
pub r#type: String,

// Or with underscore suffix
#[sea_orm(column_name = "type")]
pub type_: String,
```

### 2. Invalid Rust Identifiers

- Names starting with numbers: `1column` -> `_1column` or `column_1`
- Names with special characters: `column-name` -> `column_name`
- Names with spaces: `column name` -> `column_name`

```rust
/// Sanitizes a database identifier for use as a Rust identifier.
pub fn sanitize_identifier(name: &str, strategy: &ReservedWordStrategy) -> SanitizedName {
    let mut result = String::new();

    // Handle leading digit
    let chars: Vec<char> = name.chars().collect();
    if chars.first().is_some_and(|c| c.is_ascii_digit()) {
        result.push('_');
    }

    // Replace invalid characters
    for c in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            result.push(c);
        } else {
            result.push('_');
        }
    }

    // Handle reserved words
    if is_rust_reserved_word(&result) {
        match strategy {
            ReservedWordStrategy::AppendUnderscore => {
                result.push('_');
            }
            ReservedWordStrategy::RawIdentifier => {
                return SanitizedName {
                    identifier: format!("r#{}", result),
                    needs_column_name_attr: true,
                    original: name.to_string(),
                };
            }
            ReservedWordStrategy::PrependPrefix(prefix) => {
                result = format!("{}{}", prefix, result);
            }
        }
    }

    SanitizedName {
        identifier: result,
        needs_column_name_attr: result != name,
        original: name.to_string(),
    }
}
```

### 3. Circular Foreign Key References

Tables may reference each other:

```rust
// In user.rs - references post for featured_post_id
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
    #[sea_orm(
        belongs_to = "super::post::Entity",
        from = "Column::FeaturedPostId",
        to = "super::post::Column::Id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    FeaturedPost,
}
```

The key is using `super::` module paths which work regardless of definition order.

### 4. Schema-Qualified Names

For multi-schema databases:

```rust
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(schema_name = "inventory", table_name = "products")]
pub struct Model {
    // ...
}

// Foreign key to different schema
#[sea_orm(
    belongs_to = "crate::entities::public::user::Entity",
    from = "Column::UserId",
    to = "crate::entities::public::user::Column::Id"
)]
User,
```

### 5. Array Types (PostgreSQL-only)

```rust
// Integer array
pub tags: Vec<i32>,

// Nullable string array
pub categories: Option<Vec<String>>,
```

### 6. JSON/JSONB Fields

```rust
use serde_json::Value as JsonValue;

#[sea_orm(column_type = "JsonBinary")]
pub metadata: JsonValue,

// Nullable JSON
#[sea_orm(column_type = "Json")]
pub config: Option<JsonValue>,
```

For typed JSON (requires custom type):

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
pub struct Settings {
    pub theme: String,
    pub notifications: bool,
}

// In model
pub settings: Settings,
```

### 7. Generated Columns

SeaORM has limited support for generated columns. Generate as ignored with comment:

```rust
/// Generated column (STORED): first_name || ' ' || last_name
/// Note: This column is computed by the database and should not be set directly.
#[sea_orm(ignore)]
pub full_name: String,
```

Or omit entirely with a warning comment at the model level.

### 8. Empty Tables

Tables with no columns are invalid in SeaORM (and rare in practice):

```rust
// WARNING: Table 'empty_table' has no columns and cannot be represented in SeaORM.
// Skipping entity generation for this table.
```

### 9. Tables Without Primary Keys

SeaORM requires a primary key. For tables without one:

```rust
// WARNING: Table 'log_entries' has no primary key.
// SeaORM requires a primary key for entity operations.
// Consider adding a primary key or using this table with raw SQL queries.
```

Options:
1. Skip the table with a warning
2. Generate a read-only entity (no ActiveModel operations)
3. Use a synthetic primary key (all columns)

### 10. Very Long Identifiers

PostgreSQL allows identifiers up to 63 bytes. Rust has no hard limit but very long names are unwieldy:

```rust
/// Truncates identifier if necessary, ensuring uniqueness.
pub fn truncate_identifier(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        // Truncate and add hash suffix for uniqueness
        let hash = calculate_short_hash(name);
        format!("{}_{}", &name[..max_len - 9], hash)
    }
}
```

### 11. Conflicting Relation Names

When multiple foreign keys reference the same table:

```rust
// user_id and reviewer_id both reference users
#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::ReviewerId",
        to = "super::user::Column::Id"
    )]
    Reviewer,
}
```

The relation name is derived from the FK column name or constraint name.

## Implementation Plan

### Phase 1: Core Infrastructure

1. Add required dependencies to `tern-codegen/Cargo.toml`
2. Create module structure under `src/rust/`
3. Implement `RustSeaOrmCodegenConfig` and `RustSeaOrmCodegen` struct
4. Implement basic `Codegen` trait with minimal generation
5. Set up test infrastructure with fixtures

**Deliverables:**
- `mod.rs` with config types and struct definition
- `generator.rs` skeleton implementing `Codegen`
- Basic test setup

### Phase 2: Type Mapping

1. Implement `type_mapping.rs` with PostgreSQL -> Rust conversions
2. Handle all scalar types from the mapping table
3. Parse type modifiers (precision, scale, length) from `formatted` field
4. Add array type detection and handling
5. Track required feature flags and imports
6. Add comprehensive tests for type mapping

**Deliverables:**
- `type_mapping.rs` with `map_pg_type()` function
- Unit tests covering all PostgreSQL types

### Phase 3: Name Handling

1. Implement `naming.rs` with identifier sanitization
2. Build Rust reserved word detection and handling
3. Implement table/column name conversion (snake_case, PascalCase)
4. Handle edge cases (digits, special chars, conflicts)
5. Add tests for naming edge cases

**Deliverables:**
- `naming.rs` with sanitization functions
- Unit tests for reserved words and edge cases

### Phase 4: Basic Entity Generation (Compact Format)

1. Implement `entity.rs` for Model struct generation
2. Generate basic fields (non-nullable, no constraints)
3. Handle nullable fields with `Option` types
4. Generate proper imports using genco
5. Implement `imports.rs` for import management
6. Generate `ActiveModelBehavior` impl

**Deliverables:**
- `entity.rs` generating complete Model struct
- `imports.rs` managing genco imports
- Snapshot tests for basic entities

### Phase 5: Primary Key and Column Attributes

1. Implement `column.rs` for column attribute generation
2. Implement `primary_key.rs` for primary key detection
3. Handle single and composite primary keys
4. Handle auto-increment vs non-auto-increment
5. Generate `#[sea_orm(...)]` attributes correctly

**Deliverables:**
- `column.rs` with attribute generation
- `primary_key.rs` with PK detection
- Tests for single and composite PKs

### Phase 6: Constraint Support

1. Single-column unique constraints via `#[sea_orm(unique)]`
2. Multi-column unique constraints as comments
3. Check constraints as comments
4. Index attributes for simple indexes
5. Complex indexes as comments

**Deliverables:**
- Constraint handling in `column.rs`
- Warning/comment generation for unsupported constraints
- Tests for various constraint types

### Phase 7: Relation Generation

1. Implement `relation.rs` for relation analysis
2. Generate `Relation` enum with `DeriveRelation`
3. Generate `Related` trait implementations
4. Handle self-referential relations
5. Handle composite foreign keys
6. Detect and handle many-to-many junction tables

**Deliverables:**
- `relation.rs` with full relation generation
- Tests for all relation types

### Phase 8: ActiveEnum Generation

1. Implement `active_enum.rs` for enum generation
2. Generate `DeriveActiveEnum` enums
3. Integrate enum types into model fields
4. Handle enums in separate files

**Deliverables:**
- `active_enum.rs` generating ActiveEnum types
- Tests for enum generation

### Phase 9: Expanded Format Support

1. Extend `entity.rs` for expanded format
2. Generate explicit `Column` enum with `DeriveColumn`
3. Generate `ColumnTrait` implementation
4. Generate `PrimaryKey` enum with `DerivePrimaryKey`
5. Generate `PrimaryKeyTrait` implementation
6. Generate `DeriveModel` and `DeriveActiveModel`

**Deliverables:**
- Expanded format generation
- Tests comparing compact vs expanded output

### Phase 10: Multi-File Output

1. Implement file organization (one entity per file)
2. Generate `mod.rs` with re-exports
3. Generate `prelude.rs` for common imports
4. Handle cross-module references correctly
5. Generate proper module paths

**Deliverables:**
- Multi-file output generation
- Tests for file organization

### Phase 11: Testing and Polish

1. Comprehensive snapshot tests for various schemas
2. Edge case tests (reserved words, circular refs, etc.)
3. Integration tests with complex multi-table schemas
4. Documentation and doc comments
5. Code review and cleanup

**Deliverables:**
- Full test coverage
- Documentation
- Clean, production-ready code

## Dependencies

### Cargo.toml Updates

```toml
[dependencies]
tern-ddl = { path = "../ddl" }
genco = "0.19"
thiserror = "2.0"

[dev-dependencies]
insta = { version = "1.42", features = ["yaml"] }
pretty_assertions = "1.4"
```

### Feature Flags Documentation

The generated code may require feature flags in the user's SeaORM dependency:

```toml
# Example user Cargo.toml
[dependencies]
sea-orm = { version = "1.0", features = [
    "runtime-tokio-rustls",
    "sqlx-postgres",
    "with-chrono",       # For date/time types
    "with-uuid",         # For UUID type
    "with-json",         # For JSON/JSONB types
    "with-rust_decimal", # For DECIMAL/NUMERIC types
    "with-ipnetwork",    # For INET/CIDR types
    "with-mac_address",  # For MACADDR type
] }
```

The generator should output a comment or separate file listing required features.

## Testing Strategy

### Unit Tests

Test individual components in isolation:

```rust
#[test]
fn test_type_mapping_integer() {
    let type_info = TypeInfo {
        name: TypeName::try_new("int4".to_string()).unwrap(),
        schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        formatted: "integer".to_string(),
        is_array: false,
    };
    let mapping = map_pg_type(&type_info);
    assert_eq!(mapping.rust_type, "i32");
    assert_eq!(mapping.column_type, "ColumnType::Integer");
}

#[test]
fn test_sanitize_reserved_word() {
    let result = sanitize_identifier("type", &ReservedWordStrategy::AppendUnderscore);
    assert_eq!(result.identifier, "type_");
    assert!(result.needs_column_name_attr);
}

#[test]
fn test_sanitize_raw_identifier() {
    let result = sanitize_identifier("type", &ReservedWordStrategy::RawIdentifier);
    assert_eq!(result.identifier, "r#type");
    assert!(result.needs_column_name_attr);
}
```

### Snapshot Tests

Use `insta` for snapshot testing of generated code:

```rust
#[test]
fn snapshot_simple_table() {
    let tables = vec![create_users_table()];
    let codegen = RustSeaOrmCodegen::new(RustSeaOrmCodegenConfig::default());
    let output = codegen.generate(tables);

    insta::assert_snapshot!(output.get("user.rs").unwrap());
}

#[test]
fn snapshot_foreign_key_relationship() {
    let tables = vec![create_users_table(), create_posts_table()];
    let config = RustSeaOrmCodegenConfig {
        generate_relations: true,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(tables);

    insta::assert_snapshot!("user", output.get("user.rs").unwrap());
    insta::assert_snapshot!("post", output.get("post.rs").unwrap());
}

#[test]
fn snapshot_expanded_format() {
    let tables = vec![create_users_table()];
    let config = RustSeaOrmCodegenConfig {
        entity_format: EntityFormat::Expanded,
        ..Default::default()
    };
    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(tables);

    insta::assert_snapshot!(output.get("user.rs").unwrap());
}
```

### Edge Case Tests

```rust
#[test]
fn test_reserved_word_column() {
    let table = Table {
        name: TableName::try_new("items".to_string()).unwrap(),
        columns: vec![
            column("type", "text"),
            column("match", "integer"),
        ],
        ..Default::default()
    };
    let output = generate_entity(&table);
    // Verify column_name attributes and escaped identifiers
    assert!(output.contains("#[sea_orm(column_name = \"type\")]"));
}

#[test]
fn test_self_referential_foreign_key() {
    let table = create_table_with_self_reference("employees", "manager_id");
    let output = generate_entity(&table);
    // Verify self-referential relation
    assert!(output.contains("belongs_to = \"Entity\""));
}

#[test]
fn test_circular_foreign_keys() {
    let (user_table, post_table) = create_circular_reference_tables();
    let output = generate_entities(&[user_table, post_table]);
    // Verify both directions of the relationship
}

#[test]
fn test_composite_primary_key() {
    let table = create_junction_table("order_items", "order_id", "product_id");
    let output = generate_entity(&table);
    assert!(output.contains("#[sea_orm(primary_key, auto_increment = false)]"));
    // Should appear twice
}

#[test]
fn test_composite_foreign_key() {
    let table = create_table_with_composite_fk();
    let output = generate_entity(&table);
    assert!(output.contains("from = \"(Column::"));
    assert!(output.contains("to = \"(super::"));
}

#[test]
fn test_nullable_array_column() {
    let table = create_table_with_array_column("documents", "tags", "text[]", true);
    let output = generate_entity(&table);
    assert!(output.contains("pub tags: Option<Vec<String>>"));
}

#[test]
fn test_table_without_primary_key() {
    let table = create_table_without_pk("audit_log");
    let output = generate_entity(&table);
    // Verify warning comment is generated
    assert!(output.contains("WARNING") || output.contains("no primary key"));
}
```

### Integration Tests

```rust
#[test]
fn integration_complex_schema() {
    // Create a realistic multi-table schema
    let tables = vec![
        create_users_table(),
        create_teams_table(),
        create_team_members_table(),  // Junction table
        create_posts_table(),
        create_comments_table(),
        create_tags_table(),
        create_post_tags_table(),  // Junction table
    ];

    let config = RustSeaOrmCodegenConfig {
        generate_relations: true,
        include_doc_comments: true,
        ..Default::default()
    };

    let codegen = RustSeaOrmCodegen::new(config);
    let output = codegen.generate(tables);

    // Verify all expected files are generated
    assert!(output.contains_key("mod.rs"));
    assert!(output.contains_key("prelude.rs"));
    assert!(output.contains_key("user.rs"));
    assert!(output.contains_key("team.rs"));
    // ...

    // Verify the generated code compiles (optional, requires integration test setup)
}
```

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum RustSeaOrmCodegenError {
    #[error("unsupported PostgreSQL type: {type_name} (formatted: {formatted})")]
    UnsupportedType {
        type_name: String,
        formatted: String,
    },

    #[error("table '{table_name}' has no columns")]
    EmptyTable { table_name: String },

    #[error("table '{table_name}' has no primary key")]
    NoPrimaryKey { table_name: String },

    #[error("invalid identifier after sanitization: '{original}' -> '{sanitized}'")]
    InvalidIdentifier { original: String, sanitized: String },

    #[error("circular module dependency detected: {path:?}")]
    CircularDependency { path: Vec<String> },

    #[error("code generation failed: {message}")]
    GenerationError { message: String },
}

/// Warnings that don't prevent generation but should be reported.
#[derive(Debug)]
pub enum RustSeaOrmCodegenWarning {
    UnsupportedConstraint {
        table: String,
        constraint: String,
        kind: String,
    },
    GeneratedColumnIgnored {
        table: String,
        column: String,
    },
    FeatureFlagRequired {
        feature: String,
        reason: String,
    },
}
```

**Note**: The current `Codegen` trait returns `HashMap<String, String>` without error handling. The implementation should collect warnings internally and optionally include them as comments in the generated code or in a separate output file.

## Generated Code Example

### Input

```rust
// Simplified representation of input
Table {
    name: "users",
    columns: [
        Column { name: "id", type: "integer", identity: Some(Always), is_nullable: false },
        Column { name: "email", type: "text", is_nullable: false },
        Column { name: "name", type: "text", is_nullable: true },
        Column { name: "status", type: "user_status", is_nullable: false },
        Column { name: "created_at", type: "timestamptz", is_nullable: false },
        Column { name: "metadata", type: "jsonb", is_nullable: true },
    ],
    constraints: [
        PrimaryKey { columns: ["id"] },
        Unique { columns: ["email"] },
    ],
    indexes: [
        Index { columns: ["created_at"], is_unique: false },
    ],
}

Table {
    name: "posts",
    columns: [
        Column { name: "id", type: "integer", identity: Some(Always) },
        Column { name: "user_id", type: "integer", is_nullable: false },
        Column { name: "title", type: "text", is_nullable: false },
        Column { name: "body", type: "text", is_nullable: true },
    ],
    constraints: [
        PrimaryKey { columns: ["id"] },
        ForeignKey { columns: ["user_id"], referenced_table: "users", referenced_columns: ["id"] },
    ],
}
```

### Output: user.rs

```rust
//! SeaORM entity for `users` table.
//!
//! Generated by Tern.

use sea_orm::entity::prelude::*;

/// User accounts.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    #[sea_orm(unique)]
    pub email: String,
    pub name: Option<String>,
    pub status: super::enums::UserStatus,
    #[sea_orm(indexed)]
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    #[sea_orm(column_type = "JsonBinary", nullable)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::post::Entity")]
    Posts,
}

impl Related<super::post::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Posts.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

### Output: post.rs

```rust
//! SeaORM entity for `posts` table.
//!
//! Generated by Tern.

use sea_orm::entity::prelude::*;

/// Blog posts.
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "posts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub user_id: i32,
    pub title: String,
    pub body: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
```

### Output: enums/user_status.rs

```rust
//! ActiveEnum for `user_status` PostgreSQL enum.
//!
//! Generated by Tern.

use sea_orm::entity::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "user_status")]
pub enum UserStatus {
    #[sea_orm(string_value = "pending")]
    Pending,
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "archived")]
    Archived,
}
```

### Output: mod.rs

```rust
//! SeaORM entity definitions generated by Tern.
//!
//! Required SeaORM features:
//! - `with-chrono` (for DateTime types)
//! - `with-json` (for JSON/JSONB types)

pub mod prelude;

pub mod enums;
pub mod post;
pub mod user;
```

### Output: prelude.rs

```rust
//! Re-exports commonly used entity types.

pub use super::post::Entity as Post;
pub use super::user::Entity as User;
```

## Open Questions

1. **Entity Format Default**: Should we default to compact or expanded format?
   - **Recommendation**: Compact format as it's the modern SeaORM standard and requires less generated code.

2. **Relation Generation**: Should relations be opt-in or opt-out?
   - **Recommendation**: Opt-in via config flag, as relations add complexity and may not always be desired.

3. **Error Handling**: Should the `Codegen` trait be updated to return `Result`?
   - **Recommendation**: Keep the current signature but collect errors/warnings in comments or a separate "warnings.txt" output file.

4. **Enum Handling**: How should we receive enum definitions?
   - **Recommendation**: Accept enums via configuration. If the Namespace type from the model is available, it contains enum definitions.

5. **Feature Flag Documentation**: Should we generate a separate file listing required Cargo features?
   - **Recommendation**: Yes, generate a `REQUIRED_FEATURES.md` or include in mod.rs header comments.

6. **Generated Column Handling**: Should generated columns be completely omitted or included as ignored?
   - **Recommendation**: Include with `#[sea_orm(ignore)]` and doc comment explaining the column is database-computed.

## Future Enhancements

1. **SeaQuery Migration Generation**: Generate SeaQuery migration statements alongside entities
2. **Custom Type Support**: Allow users to provide custom type mappings
3. **Validation**: Generate validation attributes compatible with `validator` crate
4. **GraphQL Integration**: Generate async-graphql compatible derives
5. **API Route Generation**: Generate Axum/Actix route stubs for CRUD operations
6. **Type-Safe Builders**: Generate builder patterns for insert/update operations
7. **Audit Trail Support**: Generate audit column handling (created_at, updated_at, etc.)

## References

- [SeaORM Documentation](https://www.sea-ql.org/SeaORM/)
- [SeaORM Entity Format](https://www.sea-ql.org/SeaORM/docs/generate-entity/entity-format/)
- [SeaORM Expanded Entity Format](https://www.sea-ql.org/SeaORM/docs/internal-design/expanded-entity-format/)
- [SeaORM Column Types](https://www.sea-ql.org/SeaORM/docs/generate-entity/column-types/)
- [SeaORM ActiveEnum](https://www.sea-ql.org/SeaORM/docs/generate-entity/enumeration/)
- [SeaORM Relations](https://www.sea-ql.org/SeaORM/docs/relation/one-to-many/)
- [genco Documentation](https://docs.rs/genco)
- [genco quote! Macro](https://docs.rs/genco/latest/genco/macro.quote.html)
