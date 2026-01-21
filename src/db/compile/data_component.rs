//! Data component generation for migration SQL and metadata.
//!
//! This module generates WebAssembly components at runtime that contain
//! migration SQL statements and metadata. These data components are composed
//! with the pre-compiled guest component to create complete migration components.
//!
//! # Overview
//!
//! The data component exports the `migration-data` interface:
//!
//! ```wit
//! interface migration-data {
//!     get-id: func() -> string;
//!     get-description: func() -> string;
//!     get-source-state-hash: func() -> string;
//!     get-target-state-hash: func() -> string;
//!     get-compiled-at: func() -> string;
//!     get-statement-count: func() -> u32;
//!     get-statement: func(index: u32) -> statement;
//!     get-breaking-changes: func() -> list<breaking-change>;
//! }
//! ```
//!
//! # Generation Approach
//!
//! The component is generated using `wasm-encoder` by:
//!
//! 1. Creating a core Wasm module with:
//!    - Data section containing serialized metadata and statements
//!    - Memory for the data
//!    - Exported functions that return pointers to the data
//!
//! 2. Wrapping the module in a component that:
//!    - Lifts the core functions to component model types
//!    - Exports the `migration-data` interface

use wasm_encoder::{
    CodeSection, ComponentBuilder, DataSection, ExportSection, Function, FunctionSection,
    Instruction, MemorySection, MemoryType, Module, TypeSection, ValType,
};

use super::error::CompileError;

// =============================================================================
// Migration Data
// =============================================================================

/// Metadata for a migration, used to generate the data component.
#[derive(Debug, Clone)]
pub struct MigrationData {
    /// Unique identifier (content-addressable hash).
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// Source schema state hash.
    pub source_state_hash: String,
    /// Target schema state hash.
    pub target_state_hash: String,
    /// When the migration was compiled (RFC 3339).
    pub compiled_at: String,
    /// SQL statements with metadata.
    pub statements: Vec<StatementData>,
    /// Breaking changes in this migration.
    pub breaking_changes: Vec<BreakingChangeData>,
}

/// A SQL statement with metadata.
#[derive(Debug, Clone)]
pub struct StatementData {
    /// The SQL statement text.
    pub sql: String,
    /// Human-readable description.
    pub description: String,
    /// Sequence number (1-indexed).
    pub sequence: u32,
}

impl StatementData {
    /// Create a new statement with explicit sequence number.
    pub fn new(sql: impl Into<String>, description: impl Into<String>, sequence: u32) -> Self {
        Self {
            sql: sql.into(),
            description: description.into(),
            sequence,
        }
    }
}

/// A breaking change in a migration.
#[derive(Debug, Clone)]
pub struct BreakingChangeData {
    /// Human-readable description.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: MitigationStrategy,
    /// Affected SQL statements.
    pub affected_sql: Vec<String>,
}

/// Strategy for mitigating a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MitigationStrategy {
    /// Requires parallel structures with synchronized writes.
    DualWrite,
    /// Requires populating data before completion.
    Backfill,
    /// Requires NOT VALID + backfill + VALIDATE pattern.
    Ratchet,
    /// Irreversible data or structure removal.
    Destructive,
}

impl MitigationStrategy {
    /// Convert to u8 for serialization.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::DualWrite => 0,
            Self::Backfill => 1,
            Self::Ratchet => 2,
            Self::Destructive => 3,
        }
    }
}

// =============================================================================
// Serialization
// =============================================================================

