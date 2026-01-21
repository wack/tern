//! Host function implementations for migration components.
//!
//! This module provides the host-side implementations of the interfaces
//! that migration components import: database access and logging.
//!
//! # Overview
//!
//! The WebAssembly component imports two interfaces:
//! - `tern:migration/database@0.1.0` - For executing SQL statements
//! - `tern:migration/log@0.1.0` - For emitting log messages
//!
//! This module provides the implementations that are linked into the
//! component when it's instantiated.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio_postgres::Client;
use tracing::{debug, error, info, warn};

use crate::error::DatabaseError;

/// State maintained by the host during migration execution.
///
/// This is passed to the WebAssembly component's host functions and provides
/// access to the database connection and execution mode.
pub struct HostState {
    /// The PostgreSQL client, if connected.
    /// Wrapped in Arc<Mutex> for safe sharing across async boundaries.
    client: Option<Arc<Mutex<Client>>>,

    /// Whether we're in dry-run mode (no actual SQL execution).
    dry_run: bool,

    /// Collected SQL statements when in dry-run mode.
    dry_run_statements: Vec<String>,

    /// Log messages collected during execution.
    log_messages: Vec<LogMessage>,

    /// Whether to capture log messages (for testing).
    capture_logs: bool,
}

/// A captured log message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogMessage {
    /// The log level.
    pub level: LogLevel,
    /// The message text.
    pub message: String,
}

/// Log severity levels matching the WIT definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    /// Debug-level messages.
    Debug,
    /// Informational messages.
    Info,
    /// Warning messages.
    Warn,
    /// Error messages.
    Error,
}

impl LogLevel {
    /// Convert from the WIT level enum value.
    pub fn from_wit_value(value: u8) -> Self {
        match value {
            0 => Self::Debug,
            1 => Self::Info,
            2 => Self::Warn,
            _ => Self::Error,
        }
    }

    /// Get a string representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Result of executing a SQL statement.
#[derive(Debug, Clone)]
pub struct ExecuteResult {
    /// Number of rows affected.
    pub rows_affected: u64,
}

/// Error returned from database operations in the WIT interface.
#[derive(Debug, Clone)]
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

    /// Create from a tokio_postgres error.
    pub fn from_postgres_error(err: &tokio_postgres::Error) -> Self {
        let message = err.to_string();

        // Try to extract PostgreSQL-specific error details
        let (code, constraint_name, table_name) = if let Some(db_err) = err.as_db_error() {
            (
                Some(db_err.code().code().to_string()),
                db_err.constraint().map(|s| s.to_string()),
                db_err.table().map(|s| s.to_string()),
            )
        } else {
            (None, None, None)
        };

        Self {
            message,
            code,
            constraint_name,
            table_name,
        }
    }
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(code) = &self.code {
            write!(f, " (code: {})", code)?;
        }
        if let Some(constraint) = &self.constraint_name {
            write!(f, " (constraint: {})", constraint)?;
        }
        if let Some(table) = &self.table_name {
            write!(f, " (table: {})", table)?;
        }
        Ok(())
    }
}

impl HostState {
    /// Create a new host state with no database connection.
    ///
    /// This is useful for describe/dry-run operations.
    pub fn new() -> Self {
        Self {
            client: None,
            dry_run: true,
            dry_run_statements: Vec::new(),
            log_messages: Vec::new(),
            capture_logs: false,
        }
    }

    /// Create a host state for dry-run mode.
    pub fn dry_run() -> Self {
        Self {
            client: None,
            dry_run: true,
            dry_run_statements: Vec::new(),
            log_messages: Vec::new(),
            capture_logs: false,
        }
    }

    /// Create a host state with a database connection.
    pub fn with_client(client: Client) -> Self {
        Self {
            client: Some(Arc::new(Mutex::new(client))),
            dry_run: false,
            dry_run_statements: Vec::new(),
            log_messages: Vec::new(),
            capture_logs: false,
        }
    }

    /// Enable log capture mode (for testing).
    pub fn with_log_capture(mut self) -> Self {
        self.capture_logs = true;
        self
    }

    /// Enable dry-run mode.
    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    /// Set the database client.
    pub fn set_client(&mut self, client: Client) {
        self.client = Some(Arc::new(Mutex::new(client)));
    }

    /// Check if we're in dry-run mode.
    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Check if we have a database connection.
    pub fn has_client(&self) -> bool {
        self.client.is_some()
    }

