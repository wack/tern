//! Tern Migration Guest - WASI Delegation Component
//!
//! This crate implements the migration guest component, which is a pure delegation
//! layer between the data component and the runner component.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Guest Component                          │
//! │                                                             │
//! │  Imports:                                                   │
//! │    - tern:migration-data/migration-data (from data comp)    │
//! │    - tern:migration/database (from runner)                  │
//! │    - tern:migration/log (from runner)                       │
//! │                                                             │
//! │  Exports:                                                   │
//! │    - tern:migration/migration                               │
//! │                                                             │
//! │  Implementation:                                            │
//! │    describe() -> builds metadata from migration-data        │
//! │    get_statements() -> returns all statements from data     │
//! │    run() -> iterates statements, calls database.execute()   │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! The guest component is pre-compiled to `wasm32-wasip2` and embedded in the
//! Tern binary. At migration compilation time, it is composed with a dynamically
//! generated data component to create a complete migration component.
//!
//! # Composition Flow
//!
//! 1. **Data Component** (generated at runtime): Provides SQL statements
//! 2. **Guest Component** (this crate): Delegates data to migration interface
//! 3. **Migration Component** = Guest + Data: Exports migration interface
//! 4. **Runner Component**: CLI that imports migration interface
//! 5. **Final Executable** = Runner + Migration: Standalone CLI application

// Re-export WIT path for build scripts
pub use tern_migration_wit::{
    GUEST_WIT_FILE, GUEST_WIT_PACKAGE, MAIN_WIT_FILE, MIGRATION_DATA_WIT_FILE,
    MIGRATION_DATA_WIT_PACKAGE, WIT_PACKAGE, WIT_PATH, WIT_VERSION,
};

// =============================================================================
// WASI Component Bindings
// =============================================================================

#[cfg(target_family = "wasm")]
mod wasm {
    // Generate bindings for the guest world
    wit_bindgen::generate!({
        path: "../tern-migration-wit/wit/tern-guest",
        world: "tern-guest",
        generate_all,
    });

    // Aliases for the exported types from the migration interface
    use exports::tern::migration::migration::{
        BreakingChange, Guest, Metadata, MitigationStrategy, Statement,
    };

    /// The guest implementation that delegates to imported interfaces.
    pub struct GuestImpl;

    impl Guest for GuestImpl {
        /// Build metadata from the imported migration-data interface.
        fn describe() -> Metadata {
            // Get breaking changes from data component
            let data_breaking_changes =
                tern::migration_data::migration_data::get_breaking_changes();
            let breaking_changes: Vec<BreakingChange> = data_breaking_changes
                .into_iter()
                .map(|bc| BreakingChange {
                    description: bc.description,
                    mitigation: convert_mitigation_strategy(bc.mitigation),
                    affected_sql: bc.affected_sql,
                })
                .collect();

            Metadata {
                id: tern::migration_data::migration_data::get_id(),
                description: tern::migration_data::migration_data::get_description(),
                breaking_changes,
                statement_count: tern::migration_data::migration_data::get_statement_count(),
                source_state_hash: tern::migration_data::migration_data::get_source_state_hash(),
                target_state_hash: tern::migration_data::migration_data::get_target_state_hash(),
                compiled_at: tern::migration_data::migration_data::get_compiled_at(),
            }
        }

        /// Get all statements from the imported migration-data interface.
        fn get_statements() -> Vec<Statement> {
            let count = tern::migration_data::migration_data::get_statement_count();
            (0..count)
                .map(|i| {
                    let stmt = tern::migration_data::migration_data::get_statement(i);
                    Statement {
                        sql: stmt.sql,
                        description: stmt.description,
                        sequence: stmt.sequence,
                    }
                })
                .collect()
        }