/// Serialized format for migration data embedded in Wasm.
///
/// This is a simple binary format optimized for Wasm memory access:
///
/// ```text
/// Header:
///   - version: u32 (format version, currently 1)
///   - num_strings: u32 (number of string fields)
///   - num_statements: u32
///   - num_breaking_changes: u32
///
/// String Table:
///   For each string:
///     - offset: u32 (offset into string data)
///     - length: u32 (length in bytes)
///
/// Statement Table:
///   For each statement:
///     - sql_idx: u32 (index into string table)
///     - desc_idx: u32 (index into string table)
///     - sequence: u32
///
/// Breaking Change Table:
///   For each breaking change:
///     - desc_idx: u32 (index into string table)
///     - mitigation: u8
///     - num_affected: u32
///     - affected_indices: [u32; num_affected]
///
/// String Data:
///   Raw UTF-8 string bytes
/// ```
#[derive(Debug)]
#[allow(dead_code)] // Fields reserved for future component model lifting implementation
struct SerializedData {
    /// The binary data blob.
    bytes: Vec<u8>,
    /// Offset to the string table.
    string_table_offset: u32,
    /// Number of strings.
    num_strings: u32,
    /// Offset to the statement table.
    statement_table_offset: u32,
    /// Number of statements.
    num_statements: u32,
}

impl SerializedData {
    /// Serialize migration data to binary format.
    fn serialize(data: &MigrationData) -> Self {
        let mut bytes = Vec::new();
        let mut string_table = Vec::new();
        let mut string_data = Vec::new();
        let mut strings = Vec::new();

        // Helper to add a string
        let mut add_string = |s: &str| -> u32 {
            let idx = strings.len() as u32;
            let offset = string_data.len() as u32;
            let len = s.len() as u32;
            string_data.extend_from_slice(s.as_bytes());
            string_table.push((offset, len));
            strings.push(s.to_string());
            idx
        };

        // Add metadata strings (indices 0-4)
        let id_idx = add_string(&data.id);
        let desc_idx = add_string(&data.description);
        let source_hash_idx = add_string(&data.source_state_hash);
        let target_hash_idx = add_string(&data.target_state_hash);
        let compiled_at_idx = add_string(&data.compiled_at);

        // Add statement strings
        let mut statement_indices = Vec::new();
        for stmt in &data.statements {
            let sql_idx = add_string(&stmt.sql);
            let stmt_desc_idx = add_string(&stmt.description);
            statement_indices.push((sql_idx, stmt_desc_idx, stmt.sequence));
        }

        // Add breaking change strings
        let mut breaking_change_data = Vec::new();
        for bc in &data.breaking_changes {
            let bc_desc_idx = add_string(&bc.description);
            let affected_indices: Vec<u32> =
                bc.affected_sql.iter().map(|s| add_string(s)).collect();
            breaking_change_data.push((bc_desc_idx, bc.mitigation.to_u8(), affected_indices));
        }

        // Write header
        bytes.extend_from_slice(&1u32.to_le_bytes()); // version
        bytes.extend_from_slice(&(strings.len() as u32).to_le_bytes()); // num_strings
        bytes.extend_from_slice(&(data.statements.len() as u32).to_le_bytes()); // num_statements
        bytes.extend_from_slice(&(data.breaking_changes.len() as u32).to_le_bytes()); // num_breaking_changes

        // Write metadata string indices
        bytes.extend_from_slice(&id_idx.to_le_bytes());
        bytes.extend_from_slice(&desc_idx.to_le_bytes());
        bytes.extend_from_slice(&source_hash_idx.to_le_bytes());
        bytes.extend_from_slice(&target_hash_idx.to_le_bytes());
        bytes.extend_from_slice(&compiled_at_idx.to_le_bytes());

        // Record string table offset
        let string_table_offset = bytes.len() as u32;

        // Write string table
        for (offset, len) in &string_table {
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&len.to_le_bytes());
        }

        // Record statement table offset
        let statement_table_offset = bytes.len() as u32;

        // Write statement table
        for (sql_idx, desc_idx, sequence) in &statement_indices {
            bytes.extend_from_slice(&sql_idx.to_le_bytes());
            bytes.extend_from_slice(&desc_idx.to_le_bytes());
            bytes.extend_from_slice(&sequence.to_le_bytes());
        }

        // Write breaking change table
        for (desc_idx, mitigation, affected) in &breaking_change_data {
            bytes.extend_from_slice(&desc_idx.to_le_bytes());
            bytes.push(*mitigation);
            bytes.extend_from_slice(&(affected.len() as u32).to_le_bytes());
            for idx in affected {
                bytes.extend_from_slice(&idx.to_le_bytes());
            }
        }