    /// Get the collected SQL statements from dry-run mode.
    pub fn dry_run_statements(&self) -> &[String] {
        &self.dry_run_statements
    }

    /// Get the captured log messages.
    pub fn log_messages(&self) -> &[LogMessage] {
        &self.log_messages
    }

    /// Clear captured log messages.
    pub fn clear_log_messages(&mut self) {
        self.log_messages.clear();
    }

    /// Clear dry-run statements.
    pub fn clear_dry_run_statements(&mut self) {
        self.dry_run_statements.clear();
    }

    /// Record a SQL statement (for dry-run mode).
    pub fn record_statement(&mut self, sql: &str) {
        self.dry_run_statements.push(sql.to_string());
    }
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

/// Database host functions implementation.
///
/// These functions are called by the WebAssembly component to execute SQL.
pub mod database {
    use super::*;

    /// Execute a SQL statement and return the number of rows affected.
    ///
    /// In dry-run mode, this records the statement but doesn't execute it.
    pub async fn execute(state: &mut HostState, sql: &str) -> Result<u64, DbError> {
        debug!(sql = sql, "Executing SQL statement");

        if state.dry_run {
            // In dry-run mode, just record the statement
            state.dry_run_statements.push(sql.to_string());
            return Ok(0);
        }

        let client = state
            .client
            .as_ref()
            .ok_or_else(|| DbError::new("No database connection"))?;

        let client = client.lock().await;
        let rows_affected = client.execute(sql, &[]).await.map_err(|e| {
            error!(error = %e, sql = sql, "SQL execution failed");
            DbError::from_postgres_error(&e)
        })?;

        debug!(rows_affected = rows_affected, "Statement executed");
        Ok(rows_affected)
    }

    /// Execute a SQL query and return results as JSON.
    ///
    /// In dry-run mode, returns an empty array.
    pub async fn query(state: &mut HostState, sql: &str) -> Result<String, DbError> {
        debug!(sql = sql, "Executing SQL query");

        if state.dry_run {
            // In dry-run mode, return empty results
            state.dry_run_statements.push(sql.to_string());
            return Ok("[]".to_string());
        }

        let client = state
            .client
            .as_ref()
            .ok_or_else(|| DbError::new("No database connection"))?;

        let client = client.lock().await;
        let rows = client.query(sql, &[]).await.map_err(|e| {
            error!(error = %e, sql = sql, "SQL query failed");
            DbError::from_postgres_error(&e)
        })?;

        // Convert rows to JSON
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| row_to_json(row))
            .collect::<Result<Vec<_>, _>>()?;

        let json = serde_json::to_string(&json_rows)
            .map_err(|e| DbError::new(format!("Failed to serialize query results: {}", e)))?;

        debug!(row_count = json_rows.len(), "Query executed");
        Ok(json)
    }

    /// Convert a PostgreSQL row to a JSON value.
    fn row_to_json(row: &tokio_postgres::Row) -> Result<serde_json::Value, DbError> {
        use serde_json::{Map, Value};

        let mut obj = Map::new();
        for (i, column) in row.columns().iter().enumerate() {
            let name = column.name().to_string();
            let value = column_value_to_json(row, i, column)?;
            obj.insert(name, value);
        }
        Ok(Value::Object(obj))
    }

    /// Convert a column value to JSON.
    fn column_value_to_json(
        row: &tokio_postgres::Row,
        idx: usize,
        column: &tokio_postgres::Column,
    ) -> Result<serde_json::Value, DbError> {
        use serde_json::Value;
        use tokio_postgres::types::Type;

        // Handle NULL values
        let is_null: Option<Option<String>> = row.try_get(idx).ok();
        if matches!(is_null, Some(None)) {
            return Ok(Value::Null);
        }

        // Convert based on type
        let value = match *column.type_() {
            Type::BOOL => row
                .try_get::<_, bool>(idx)
                .map(Value::Bool)
                .unwrap_or(Value::Null),
            Type::INT2 => row
                .try_get::<_, i16>(idx)
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            Type::INT4 => row
                .try_get::<_, i32>(idx)
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            Type::INT8 => row
                .try_get::<_, i64>(idx)
                .map(|v| Value::Number(v.into()))
                .unwrap_or(Value::Null),
            Type::FLOAT4 => row
                .try_get::<_, f32>(idx)
                .ok()
                .and_then(|v| serde_json::Number::from_f64(v as f64))
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Type::FLOAT8 => row
                .try_get::<_, f64>(idx)
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            Type::TEXT | Type::VARCHAR | Type::CHAR | Type::NAME | Type::BPCHAR => row
                .try_get::<_, String>(idx)
                .map(Value::String)
                .unwrap_or(Value::Null),
            Type::JSON | Type::JSONB => {
                // Get JSON as string first, then parse
                row.try_get::<_, String>(idx)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null)
            }
            _ => {
                // Fallback: try to get as string
                row.try_get::<_, String>(idx)
                    .map(Value::String)
                    .unwrap_or(Value::Null)
            }
        };