        /// Execute the migration by iterating statements and calling database.execute().
        fn run() -> Result<(), String> {
            use tern::migration::database;
            use tern::migration::log;

            let count = tern::migration_data::migration_data::get_statement_count();

            for i in 0..count {
                let stmt = tern::migration_data::migration_data::get_statement(i);

                // Log progress
                log::log(
                    log::Level::Info,
                    &format!("[{}/{}] {}", stmt.sequence, count, stmt.description),
                );

                // Execute the statement
                match database::execute(&stmt.sql) {
                    Ok(_rows) => {}
                    Err(e) => {
                        let error_msg = format!(
                            "Statement {} failed: {}{}",
                            stmt.sequence,
                            e.message,
                            e.code
                                .map(|c| format!(" (code: {})", c))
                                .unwrap_or_default()
                        );
                        log::log(log::Level::Error, &error_msg);
                        return Err(error_msg);
                    }
                }
            }

            log::log(log::Level::Info, "Migration completed successfully");
            Ok(())
        }
    }

    /// Convert mitigation strategy from data format to migration format.
    fn convert_mitigation_strategy(
        strategy: tern::migration_data::migration_data::MitigationStrategy,
    ) -> MitigationStrategy {
        use tern::migration_data::migration_data::MitigationStrategy as ImportMs;

        match strategy {
            ImportMs::DualWrite => MitigationStrategy::DualWrite,
            ImportMs::Backfill => MitigationStrategy::Backfill,
            ImportMs::Ratchet => MitigationStrategy::Ratchet,
            ImportMs::Destructive => MitigationStrategy::Destructive,
        }
    }

    // Export the component
    export!(GuestImpl);
}

// =============================================================================
// Native Types (for testing without WASM target)
// =============================================================================

/// Mitigation strategy for breaking changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MitigationStrategy {
    /// Requires parallel structures with synchronized writes.
    DualWrite = 0,
    /// Requires populating data before completion.
    Backfill = 1,
    /// Requires the NOT VALID + backfill + VALIDATE pattern.
    Ratchet = 2,
    /// Intentionally removes data or structure. Irreversible.
    Destructive = 3,
}

impl MitigationStrategy {
    /// Convert from a u8 value.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::DualWrite),
            1 => Some(Self::Backfill),
            2 => Some(Self::Ratchet),
            3 => Some(Self::Destructive),
            _ => None,
        }
    }

    /// Convert to a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DualWrite => "dual-write",
            Self::Backfill => "backfill",
            Self::Ratchet => "ratchet",
            Self::Destructive => "destructive",
        }
    }
}

/// A breaking change in the migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakingChange {
    /// Human-readable description of the breaking change.
    pub description: String,
    /// Strategy for safely executing this change.
    pub mitigation: MitigationStrategy,
    /// The SQL statement(s) that cause this breaking change.
    pub affected_sql: Vec<String>,
}

impl BreakingChange {
    /// Create a new breaking change.
    pub fn new(
        description: impl Into<String>,
        mitigation: MitigationStrategy,
        affected_sql: Vec<String>,
    ) -> Self {
        Self {
            description: description.into(),
            mitigation,
            affected_sql,
        }
    }
}

/// Migration metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    /// Unique identifier for this migration (content-addressable hash).
    pub id: String,
    /// Human-readable description of what this migration does.
    pub description: String,
    /// List of breaking changes that require mitigation.
    pub breaking_changes: Vec<BreakingChange>,
    /// Total number of SQL statements in this migration.
    pub statement_count: u32,
    /// Hash of the source schema state (before migration).
    pub source_state_hash: String,
    /// Hash of the target schema state (after migration).
    pub target_state_hash: String,
    /// Timestamp when this migration was compiled (RFC 3339 format).
    pub compiled_at: String,
}

impl Metadata {
    /// Check if this migration has any breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        !self.breaking_changes.is_empty()
    }

    /// Check if this migration has any destructive changes.
    pub fn has_destructive_changes(&self) -> bool {
        self.breaking_changes
            .iter()
            .any(|bc| bc.mitigation == MitigationStrategy::Destructive)
    }
}

/// A SQL statement with metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    /// The SQL statement text.
    pub sql: String,
    /// Human-readable description of what this statement does.
    pub description: String,
    /// Sequence number (1-indexed) within the migration.
    pub sequence: u32,
}

impl Statement {
    /// Create a new statement.
    pub fn new(sql: impl Into<String>, description: impl Into<String>, sequence: u32) -> Self {
        Self {
            sql: sql.into(),
            description: description.into(),
            sequence,
        }
    }
}

