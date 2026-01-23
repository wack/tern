//! Schema checksum computation using xxhash3.
//!
//! This module provides tools for computing deterministic checksums of database
//! schemas. The checksum algorithm (xxhash3) is fast and produces high-quality
//! 64-bit hashes suitable for change detection.
//!
//! # Architecture
//!
//! The checksum computation is split into two parts:
//!
//! 1. **Canonical byte generation** (caller's responsibility): Convert schema
//!    objects into a deterministic byte sequence. This must be done identically
//!    regardless of the data source (CLI queries, WASM interface, etc.).
//!
//! 2. **Hash computation** (this module): Compute xxhash3 over the canonical bytes.
//!
//! # Canonical Format
//!
//! The [`SchemaHasher`] provides a structured way to build canonical input.
//! Callers should add schema objects in a consistent order:
//!
//! 1. Namespaces (sorted by name)
//! 2. Within each namespace: enums, sequences, tables, views (each sorted by name)
//! 3. Within each table: columns (by position), constraints (by name), indexes (by name)
//!
//! # Example
//!
//! ```
//! use tern_migration_wit::checksum::SchemaHasher;
//!
//! let mut hasher = SchemaHasher::new();
//!
//! // Add schema objects in deterministic order
//! hasher.add_namespace("public");
//! hasher.add_table("users", "regular");
//! hasher.add_column("id", "integer", false, None);
//! hasher.add_column("name", "text", false, None);
//! hasher.end_table();
//! hasher.end_namespace();
//!
//! let checksum = hasher.finish();
//! println!("Schema checksum: {:016x}", checksum);
//! ```
//!
//! # Determinism Requirements
//!
//! For checksums to be reproducible across different contexts (CLI vs WASM,
//! different machines, etc.), callers must ensure:
//!
//! - Objects are added in sorted order (by name or position as appropriate)
//! - String representations are normalized (e.g., consistent whitespace in SQL)
//! - Optional fields are handled consistently (explicit markers for None vs empty)

use twox_hash::XxHash3_64;

/// A hasher for computing deterministic schema checksums.
///
/// This type accumulates schema object data and produces an xxhash3 checksum.
/// Objects must be added in a consistent, sorted order to ensure deterministic
/// results across different execution contexts.
///
/// # Object Markers
///
/// The hasher uses single-byte markers to delimit different object types,
/// ensuring that the byte stream is unambiguous:
///
/// - `0x01`: Namespace start
/// - `0x02`: Namespace end
/// - `0x10`: Enum type
/// - `0x11`: Enum value
/// - `0x20`: Sequence
/// - `0x30`: Table start
/// - `0x31`: Table end
/// - `0x32`: Column
/// - `0x33`: Constraint
/// - `0x34`: Index
/// - `0x40`: View
pub struct SchemaHasher {
    buffer: Vec<u8>,
}

// Object type markers for unambiguous byte stream
const MARKER_NAMESPACE_START: u8 = 0x01;
const MARKER_NAMESPACE_END: u8 = 0x02;
const MARKER_ENUM: u8 = 0x10;
const MARKER_ENUM_VALUE: u8 = 0x11;
const MARKER_SEQUENCE: u8 = 0x20;
const MARKER_TABLE_START: u8 = 0x30;
const MARKER_TABLE_END: u8 = 0x31;
const MARKER_COLUMN: u8 = 0x32;
const MARKER_CONSTRAINT: u8 = 0x33;
const MARKER_INDEX: u8 = 0x34;
const MARKER_VIEW: u8 = 0x40;