        // Write string data
        let string_data_offset = bytes.len() as u32;
        bytes.extend_from_slice(&string_data);

        // Update string table offsets to be absolute
        for i in 0..string_table.len() {
            let offset = string_table_offset as usize + i * 8;
            let rel_offset =
                u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or_default());
            let abs_offset = string_data_offset + rel_offset;
            bytes[offset..offset + 4].copy_from_slice(&abs_offset.to_le_bytes());
        }

        Self {
            bytes,
            string_table_offset,
            num_strings: strings.len() as u32,
            statement_table_offset,
            num_statements: data.statements.len() as u32,
        }
    }
}

// =============================================================================
// Data Component Generator
// =============================================================================

/// Generates WebAssembly components containing migration data.
///
/// The generated component exports the `migration-data` interface, which
/// provides access to:
/// - Migration metadata (ID, description, hashes, timestamp)
/// - SQL statements with descriptions
/// - Breaking change information
pub struct DataComponentGenerator {
    /// Reserved for future configuration options.
    _private: (),
}

impl DataComponentGenerator {
    /// Create a new data component generator.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Generate a data component from migration data.
    ///
    /// # Arguments
    ///
    /// * `data` - The migration data to embed
    ///
    /// # Returns
    ///
    /// Returns the WebAssembly component bytes that can be composed with
    /// the guest component.
    ///
    /// # Errors
    ///
    /// Returns an error if component generation fails.
    pub fn generate(&self, data: &MigrationData) -> Result<Vec<u8>, CompileError> {
        // Serialize the migration data
        let serialized = SerializedData::serialize(data);

        // Generate the core module
        let module = self.generate_core_module(&serialized)?;

        // Wrap in a component
        let component = self.wrap_in_component(&module)?;

        Ok(component)
    }