/// Error returned when a database operation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError {
    /// Human-readable error message.
    pub message: String,
    /// Optional PostgreSQL error code.
    pub code: Option<String>,
    /// Optional constraint name.
    pub constraint_name: Option<String>,
    /// Optional table name.
    pub table_name: Option<String>,
}

impl DbError {
    /// Create a new database error with just a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            constraint_name: None,
            table_name: None,
        }
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(code) = &self.code {
            write!(f, " (code: {})", code)?;
        }
        Ok(())
    }
}

impl std::error::Error for DbError {}

/// Log severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

impl LogLevel {
    /// Convert from a u8 value.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Debug),
            1 => Some(Self::Info),
            2 => Some(Self::Warn),
            3 => Some(Self::Error),
            _ => None,
        }
    }

    /// Convert to a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

// =============================================================================
// Native Mock Implementations (for testing)
// =============================================================================

#[cfg(not(target_family = "wasm"))]
pub mod database {
    use super::DbError;

    std::thread_local! {
        static MOCK_EXECUTE: std::cell::RefCell<Option<Box<dyn Fn(&str) -> Result<u64, DbError>>>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Execute a SQL statement and return the number of rows affected.
    pub fn execute(sql: &str) -> Result<u64, DbError> {
        MOCK_EXECUTE.with(|mock| {
            if let Some(f) = mock.borrow().as_ref() {
                f(sql)
            } else {
                Ok(0)
            }
        })
    }

    /// Set a mock implementation for `execute` (testing only).
    pub fn set_mock_execute<F>(f: F)
    where
        F: Fn(&str) -> Result<u64, DbError> + 'static,
    {
        MOCK_EXECUTE.with(|mock| {
            *mock.borrow_mut() = Some(Box::new(f));
        });
    }

    /// Clear all mocks (testing only).
    pub fn clear_mocks() {
        MOCK_EXECUTE.with(|mock| {
            *mock.borrow_mut() = None;
        });
    }
}

#[cfg(not(target_family = "wasm"))]
pub mod log {
    use super::LogLevel;

    std::thread_local! {
        static LOG_MESSAGES: std::cell::RefCell<Vec<(LogLevel, String)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Emit a log message at the specified level.
    pub fn log(level: LogLevel, message: &str) {
        LOG_MESSAGES.with(|messages| {
            messages.borrow_mut().push((level, message.to_string()));
        });
    }

    /// Log an info message.
    pub fn info(message: &str) {
        log(LogLevel::Info, message);
    }

    /// Log a warning message.
    pub fn warn(message: &str) {
        log(LogLevel::Warn, message);
    }

    /// Log an error message.
    pub fn error(message: &str) {
        log(LogLevel::Error, message);
    }

    /// Get all captured log messages (testing only).
    pub fn get_messages() -> Vec<(LogLevel, String)> {
        LOG_MESSAGES.with(|messages| messages.borrow().clone())
    }

    /// Clear captured log messages (testing only).
    pub fn clear_messages() {
        LOG_MESSAGES.with(|messages| {
            messages.borrow_mut().clear();
        });
    }
}

#[cfg(not(target_family = "wasm"))]
pub mod migration_data {
    use super::{BreakingChange, Statement};