impl SchemaHasher {
    /// Creates a new schema hasher.
    #[must_use]
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Adds raw bytes to the hash input.
    ///
    /// This is a low-level method. Prefer the structured methods like
    /// [`add_namespace`](Self::add_namespace) for most use cases.
    pub fn update(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Adds a length-prefixed string to the hash input.
    ///
    /// The string is encoded as: `[4-byte little-endian length][utf-8 bytes]`
    fn add_string(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.buffer
            .extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.buffer.extend_from_slice(bytes);
    }

    /// Adds an optional string to the hash input.
    ///
    /// Encodes as: `[0x00]` for None, `[0x01][string]` for Some.
    fn add_option_string(&mut self, s: Option<&str>) {
        match s {
            None => self.buffer.push(0x00),
            Some(s) => {
                self.buffer.push(0x01);
                self.add_string(s);
            }
        }
    }

    /// Marks the start of a namespace (schema).
    ///
    /// All objects added after this call until [`end_namespace`](Self::end_namespace)
    /// are considered part of this namespace.
    pub fn add_namespace(&mut self, name: &str) {
        self.buffer.push(MARKER_NAMESPACE_START);
        self.add_string(name);
    }

    /// Marks the end of the current namespace.
    pub fn end_namespace(&mut self) {
        self.buffer.push(MARKER_NAMESPACE_END);
    }

    /// Adds an enum type definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The enum type name
    /// * `values` - The enum values in order (must be consistent across calls)
    pub fn add_enum(&mut self, name: &str, values: &[&str]) {
        self.buffer.push(MARKER_ENUM);
        self.add_string(name);
        self.buffer
            .extend_from_slice(&(values.len() as u32).to_le_bytes());
        for value in values {
            self.buffer.push(MARKER_ENUM_VALUE);
            self.add_string(value);
        }
    }

    /// Adds a sequence definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The sequence name
    /// * `data_type` - The sequence data type (e.g., "bigint")
    /// * `start` - Start value
    /// * `increment` - Increment value
    /// * `min` - Minimum value
    /// * `max` - Maximum value
    /// * `cache` - Cache size
    /// * `cycle` - Whether the sequence cycles
    #[allow(clippy::too_many_arguments)]
    pub fn add_sequence(
        &mut self,
        name: &str,
        data_type: &str,
        start: i64,
        increment: i64,
        min: i64,
        max: i64,
        cache: i64,
        cycle: bool,
    ) {
        self.buffer.push(MARKER_SEQUENCE);
        self.add_string(name);
        self.add_string(data_type);
        self.buffer.extend_from_slice(&start.to_le_bytes());
        self.buffer.extend_from_slice(&increment.to_le_bytes());
        self.buffer.extend_from_slice(&min.to_le_bytes());
        self.buffer.extend_from_slice(&max.to_le_bytes());
        self.buffer.extend_from_slice(&cache.to_le_bytes());
        self.buffer.push(cycle as u8);
    }

    /// Marks the start of a table definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The table name
    /// * `kind` - The table kind (e.g., "regular", "partitioned")
    pub fn add_table(&mut self, name: &str, kind: &str) {
        self.buffer.push(MARKER_TABLE_START);
        self.add_string(name);
        self.add_string(kind);
    }

    /// Marks the end of the current table.
    pub fn end_table(&mut self) {
        self.buffer.push(MARKER_TABLE_END);
    }

    /// Adds a column definition to the current table.
    ///
    /// # Arguments
    ///
    /// * `name` - The column name
    /// * `type_formatted` - The formatted type (e.g., "character varying(255)")
    /// * `is_nullable` - Whether the column allows NULL
    /// * `default` - The default expression, if any
    pub fn add_column(
        &mut self,
        name: &str,
        type_formatted: &str,
        is_nullable: bool,
        default: Option<&str>,
    ) {
        self.buffer.push(MARKER_COLUMN);
        self.add_string(name);
        self.add_string(type_formatted);
        self.buffer.push(is_nullable as u8);
        self.add_option_string(default);
    }

    /// Adds a column definition with full details.
    ///
    /// This is the complete version that includes identity and generated column info.
    ///
    /// # Arguments
    ///
    /// * `name` - The column name
    /// * `type_formatted` - The formatted type
    /// * `is_nullable` - Whether the column allows NULL
    /// * `default` - The default expression, if any
    /// * `identity` - Identity specification (e.g., "always", "by_default"), if any
    /// * `generated` - Generated column expression, if any
    /// * `collation` - The collation name, if non-default
    #[allow(clippy::too_many_arguments)]
    pub fn add_column_full(
        &mut self,
        name: &str,
        type_formatted: &str,
        is_nullable: bool,
        default: Option<&str>,
        identity: Option<&str>,
        generated: Option<&str>,
        collation: Option<&str>,
    ) {
        self.buffer.push(MARKER_COLUMN);
        self.add_string(name);
        self.add_string(type_formatted);
        self.buffer.push(is_nullable as u8);
        self.add_option_string(default);
        self.add_option_string(identity);
        self.add_option_string(generated);
        self.add_option_string(collation);
    }

    /// Adds a constraint definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The constraint name
    /// * `kind` - The constraint kind (e.g., "primary_key", "foreign_key", "unique", "check")
    /// * `definition` - The constraint definition (columns, expression, etc.)
    pub fn add_constraint(&mut self, name: &str, kind: &str, definition: &str) {
        self.buffer.push(MARKER_CONSTRAINT);
        self.add_string(name);
        self.add_string(kind);
        self.add_string(definition);
    }

    /// Adds an index definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The index name
    /// * `method` - The index method (e.g., "btree", "hash", "gist")
    /// * `definition` - The index definition (columns, expressions, predicates)
    /// * `is_unique` - Whether this is a unique index
    /// * `is_primary` - Whether this backs a primary key constraint
    pub fn add_index(
        &mut self,
        name: &str,
        method: &str,
        definition: &str,
        is_unique: bool,
        is_primary: bool,
    ) {
        self.buffer.push(MARKER_INDEX);
        self.add_string(name);
        self.add_string(method);
        self.add_string(definition);
        self.buffer.push(is_unique as u8);
        self.buffer.push(is_primary as u8);
    }

    /// Adds a view definition.
    ///
    /// # Arguments
    ///
    /// * `name` - The view name
    /// * `definition` - The view SQL definition
    /// * `is_materialized` - Whether this is a materialized view
    pub fn add_view(&mut self, name: &str, definition: &str, is_materialized: bool) {
        self.buffer.push(MARKER_VIEW);
        self.add_string(name);
        self.add_string(definition);
        self.buffer.push(is_materialized as u8);
    }

    /// Computes and returns the final checksum.
    ///
    /// This consumes the hasher. The returned value is a 64-bit xxhash3 digest.
    #[must_use]
    pub fn finish(self) -> u64 {
        XxHash3_64::oneshot(&self.buffer)
    }

    /// Computes the checksum and returns it as a hexadecimal string.
    ///
    /// This consumes the hasher.
    #[must_use]
    pub fn finish_hex(self) -> String {
        format!("{:016x}", self.finish())
    }
}

impl Default for SchemaHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes an xxhash3 checksum of the given bytes.
///
/// This is a convenience function for simple cases where the canonical
/// byte representation is already available.
///
/// # Example
///
/// ```
/// use tern_migration_wit::checksum::xxh3_hash;
///
/// let data = b"CREATE TABLE users (id integer PRIMARY KEY)";
/// let hash = xxh3_hash(data);
/// println!("Hash: {:016x}", hash);
/// ```
#[must_use]
pub fn xxh3_hash(data: &[u8]) -> u64 {
    XxHash3_64::oneshot(data)
}

/// Computes an xxhash3 checksum and returns it as a hexadecimal string.
#[must_use]
pub fn xxh3_hash_hex(data: &[u8]) -> String {
    format!("{:016x}", xxh3_hash(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_schema_has_consistent_hash() {
        let hasher1 = SchemaHasher::new();
        let hasher2 = SchemaHasher::new();

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn same_schema_produces_same_hash() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_namespace("public");
        hasher1.add_table("users", "regular");
        hasher1.add_column("id", "integer", false, None);
        hasher1.add_column("name", "text", true, None);
        hasher1.end_table();
        hasher1.end_namespace();

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_namespace("public");
        hasher2.add_table("users", "regular");
        hasher2.add_column("id", "integer", false, None);
        hasher2.add_column("name", "text", true, None);
        hasher2.end_table();
        hasher2.end_namespace();

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn different_schemas_produce_different_hashes() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_namespace("public");
        hasher1.add_table("users", "regular");
        hasher1.add_column("id", "integer", false, None);
        hasher1.end_table();
        hasher1.end_namespace();

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_namespace("public");
        hasher2.add_table("users", "regular");
        hasher2.add_column("id", "bigint", false, None); // Different type
        hasher2.end_table();
        hasher2.end_namespace();

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn column_order_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_namespace("public");
        hasher1.add_table("users", "regular");
        hasher1.add_column("id", "integer", false, None);
        hasher1.add_column("name", "text", true, None);
        hasher1.end_table();
        hasher1.end_namespace();

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_namespace("public");
        hasher2.add_table("users", "regular");
        hasher2.add_column("name", "text", true, None); // Swapped order
        hasher2.add_column("id", "integer", false, None);
        hasher2.end_table();
        hasher2.end_namespace();

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn nullable_difference_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_column("name", "text", true, None);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_column("name", "text", false, None);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn default_value_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_column("status", "text", false, None);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_column("status", "text", false, Some("'active'"));

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn enum_values_order_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_enum("status", &["pending", "active", "done"]);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_enum("status", &["active", "pending", "done"]);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn sequence_parameters_matter() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_sequence("users_id_seq", "bigint", 1, 1, 1, i64::MAX, 1, false);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_sequence("users_id_seq", "bigint", 1, 1, 1, i64::MAX, 10, false); // Different cache

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn constraint_definition_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_constraint("users_pkey", "primary_key", "id");

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_constraint("users_pkey", "primary_key", "id, tenant_id");

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn index_uniqueness_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_index("users_email_idx", "btree", "email", false, false);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_index("users_email_idx", "btree", "email", true, false);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn view_materialization_matters() {
        let mut hasher1 = SchemaHasher::new();
        hasher1.add_view("active_users", "SELECT * FROM users WHERE active", false);

        let mut hasher2 = SchemaHasher::new();
        hasher2.add_view("active_users", "SELECT * FROM users WHERE active", true);

        assert_ne!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn xxh3_hash_is_deterministic() {
        let data = b"CREATE TABLE users (id integer PRIMARY KEY)";
        assert_eq!(xxh3_hash(data), xxh3_hash(data));
    }

    #[test]
    fn xxh3_hash_hex_format() {
        let data = b"test";
        let hex = xxh3_hash_hex(data);
        assert_eq!(hex.len(), 16); // 64 bits = 16 hex chars
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn finish_hex_format() {
        let hasher = SchemaHasher::new();
        let hex = hasher.finish_hex();
        assert_eq!(hex.len(), 16);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