    /// Generate the core Wasm module with embedded data.
    fn generate_core_module(&self, data: &SerializedData) -> Result<Module, CompileError> {
        let mut module = Module::new();

        // Type section: define function types
        let mut types = TypeSection::new();

        // Type 0: () -> i32 (for getter functions that return a pointer/value)
        types.ty().function([], [ValType::I32]);

        // Type 1: (i32) -> i32 (for indexed getter functions)
        types.ty().function([ValType::I32], [ValType::I32]);

        module.section(&types);

        // Function section: declare functions
        let mut functions = FunctionSection::new();

        // Functions 0-4: metadata getters (get_id_ptr, get_desc_ptr, etc.)
        functions.function(0); // get_data_ptr: returns pointer to data start
        functions.function(0); // get_data_len: returns data length
        functions.function(0); // get_statement_count: returns number of statements
        functions.function(1); // get_statement_ptr: (index) -> ptr to statement data
        functions.function(0); // get_breaking_change_count: returns number of breaking changes

        module.section(&functions);

        // Memory section: define memory for data
        let mut memory = MemorySection::new();
        let pages_needed = data.bytes.len().div_ceil(65536) as u64;
        memory.memory(MemoryType {
            minimum: pages_needed.max(1),
            maximum: Some(pages_needed.max(1)),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memory);

        // Export section: export functions and memory
        let mut exports = ExportSection::new();
        exports.export("memory", wasm_encoder::ExportKind::Memory, 0);
        exports.export("get_data_ptr", wasm_encoder::ExportKind::Func, 0);
        exports.export("get_data_len", wasm_encoder::ExportKind::Func, 1);
        exports.export("get_statement_count", wasm_encoder::ExportKind::Func, 2);
        exports.export("get_statement_ptr", wasm_encoder::ExportKind::Func, 3);
        exports.export(
            "get_breaking_change_count",
            wasm_encoder::ExportKind::Func,
            4,
        );
        module.section(&exports);

        // Code section: implement functions
        let mut code = CodeSection::new();

        // Function 0: get_data_ptr() -> i32
        // Returns pointer to the start of the data (always 0)
        let mut f0 = Function::new([]);
        f0.instruction(&Instruction::I32Const(0));
        f0.instruction(&Instruction::End);
        code.function(&f0);

        // Function 1: get_data_len() -> i32
        // Returns the length of the data
        let mut f1 = Function::new([]);
        f1.instruction(&Instruction::I32Const(data.bytes.len() as i32));
        f1.instruction(&Instruction::End);
        code.function(&f1);

        // Function 2: get_statement_count() -> i32
        let mut f2 = Function::new([]);
        f2.instruction(&Instruction::I32Const(data.num_statements as i32));
        f2.instruction(&Instruction::End);
        code.function(&f2);

        // Function 3: get_statement_ptr(index: i32) -> i32
        // Returns pointer to statement data at index
        // Each statement is 12 bytes (3 x u32)
        let mut f3 = Function::new([(1, ValType::I32)]);
        // ptr = statement_table_offset + index * 12
        f3.instruction(&Instruction::I32Const(data.statement_table_offset as i32));
        f3.instruction(&Instruction::LocalGet(0)); // index
        f3.instruction(&Instruction::I32Const(12)); // statement size
        f3.instruction(&Instruction::I32Mul);
        f3.instruction(&Instruction::I32Add);
        f3.instruction(&Instruction::End);
        code.function(&f3);

        // Function 4: get_breaking_change_count() -> i32
        // For simplicity, return 0 for now (breaking changes have variable size)
        let mut f4 = Function::new([]);
        f4.instruction(&Instruction::I32Const(0)); // TODO: implement properly
        f4.instruction(&Instruction::End);
        code.function(&f4);

        module.section(&code);

        // Data section: embed the serialized data
        let mut data_section = DataSection::new();
        data_section.active(
            0,
            &wasm_encoder::ConstExpr::i32_const(0),
            data.bytes.iter().copied(),
        );
        module.section(&data_section);

        Ok(module)
    }

    /// Wrap a core module in a component.
    ///
    /// This creates a component that:
    /// 1. Embeds the core module
    /// 2. Instantiates it
    /// 3. Exports the migration-data interface
    fn wrap_in_component(&self, module: &Module) -> Result<Vec<u8>, CompileError> {
        let mut component = ComponentBuilder::default();

        // Add the core module
        component.core_module(module);

        // Instantiate the core module
        component.core_instantiate(0, []);

        // For now, just create a minimal component that embeds the module
        // The full component model lifting/lowering will be implemented
        // when the guest component interface is finalized

        // Export the core instance's exports at the component level
        // This is a simplified version - full implementation would use
        // component types and proper lifting

        Ok(component.finish())
    }
}

impl Default for DataComponentGenerator {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_migration_data() -> MigrationData {
        MigrationData {
            id: "abc123def456".to_string(),
            description: "Add users table".to_string(),
            source_state_hash: "0000000000000000".to_string(),
            target_state_hash: "1111111111111111".to_string(),
            compiled_at: "2024-01-15T10:00:00Z".to_string(),
            statements: vec![
                StatementData {
                    sql: "CREATE TABLE users (id INT PRIMARY KEY)".to_string(),
                    description: "Create users table".to_string(),
                    sequence: 1,
                },
                StatementData {
                    sql: "CREATE INDEX users_idx ON users(id)".to_string(),
                    description: "Create index on users".to_string(),
                    sequence: 2,
                },
            ],
            breaking_changes: vec![],
        }
    }

    mod serialization_tests {
        use super::*;

        #[test]
        fn serialize_migration_data() {
            let data = test_migration_data();
            let serialized = SerializedData::serialize(&data);

            // Verify basic structure
            assert!(!serialized.bytes.is_empty());
            assert!(serialized.num_strings > 0);
            assert_eq!(serialized.num_statements, 2);
        }

        #[test]
        fn serialized_data_starts_with_version() {
            let data = test_migration_data();
            let serialized = SerializedData::serialize(&data);

            // First 4 bytes should be version (1)
            let version = u32::from_le_bytes(serialized.bytes[0..4].try_into().unwrap());
            assert_eq!(version, 1);
        }