    std::thread_local! {
        static MOCK_ID: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static MOCK_DESCRIPTION: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static MOCK_SOURCE_HASH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static MOCK_TARGET_HASH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static MOCK_COMPILED_AT: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static MOCK_STATEMENTS: std::cell::RefCell<Vec<Statement>> = const { std::cell::RefCell::new(Vec::new()) };
        static MOCK_BREAKING_CHANGES: std::cell::RefCell<Vec<BreakingChange>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    /// Get the migration ID.
    pub fn get_id() -> String {
        MOCK_ID.with(|id| id.borrow().clone())
    }

    /// Get the migration description.
    pub fn get_description() -> String {
        MOCK_DESCRIPTION.with(|desc| desc.borrow().clone())
    }

    /// Get the source schema state hash.
    pub fn get_source_state_hash() -> String {
        MOCK_SOURCE_HASH.with(|hash| hash.borrow().clone())
    }

    /// Get the target schema state hash.
    pub fn get_target_state_hash() -> String {
        MOCK_TARGET_HASH.with(|hash| hash.borrow().clone())
    }

    /// Get the compilation timestamp.
    pub fn get_compiled_at() -> String {
        MOCK_COMPILED_AT.with(|ts| ts.borrow().clone())
    }

    /// Get the number of statements.
    pub fn get_statement_count() -> u32 {
        MOCK_STATEMENTS.with(|stmts| stmts.borrow().len() as u32)
    }

    /// Get a statement by index.
    pub fn get_statement(index: u32) -> Statement {
        MOCK_STATEMENTS.with(|stmts| {
            stmts
                .borrow()
                .get(index as usize)
                .cloned()
                .unwrap_or_else(|| Statement::new("", "", 0))
        })
    }

    /// Get all breaking changes.
    pub fn get_breaking_changes() -> Vec<BreakingChange> {
        MOCK_BREAKING_CHANGES.with(|bcs| bcs.borrow().clone())
    }

    /// Set mock migration data (testing only).
    pub fn set_mock_data(
        id: impl Into<String>,
        description: impl Into<String>,
        source_hash: impl Into<String>,
        target_hash: impl Into<String>,
        compiled_at: impl Into<String>,
        statements: Vec<Statement>,
        breaking_changes: Vec<BreakingChange>,
    ) {
        MOCK_ID.with(|m| *m.borrow_mut() = id.into());
        MOCK_DESCRIPTION.with(|m| *m.borrow_mut() = description.into());
        MOCK_SOURCE_HASH.with(|m| *m.borrow_mut() = source_hash.into());
        MOCK_TARGET_HASH.with(|m| *m.borrow_mut() = target_hash.into());
        MOCK_COMPILED_AT.with(|m| *m.borrow_mut() = compiled_at.into());
        MOCK_STATEMENTS.with(|m| *m.borrow_mut() = statements);
        MOCK_BREAKING_CHANGES.with(|m| *m.borrow_mut() = breaking_changes);
    }

    /// Clear mock data (testing only).
    pub fn clear_mock_data() {
        MOCK_ID.with(|m| *m.borrow_mut() = String::new());
        MOCK_DESCRIPTION.with(|m| *m.borrow_mut() = String::new());
        MOCK_SOURCE_HASH.with(|m| *m.borrow_mut() = String::new());
        MOCK_TARGET_HASH.with(|m| *m.borrow_mut() = String::new());
        MOCK_COMPILED_AT.with(|m| *m.borrow_mut() = String::new());
        MOCK_STATEMENTS.with(|m| m.borrow_mut().clear());
        MOCK_BREAKING_CHANGES.with(|m| m.borrow_mut().clear());
    }
}

// =============================================================================
// Native Implementation (for testing)
// =============================================================================

/// Describe the migration by building metadata from imported data.
#[cfg(not(target_family = "wasm"))]
pub fn describe() -> Metadata {
    Metadata {
        id: migration_data::get_id(),
        description: migration_data::get_description(),
        breaking_changes: migration_data::get_breaking_changes(),
        statement_count: migration_data::get_statement_count(),
        source_state_hash: migration_data::get_source_state_hash(),
        target_state_hash: migration_data::get_target_state_hash(),
        compiled_at: migration_data::get_compiled_at(),
    }
}

/// Get all SQL statements from imported data.
#[cfg(not(target_family = "wasm"))]
pub fn get_statements() -> Vec<Statement> {
    let count = migration_data::get_statement_count();
    (0..count).map(migration_data::get_statement).collect()
}

/// Execute the migration by iterating statements and calling database.execute().
#[cfg(not(target_family = "wasm"))]
pub fn run() -> Result<(), String> {
    let statements = get_statements();
    let total = statements.len();

    for stmt in &statements {
        log::info(&format!(
            "[{}/{}] {}",
            stmt.sequence, total, stmt.description
        ));
        database::execute(&stmt.sql)
            .map_err(|e| format!("Statement {} failed: {}", stmt.sequence, e.message))?;
    }

    log::info("Migration completed successfully");
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_data() {
        migration_data::set_mock_data(
            "test-migration-id-123",
            "Add users table",
            "source-hash-abc",
            "target-hash-def",
            "2024-01-15T10:00:00Z",
            vec![
                Statement::new("CREATE TABLE users (id INT)", "Create users table", 1),
                Statement::new(
                    "CREATE INDEX idx_users ON users(id)",
                    "Create users index",
                    2,
                ),
            ],
            vec![],
        );
    }

    fn setup_test_data_with_breaking_changes() {
        migration_data::set_mock_data(
            "test-migration-id-456",
            "Drop column",
            "source-hash",
            "target-hash",
            "2024-01-15T10:00:00Z",
            vec![Statement::new(
                "ALTER TABLE t DROP COLUMN c",
                "Drop column c",
                1,
            )],
            vec![BreakingChange::new(
                "Dropping column causes data loss",
                MitigationStrategy::Destructive,
                vec!["ALTER TABLE t DROP COLUMN c".to_string()],
            )],
        );
    }

    fn cleanup() {
        migration_data::clear_mock_data();
        database::clear_mocks();
        log::clear_messages();
    }

    mod describe_tests {
        use super::*;

        #[test]
        fn describe_returns_correct_metadata() {
            setup_test_data();
            let metadata = describe();

            assert_eq!(metadata.id, "test-migration-id-123");
            assert_eq!(metadata.description, "Add users table");
            assert_eq!(metadata.statement_count, 2);
            assert_eq!(metadata.source_state_hash, "source-hash-abc");
            assert_eq!(metadata.target_state_hash, "target-hash-def");
            assert!(!metadata.has_breaking_changes());

            cleanup();
        }

        #[test]
        fn describe_includes_breaking_changes() {
            setup_test_data_with_breaking_changes();
            let metadata = describe();

            assert!(metadata.has_breaking_changes());
            assert!(metadata.has_destructive_changes());
            assert_eq!(metadata.breaking_changes.len(), 1);
            assert_eq!(
                metadata.breaking_changes[0].description,
                "Dropping column causes data loss"
            );

            cleanup();
        }
    }

    mod get_statements_tests {
        use super::*;

        #[test]
        fn get_statements_returns_all_statements() {
            setup_test_data();
            let statements = get_statements();

            assert_eq!(statements.len(), 2);
            assert_eq!(statements[0].sequence, 1);
            assert_eq!(statements[0].description, "Create users table");
            assert!(statements[0].sql.contains("CREATE TABLE"));
            assert_eq!(statements[1].sequence, 2);

            cleanup();
        }

        #[test]
        fn get_statements_empty_when_no_data() {
            cleanup();
            let statements = get_statements();
            assert!(statements.is_empty());
        }
    }

    mod run_tests {
        use super::*;
        use std::cell::RefCell;
        use std::rc::Rc;

        #[test]
        fn run_executes_all_statements() {
            setup_test_data();

            let executed = Rc::new(RefCell::new(Vec::new()));
            let executed_clone = executed.clone();

            database::set_mock_execute(move |sql| {
                executed_clone.borrow_mut().push(sql.to_string());
                Ok(1)
            });

            let result = run();
            assert!(result.is_ok());

            let executed_stmts = executed.borrow();
            assert_eq!(executed_stmts.len(), 2);
            assert!(executed_stmts[0].contains("CREATE TABLE"));
            assert!(executed_stmts[1].contains("CREATE INDEX"));

            cleanup();
        }

        #[test]
        fn run_logs_progress() {
            setup_test_data();

            let result = run();
            assert!(result.is_ok());

            let messages = log::get_messages();
            assert!(messages.iter().any(|(_, msg)| msg.contains("[1/2]")));
            assert!(messages.iter().any(|(_, msg)| msg.contains("[2/2]")));
            assert!(
                messages
                    .iter()
                    .any(|(_, msg)| msg.contains("completed successfully"))
            );

            cleanup();
        }

        #[test]
        fn run_returns_error_on_database_failure() {
            setup_test_data();

            database::set_mock_execute(|sql| {
                if sql.contains("CREATE INDEX") {
                    Err(DbError::new("Index creation failed"))
                } else {
                    Ok(1)
                }
            });

            let result = run();
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("Statement 2 failed"));

            cleanup();
        }
    }