        Ok(value)
    }
}

/// Logging host functions implementation.
///
/// These functions are called by the WebAssembly component to emit logs.
pub mod log {
    use super::*;

    /// Emit a log message at the specified level.
    pub fn log_message(state: &mut HostState, level: LogLevel, message: &str) {
        // Capture if enabled
        if state.capture_logs {
            state.log_messages.push(LogMessage {
                level,
                message: message.to_string(),
            });
        }

        // Also emit through tracing
        match level {
            LogLevel::Debug => debug!(target: "migration", "{}", message),
            LogLevel::Info => info!(target: "migration", "{}", message),
            LogLevel::Warn => warn!(target: "migration", "{}", message),
            LogLevel::Error => error!(target: "migration", "{}", message),
        }
    }

    /// Emit a debug message.
    pub fn debug(state: &mut HostState, message: &str) {
        log_message(state, LogLevel::Debug, message);
    }

    /// Emit an info message.
    pub fn info(state: &mut HostState, message: &str) {
        log_message(state, LogLevel::Info, message);
    }

    /// Emit a warning message.
    pub fn warn(state: &mut HostState, message: &str) {
        log_message(state, LogLevel::Warn, message);
    }

    /// Emit an error message.
    pub fn error(state: &mut HostState, message: &str) {
        log_message(state, LogLevel::Error, message);
    }
}