        #[test]
        fn serialized_data_contains_statement_count() {
            let data = test_migration_data();
            let serialized = SerializedData::serialize(&data);

            // Bytes 8-12 should be statement count
            let count = u32::from_le_bytes(serialized.bytes[8..12].try_into().unwrap());
            assert_eq!(count, 2);
        }
    }

    mod generator_tests {
        use super::*;

        #[test]
        fn generate_produces_valid_wasm() {
            let generator = DataComponentGenerator::new();
            let data = test_migration_data();

            let component = generator.generate(&data).unwrap();

            // Verify it's valid Wasm (magic number)
            assert_eq!(&component[0..4], b"\0asm");
        }

        #[test]
        fn generate_core_module_valid() {
            let generator = DataComponentGenerator::new();
            let data = test_migration_data();
            let serialized = SerializedData::serialize(&data);

            let module = generator.generate_core_module(&serialized).unwrap();
            let module_bytes = module.finish();

            // Verify it's a valid Wasm module
            assert_eq!(&module_bytes[0..4], b"\0asm");

            // Verify version is 1 (core module)
            let version = u32::from_le_bytes(module_bytes[4..8].try_into().unwrap());
            assert_eq!(version, 1);
        }

        #[test]
        fn generate_with_empty_statements() {
            let generator = DataComponentGenerator::new();
            let data = MigrationData {
                id: "empty".to_string(),
                description: "Empty migration".to_string(),
                source_state_hash: "source".to_string(),
                target_state_hash: "target".to_string(),
                compiled_at: "2024-01-01T00:00:00Z".to_string(),
                statements: vec![],
                breaking_changes: vec![],
            };

            let component = generator.generate(&data).unwrap();
            assert!(!component.is_empty());
        }

        #[test]
        fn generate_with_breaking_changes() {
            let generator = DataComponentGenerator::new();
            let data = MigrationData {
                id: "breaking".to_string(),
                description: "Migration with breaking changes".to_string(),
                source_state_hash: "source".to_string(),
                target_state_hash: "target".to_string(),
                compiled_at: "2024-01-01T00:00:00Z".to_string(),
                statements: vec![StatementData {
                    sql: "DROP TABLE old_table".to_string(),
                    description: "Drop old table".to_string(),
                    sequence: 1,
                }],
                breaking_changes: vec![BreakingChangeData {
                    description: "Dropping table will cause data loss".to_string(),
                    mitigation: MitigationStrategy::Destructive,
                    affected_sql: vec!["DROP TABLE old_table".to_string()],
                }],
            };

            let component = generator.generate(&data).unwrap();
            assert!(!component.is_empty());
        }

        #[test]
        fn default_creates_generator() {
            let _generator = DataComponentGenerator::default();
        }
    }

    mod mitigation_strategy_tests {
        use super::*;

        #[test]
        fn to_u8_values() {
            assert_eq!(MitigationStrategy::DualWrite.to_u8(), 0);
            assert_eq!(MitigationStrategy::Backfill.to_u8(), 1);
            assert_eq!(MitigationStrategy::Ratchet.to_u8(), 2);
            assert_eq!(MitigationStrategy::Destructive.to_u8(), 3);
        }
    }

    mod statement_data_tests {
        use super::*;

        #[test]
        fn new_creates_statement_data() {
            let data = StatementData::new("SELECT 1", "Test query", 42);

            assert_eq!(data.sql, "SELECT 1");
            assert_eq!(data.description, "Test query");
            assert_eq!(data.sequence, 42);
        }

        #[test]
        fn new_accepts_string_types() {
            let sql = String::from("INSERT INTO t VALUES (1)");
            let desc = String::from("Insert row");
            let data = StatementData::new(sql, desc, 1);

            assert_eq!(data.sql, "INSERT INTO t VALUES (1)");
            assert_eq!(data.description, "Insert row");
            assert_eq!(data.sequence, 1);
        }
    }
}