    mod types_tests {
        use super::*;

        #[test]
        fn mitigation_strategy_from_u8() {
            assert_eq!(
                MitigationStrategy::from_u8(0),
                Some(MitigationStrategy::DualWrite)
            );
            assert_eq!(
                MitigationStrategy::from_u8(1),
                Some(MitigationStrategy::Backfill)
            );
            assert_eq!(
                MitigationStrategy::from_u8(2),
                Some(MitigationStrategy::Ratchet)
            );
            assert_eq!(
                MitigationStrategy::from_u8(3),
                Some(MitigationStrategy::Destructive)
            );
            assert_eq!(MitigationStrategy::from_u8(4), None);
        }

        #[test]
        fn mitigation_strategy_as_str() {
            assert_eq!(MitigationStrategy::DualWrite.as_str(), "dual-write");
            assert_eq!(MitigationStrategy::Backfill.as_str(), "backfill");
            assert_eq!(MitigationStrategy::Ratchet.as_str(), "ratchet");
            assert_eq!(MitigationStrategy::Destructive.as_str(), "destructive");
        }

        #[test]
        fn log_level_from_u8() {
            assert_eq!(LogLevel::from_u8(0), Some(LogLevel::Debug));
            assert_eq!(LogLevel::from_u8(1), Some(LogLevel::Info));
            assert_eq!(LogLevel::from_u8(2), Some(LogLevel::Warn));
            assert_eq!(LogLevel::from_u8(3), Some(LogLevel::Error));
            assert_eq!(LogLevel::from_u8(4), None);
        }