/// Connect to a PostgreSQL database.
///
/// Supports both regular TCP connections and TLS (via rustls).
pub async fn connect_database(url: &str) -> Result<Client, DatabaseError> {
    // Parse the connection string
    let config: tokio_postgres::Config = url
        .parse()
        .map_err(|e| DatabaseError::invalid_url(format!("Failed to parse URL: {}", e)))?;

    // Determine if we should use TLS
    let host = config
        .get_hosts()
        .first()
        .map(|h| match h {
            tokio_postgres::config::Host::Tcp(s) => s.as_str(),
            #[cfg(unix)]
            tokio_postgres::config::Host::Unix(_) => "localhost",
        })
        .unwrap_or("localhost");

    // Check if we should use TLS (for non-localhost connections)
    let use_tls = !matches!(host, "localhost" | "127.0.0.1" | "::1");

    if use_tls {
        // Create TLS connector
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(tls_config);

        let (client, connection) = config
            .connect(tls)
            .await
            .map_err(|e| DatabaseError::connection(e))?;

        // Spawn the connection handler
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("Database connection error: {}", e);
            }
        });

        Ok(client)
    } else {
        // For localhost, connect without TLS
        let (client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .map_err(|e| DatabaseError::connection(e))?;

        // Spawn the connection handler
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                error!("Database connection error: {}", e);
            }
        });

        Ok(client)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod host_state_tests {
        use super::*;

        #[test]
        fn new_creates_dry_run_state() {
            let state = HostState::new();
            assert!(state.is_dry_run());
            assert!(!state.has_client());
        }

        #[test]
        fn dry_run_creates_dry_run_state() {
            let state = HostState::dry_run();
            assert!(state.is_dry_run());
            assert!(!state.has_client());
        }

        #[test]
        fn with_log_capture_enables_capture() {
            let state = HostState::new().with_log_capture();
            assert!(state.capture_logs);
        }

        #[test]
        fn with_dry_run_sets_mode() {
            let state = HostState::new().with_dry_run(false);
            assert!(!state.is_dry_run());
        }

        #[test]
        fn dry_run_statements_collects_sql() {
            let state = HostState::dry_run();
            assert!(state.dry_run_statements().is_empty());
        }

        #[test]
        fn log_messages_empty_initially() {
            let state = HostState::new();
            assert!(state.log_messages().is_empty());
        }

        #[test]
        fn clear_log_messages_clears_messages() {
            let mut state = HostState::new().with_log_capture();
            log::info(&mut state, "test message");
            assert!(!state.log_messages().is_empty());
            state.clear_log_messages();
            assert!(state.log_messages().is_empty());
        }

        #[test]
        fn clear_dry_run_statements_clears_statements() {
            let mut state = HostState::dry_run();
            state.dry_run_statements.push("SELECT 1".to_string());
            assert!(!state.dry_run_statements().is_empty());
            state.clear_dry_run_statements();
            assert!(state.dry_run_statements().is_empty());
        }
    }

    mod log_level_tests {
        use super::*;

        #[test]
        fn from_wit_value_debug() {
            assert_eq!(LogLevel::from_wit_value(0), LogLevel::Debug);
        }

        #[test]
        fn from_wit_value_info() {
            assert_eq!(LogLevel::from_wit_value(1), LogLevel::Info);
        }

        #[test]
        fn from_wit_value_warn() {
            assert_eq!(LogLevel::from_wit_value(2), LogLevel::Warn);
        }

        #[test]
        fn from_wit_value_error() {
            assert_eq!(LogLevel::from_wit_value(3), LogLevel::Error);
        }

        #[test]
        fn from_wit_value_unknown_defaults_to_error() {
            assert_eq!(LogLevel::from_wit_value(255), LogLevel::Error);
        }

        #[test]
        fn as_str_returns_correct_strings() {
            assert_eq!(LogLevel::Debug.as_str(), "DEBUG");
            assert_eq!(LogLevel::Info.as_str(), "INFO");
            assert_eq!(LogLevel::Warn.as_str(), "WARN");
            assert_eq!(LogLevel::Error.as_str(), "ERROR");
        }

        #[test]
        fn display_uses_as_str() {
            assert_eq!(format!("{}", LogLevel::Info), "INFO");
        }
    }

    mod db_error_tests {
        use super::*;

        #[test]
        fn new_creates_error_with_message() {
            let err = DbError::new("test error");
            assert_eq!(err.message, "test error");
            assert!(err.code.is_none());
            assert!(err.constraint_name.is_none());
            assert!(err.table_name.is_none());
        }

        #[test]
        fn display_shows_message() {
            let err = DbError::new("test error");
            assert_eq!(format!("{}", err), "test error");
        }

        #[test]
        fn display_shows_code() {
            let err = DbError {
                message: "error".to_string(),
                code: Some("23505".to_string()),
                constraint_name: None,
                table_name: None,
            };
            let display = format!("{}", err);
            assert!(display.contains("23505"));
        }

        #[test]
        fn display_shows_constraint() {
            let err = DbError {
                message: "error".to_string(),
                code: None,
                constraint_name: Some("users_pk".to_string()),
                table_name: None,
            };
            let display = format!("{}", err);
            assert!(display.contains("users_pk"));
        }

        #[test]
        fn display_shows_table() {
            let err = DbError {
                message: "error".to_string(),
                code: None,
                constraint_name: None,
                table_name: Some("users".to_string()),
            };
            let display = format!("{}", err);
            assert!(display.contains("users"));
        }

        #[test]
        fn display_shows_all_details() {
            let err = DbError {
                message: "unique violation".to_string(),
                code: Some("23505".to_string()),
                constraint_name: Some("users_email_key".to_string()),
                table_name: Some("users".to_string()),
            };
            let display = format!("{}", err);
            assert!(display.contains("unique violation"));
            assert!(display.contains("23505"));
            assert!(display.contains("users_email_key"));
            assert!(display.contains("users"));
        }
    }

    mod log_tests {
        use super::*;

        #[test]
        fn log_message_captures_when_enabled() {
            let mut state = HostState::new().with_log_capture();
            log::log_message(&mut state, LogLevel::Info, "test message");

            let messages = state.log_messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].level, LogLevel::Info);
            assert_eq!(messages[0].message, "test message");
        }

        #[test]
        fn log_message_does_not_capture_when_disabled() {
            let mut state = HostState::new();
            log::log_message(&mut state, LogLevel::Info, "test message");

            assert!(state.log_messages().is_empty());
        }

        #[test]
        fn debug_logs_at_debug_level() {
            let mut state = HostState::new().with_log_capture();
            log::debug(&mut state, "debug message");

            assert_eq!(state.log_messages()[0].level, LogLevel::Debug);
        }

        #[test]
        fn info_logs_at_info_level() {
            let mut state = HostState::new().with_log_capture();
            log::info(&mut state, "info message");

            assert_eq!(state.log_messages()[0].level, LogLevel::Info);
        }

        #[test]
        fn warn_logs_at_warn_level() {
            let mut state = HostState::new().with_log_capture();
            log::warn(&mut state, "warn message");

            assert_eq!(state.log_messages()[0].level, LogLevel::Warn);
        }

        #[test]
        fn error_logs_at_error_level() {
            let mut state = HostState::new().with_log_capture();
            log::error(&mut state, "error message");

            assert_eq!(state.log_messages()[0].level, LogLevel::Error);
        }

        #[test]
        fn multiple_messages_captured_in_order() {
            let mut state = HostState::new().with_log_capture();
            log::info(&mut state, "first");
            log::warn(&mut state, "second");
            log::error(&mut state, "third");

            let messages = state.log_messages();
            assert_eq!(messages.len(), 3);
            assert_eq!(messages[0].message, "first");
            assert_eq!(messages[1].message, "second");
            assert_eq!(messages[2].message, "third");
        }
    }

    mod database_tests {
        use super::*;

        #[tokio::test]
        async fn execute_in_dry_run_collects_statements() {
            let mut state = HostState::dry_run();

            let result = database::execute(&mut state, "CREATE TABLE test (id INT)").await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 0);
            assert_eq!(state.dry_run_statements().len(), 1);
            assert_eq!(state.dry_run_statements()[0], "CREATE TABLE test (id INT)");
        }

        #[tokio::test]
        async fn execute_without_client_returns_error() {
            let mut state = HostState::new().with_dry_run(false);

            let result = database::execute(&mut state, "SELECT 1").await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("No database connection")
            );
        }

        #[tokio::test]
        async fn query_in_dry_run_returns_empty_array() {
            let mut state = HostState::dry_run();

            let result = database::query(&mut state, "SELECT * FROM test").await;

            assert!(result.is_ok());
            assert_eq!(result.unwrap(), "[]");
            assert_eq!(state.dry_run_statements().len(), 1);
        }

        #[tokio::test]
        async fn query_without_client_returns_error() {
            let mut state = HostState::new().with_dry_run(false);

            let result = database::query(&mut state, "SELECT 1").await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .message
                    .contains("No database connection")
            );
        }

        #[tokio::test]
        async fn multiple_execute_calls_collect_all_statements() {
            let mut state = HostState::dry_run();

            database::execute(&mut state, "CREATE TABLE t1 (id INT)")
                .await
                .unwrap();
            database::execute(&mut state, "CREATE TABLE t2 (id INT)")
                .await
                .unwrap();
            database::execute(&mut state, "CREATE INDEX idx ON t1(id)")
                .await
                .unwrap();

            assert_eq!(state.dry_run_statements().len(), 3);
        }
    }

    mod log_message_tests {
        use super::*;

        #[test]
        fn log_message_equality() {
            let msg1 = LogMessage {
                level: LogLevel::Info,
                message: "test".to_string(),
            };
            let msg2 = LogMessage {
                level: LogLevel::Info,
                message: "test".to_string(),
            };
            assert_eq!(msg1, msg2);
        }

        #[test]
        fn log_message_inequality_level() {
            let msg1 = LogMessage {
                level: LogLevel::Info,
                message: "test".to_string(),
            };
            let msg2 = LogMessage {
                level: LogLevel::Warn,
                message: "test".to_string(),
            };
            assert_ne!(msg1, msg2);
        }

        #[test]
        fn log_message_inequality_message() {
            let msg1 = LogMessage {
                level: LogLevel::Info,
                message: "test1".to_string(),
            };
            let msg2 = LogMessage {
                level: LogLevel::Info,
                message: "test2".to_string(),
            };
            assert_ne!(msg1, msg2);
        }

        #[test]
        fn log_message_clone() {
            let msg = LogMessage {
                level: LogLevel::Info,
                message: "test".to_string(),
            };
            let cloned = msg.clone();
            assert_eq!(msg, cloned);
        }

        #[test]
        fn log_message_debug() {
            let msg = LogMessage {
                level: LogLevel::Info,
                message: "test".to_string(),
            };
            let debug_str = format!("{:?}", msg);
            assert!(debug_str.contains("Info"));
            assert!(debug_str.contains("test"));
        }
    }
}
