//! WIT Interface Definitions for Tern Migration Components
//!
//! This crate contains the WebAssembly Interface Types (WIT) definitions
//! that specify the contract between compiled migration components and
//! the Tern runtime.
//!
//! # Overview
//!
//! Tern compiles database migrations into WebAssembly components. These
//! components implement the `tern-migration` world, which specifies:
//!
//! - **Imports**: Capabilities the runtime provides to migrations
//!   - `database`: Execute SQL statements against the target database
//!   - `log`: Emit log messages for progress reporting
//!
//! - **Exports**: Interface migrations must implement
//!   - `migration`: Describe and execute the migration
//!
//! # WIT Location
//!
//! The WIT definitions are in the `wit/` directory of this crate:
//!
//! ```text
//! crates/tern-migration-wit/
//! └── wit/
//!     └── migration.wit    # Main interface definitions
//! ```
//!
//! # Usage
//!
//! This crate is used by:
//!
//! 1. **`tern-migration-guest`**: Generates Rust bindings for migration
//!    component authors (or code generators) to implement.
//!
//! 2. **`tern-migration-runner`**: Generates Rust bindings for the runtime
//!    to instantiate and execute migration components.
//!
//! The WIT files are consumed at build time by `wit-bindgen` (for guests)
//! and `wasmtime` (for hosts).
//!
//! # Interface Details
//!
//! ## Database Interface
//!
//! ```wit
//! interface database {
//!     record db-error { message: string, code: option<string>, ... }
//!     execute: func(sql: string) -> result<u64, db-error>;
//!     query: func(sql: string) -> result<string, db-error>;
//! }
//! ```
//!
//! ## Log Interface
//!
//! ```wit
//! interface log {
//!     enum level { debug, info, warn, error }
//!     log: func(level: level, message: string);
//! }
//! ```
//!
//! ## Migration Interface
//!
//! ```wit
//! interface migration {
//!     record metadata { id: string, description: string, ... }
//!     record statement { sql: string, description: string, sequence: u32 }
//!     describe: func() -> metadata;
//!     get-statements: func() -> list<statement>;
//!     run: func() -> result<_, string>;
//! }
//! ```
//!
//! # Version Compatibility
//!
//! The interface is versioned (currently `@0.1.0`). Components compiled
//! against a specific version should be compatible with any runtime that
//! supports that version.

/// Path to the WIT directory relative to this crate's root.
///
/// This constant is useful for build scripts that need to locate
/// the WIT files.
pub const WIT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/wit");

/// The main WIT file name.
pub const MAIN_WIT_FILE: &str = "migration.wit";

/// Current version of the WIT interface.
pub const WIT_VERSION: &str = "0.1.0";

/// Package identifier for the WIT interface.
pub const WIT_PACKAGE: &str = "tern:migration@0.1.0";

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn wit_path_exists() {
        let wit_dir = Path::new(WIT_PATH);
        assert!(
            wit_dir.exists(),
            "WIT directory should exist at: {}",
            WIT_PATH
        );
        assert!(
            wit_dir.is_dir(),
            "WIT_PATH should point to a directory: {}",
            WIT_PATH
        );
    }

    #[test]
    fn main_wit_file_exists() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        assert!(
            wit_file.exists(),
            "Main WIT file should exist at: {}",
            wit_file.display()
        );
    }

    #[test]
    fn wit_file_contains_package_declaration() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("package tern:migration@0.1.0"),
            "WIT file should contain package declaration"
        );
    }

    #[test]
    fn wit_file_contains_required_interfaces() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        // Check for database interface
        assert!(
            content.contains("interface database"),
            "WIT file should define database interface"
        );
        assert!(
            content.contains("execute: func(sql: string)"),
            "database interface should have execute function"
        );
        assert!(
            content.contains("query: func(sql: string)"),
            "database interface should have query function"
        );

        // Check for log interface
        assert!(
            content.contains("interface log"),
            "WIT file should define log interface"
        );
        assert!(
            content.contains("log: func(level: level, message: string)"),
            "log interface should have log function"
        );

        // Check for migration interface
        assert!(
            content.contains("interface migration"),
            "WIT file should define migration interface"
        );
        assert!(
            content.contains("describe: func() -> metadata"),
            "migration interface should have describe function"
        );
        assert!(
            content.contains("get-statements: func() -> list<statement>"),
            "migration interface should have get-statements function"
        );
        assert!(
            content.contains("run: func() -> result<_, string>"),
            "migration interface should have run function"
        );
    }

    #[test]
    fn wit_file_contains_world_definition() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("world tern-migration"),
            "WIT file should define tern-migration world"
        );
        assert!(
            content.contains("import database"),
            "World should import database"
        );
        assert!(content.contains("import log"), "World should import log");
        assert!(
            content.contains("export migration"),
            "World should export migration"
        );
    }

    #[test]
    fn wit_version_matches_package() {
        assert_eq!(WIT_PACKAGE, format!("tern:migration@{}", WIT_VERSION));
    }

    #[test]
    fn wit_file_contains_db_error_record() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("record db-error"),
            "WIT file should define db-error record"
        );
        assert!(
            content.contains("message: string"),
            "db-error should have message field"
        );
        assert!(
            content.contains("code: option<string>"),
            "db-error should have code field"
        );
    }

    #[test]
    fn wit_file_contains_metadata_record() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("record metadata"),
            "WIT file should define metadata record"
        );
        assert!(
            content.contains("id: string"),
            "metadata should have id field"
        );
        assert!(
            content.contains("description: string"),
            "metadata should have description field"
        );
        assert!(
            content.contains("breaking-changes: list<breaking-change>"),
            "metadata should have breaking-changes field"
        );
        assert!(
            content.contains("statement-count: u32"),
            "metadata should have statement-count field"
        );
        assert!(
            content.contains("source-state-hash: string"),
            "metadata should have source-state-hash field"
        );
        assert!(
            content.contains("target-state-hash: string"),
            "metadata should have target-state-hash field"
        );
    }

    #[test]
    fn wit_file_contains_breaking_change_types() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("enum mitigation-strategy"),
            "WIT file should define mitigation-strategy enum"
        );
        assert!(
            content.contains("dual-write,"),
            "mitigation-strategy should include dual-write"
        );
        assert!(
            content.contains("backfill,"),
            "mitigation-strategy should include backfill"
        );
        assert!(
            content.contains("ratchet,"),
            "mitigation-strategy should include ratchet"
        );
        assert!(
            content.contains("destructive,"),
            "mitigation-strategy should include destructive"
        );
        assert!(
            content.contains("record breaking-change"),
            "WIT file should define breaking-change record"
        );
        assert!(
            content.contains("mitigation: mitigation-strategy"),
            "breaking-change should have mitigation field"
        );
    }

    #[test]
    fn wit_file_contains_log_levels() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("enum level"),
            "WIT file should define log level enum"
        );
        assert!(content.contains("debug,"), "log level should include debug");
        assert!(content.contains("info,"), "log level should include info");
        assert!(content.contains("warn,"), "log level should include warn");
        assert!(content.contains("error,"), "log level should include error");
    }

    #[test]
    fn wit_file_contains_statement_record() {
        let wit_file = Path::new(WIT_PATH).join(MAIN_WIT_FILE);
        let content = std::fs::read_to_string(&wit_file).expect("Should be able to read WIT file");

        assert!(
            content.contains("record statement"),
            "WIT file should define statement record"
        );
        assert!(
            content.contains("sql: string"),
            "statement should have sql field"
        );
        assert!(
            content.contains("sequence: u32"),
            "statement should have sequence field"
        );
    }
}