        #[test]
        fn log_level_as_str() {
            assert_eq!(LogLevel::Debug.as_str(), "debug");
            assert_eq!(LogLevel::Info.as_str(), "info");
            assert_eq!(LogLevel::Warn.as_str(), "warn");
            assert_eq!(LogLevel::Error.as_str(), "error");
        }

        #[test]
        fn db_error_display() {
            let error = DbError::new("Connection failed");
            assert_eq!(format!("{}", error), "Connection failed");

            let error_with_code = DbError {
                message: "Constraint violation".to_string(),
                code: Some("23505".to_string()),
                constraint_name: None,
                table_name: None,
            };
            assert!(format!("{}", error_with_code).contains("23505"));
        }

        #[test]
        fn statement_construction() {
            let stmt = Statement::new("SELECT 1", "Test query", 1);
            assert_eq!(stmt.sql, "SELECT 1");
            assert_eq!(stmt.description, "Test query");
            assert_eq!(stmt.sequence, 1);
        }

        #[test]
        fn breaking_change_construction() {
            let bc = BreakingChange::new(
                "Test change",
                MitigationStrategy::Destructive,
                vec!["SQL1".to_string(), "SQL2".to_string()],
            );
            assert_eq!(bc.description, "Test change");
            assert_eq!(bc.mitigation, MitigationStrategy::Destructive);
            assert_eq!(bc.affected_sql.len(), 2);
        }

        #[test]
        fn metadata_has_breaking_changes() {
            let metadata = Metadata {
                id: "test".to_string(),
                description: "test".to_string(),
                breaking_changes: vec![],
                statement_count: 0,
                source_state_hash: "".to_string(),
                target_state_hash: "".to_string(),
                compiled_at: "".to_string(),
            };
            assert!(!metadata.has_breaking_changes());
            assert!(!metadata.has_destructive_changes());

            let metadata_with_bc = Metadata {
                id: "test".to_string(),
                description: "test".to_string(),
                breaking_changes: vec![BreakingChange::new(
                    "test",
                    MitigationStrategy::Destructive,
                    vec![],
                )],
                statement_count: 0,
                source_state_hash: "".to_string(),
                target_state_hash: "".to_string(),
                compiled_at: "".to_string(),
            };
            assert!(metadata_with_bc.has_breaking_changes());
            assert!(metadata_with_bc.has_destructive_changes());
        }
    }
}
