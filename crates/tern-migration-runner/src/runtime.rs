//! WebAssembly runtime for executing migration components.
//!
//! This module provides the `MigrationRuntime` which wraps Wasmtime and
//! provides a high-level interface for loading and executing migration
//! WebAssembly components.
//!
//! # Overview
//!
//! The runtime handles:
//! - Loading and validating WebAssembly components
//! - Linking host functions (database, logging)
//! - Executing migration operations
//! - Extracting metadata from components
//!
//! # Usage
//!
//! ```ignore
//! use tern_migration_runner::runtime::MigrationRuntime;
//! use tern_migration_runner::host::HostState;
//!
//! // Load component from bytes
//! let runtime = MigrationRuntime::from_bytes(&component_bytes)?;
//!
//! // Get metadata (dry run)
//! let metadata = runtime.describe()?;
//! println!("Migration: {}", metadata.description);
//!
//! // Execute with database connection
//! let state = HostState::with_client(client);
//! runtime.run(state)?;
//! ```

use wasmtime::component::{Component, HasSelf, ResourceTable};
use wasmtime::{Config, Engine, Store};

use crate::error::RuntimeError;
use crate::host::{self, HostState, LogLevel};

// Use Wasmtime's bindgen macro to generate the host bindings
wasmtime::component::bindgen!({
    path: "../tern-migration-wit/wit",
    world: "tern-migration",
});

/// Metadata about a migration component.
///
/// This is extracted from the component's `describe` export.
#[derive(Debug, Clone)]
pub struct MigrationMetadata {
    /// Unique identifier (content-addressable hash).
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// List of breaking changes that require mitigation.
    pub breaking_changes: Vec<BreakingChangeInfo>,
    /// Total number of SQL statements.
    pub statement_count: u32,
    /// Hash of the source schema state.
    pub source_state_hash: String,
    /// Hash of the target schema state.
    pub target_state_hash: String,
    /// When the migration was compiled (RFC 3339).
    pub compiled_at: String,
}

impl MigrationMetadata {
    /// Returns true if this migration has any breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        !self.breaking_changes.is_empty()
    }

    /// Returns true if this migration has any destructive changes.
    pub fn has_destructive_changes(&self) -> bool {
        self.breaking_changes
            .iter()
            .any(|bc| bc.mitigation == MitigationStrategy::Destructive)
    }

    /// Returns true if this migration is safe (no breaking changes).
    pub fn is_safe(&self) -> bool {
        self.breaking_changes.is_empty()
    }
}

impl From<exports::tern::migration::migration::Metadata> for MigrationMetadata {
    fn from(m: exports::tern::migration::migration::Metadata) -> Self {
        Self {
            id: m.id,
            description: m.description,
            breaking_changes: m.breaking_changes.into_iter().map(Into::into).collect(),
            statement_count: m.statement_count,
            source_state_hash: m.source_state_hash,
            target_state_hash: m.target_state_hash,
            compiled_at: m.compiled_at,
        }
    }
}

/// A breaking change in a migration.
#[derive(Debug, Clone)]
pub struct BreakingChangeInfo {
    /// Human-readable description.
    pub description: String,
    /// Mitigation strategy.
    pub mitigation: MitigationStrategy,
    /// Affected SQL statements.
    pub affected_sql: Vec<String>,
}

impl From<exports::tern::migration::migration::BreakingChange> for BreakingChangeInfo {
    fn from(bc: exports::tern::migration::migration::BreakingChange) -> Self {
        Self {
            description: bc.description,
            mitigation: bc.mitigation.into(),
            affected_sql: bc.affected_sql,
        }
    }
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
    /// Convert from a u8 value (WIT enum representation).
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::DualWrite,
            1 => Self::Backfill,
            2 => Self::Ratchet,
            _ => Self::Destructive,
        }
    }

    /// Get a human-readable name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DualWrite => "dual-write",
            Self::Backfill => "backfill",
            Self::Ratchet => "ratchet",
            Self::Destructive => "destructive",
        }
    }
}

impl std::fmt::Display for MitigationStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<exports::tern::migration::migration::MitigationStrategy> for MitigationStrategy {
    fn from(s: exports::tern::migration::migration::MitigationStrategy) -> Self {
        use exports::tern::migration::migration::MitigationStrategy as WitStrategy;
        match s {
            WitStrategy::DualWrite => Self::DualWrite,
            WitStrategy::Backfill => Self::Backfill,
            WitStrategy::Ratchet => Self::Ratchet,
            WitStrategy::Destructive => Self::Destructive,
        }
    }
}

/// A SQL statement with metadata.
#[derive(Debug, Clone)]
pub struct StatementInfo {
    /// The SQL statement text.
    pub sql: String,
    /// Human-readable description.
    pub description: String,
    /// Sequence number (1-indexed).
    pub sequence: u32,
}

impl From<exports::tern::migration::migration::Statement> for StatementInfo {
    fn from(s: exports::tern::migration::migration::Statement) -> Self {
        Self {
            sql: s.sql,
            description: s.description,
            sequence: s.sequence,
        }
    }
}

/// Configuration for the migration runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Enable fuel-based execution limits.
    pub enable_fuel: bool,
    /// Maximum fuel for execution (if enabled).
    pub max_fuel: u64,
    /// Enable epoch interruption.
    pub enable_epoch_interruption: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            enable_fuel: false,
            max_fuel: 10_000_000_000, // 10 billion ops
            enable_epoch_interruption: false,
        }
    }
}

/// Host state wrapper for the WebAssembly store.
///
/// This wraps our `HostState` along with the resource table required by
/// the component model.
pub struct StoreState {
    /// Our host state.
    pub host: HostState,
    /// Resource table for component model resources.
    pub table: ResourceTable,
}

impl StoreState {
    /// Create a new store state.
    pub fn new(host: HostState) -> Self {
        Self {
            host,
            table: ResourceTable::new(),
        }
    }
}

/// Implementation of the database host interface.
impl tern::migration::database::Host for StoreState {
    fn execute(&mut self, sql: String) -> Result<u64, tern::migration::database::DbError> {
        // Use blocking execution since we're in sync context
        let rt = tokio::runtime::Handle::try_current();
        match rt {
            Ok(handle) => {
                let result =
                    handle.block_on(async { host::database::execute(&mut self.host, &sql).await });
                match result {
                    Ok(rows) => Ok(rows),
                    Err(e) => Err(tern::migration::database::DbError {
                        message: e.message,
                        code: e.code,
                        constraint_name: e.constraint_name,
                        table_name: e.table_name,
                    }),
                }
            }
            Err(_) => {
                // No runtime, execute synchronously (dry-run mode)
                if self.host.is_dry_run() {
                    self.host.record_statement(&sql);
                    Ok(0)
                } else {
                    Err(tern::migration::database::DbError {
                        message: "No async runtime available".to_string(),
                        code: None,
                        constraint_name: None,
                        table_name: None,
                    })
                }
            }
        }
    }

    fn query(&mut self, sql: String) -> Result<String, tern::migration::database::DbError> {
        // Use blocking execution since we're in sync context
        let rt = tokio::runtime::Handle::try_current();
        match rt {
            Ok(handle) => {
                let result =
                    handle.block_on(async { host::database::query(&mut self.host, &sql).await });
                match result {
                    Ok(json) => Ok(json),
                    Err(e) => Err(tern::migration::database::DbError {
                        message: e.message,
                        code: e.code,
                        constraint_name: e.constraint_name,
                        table_name: e.table_name,
                    }),
                }
            }
            Err(_) => {
                // No runtime, return empty result (dry-run mode)
                if self.host.is_dry_run() {
                    self.host.record_statement(&sql);
                    Ok("[]".to_string())
                } else {
                    Err(tern::migration::database::DbError {
                        message: "No async runtime available".to_string(),
                        code: None,
                        constraint_name: None,
                        table_name: None,
                    })
                }
            }
        }
    }
}

/// Implementation of the log host interface.
impl tern::migration::log::Host for StoreState {
    fn log(&mut self, level: tern::migration::log::Level, message: String) {
        let level = match level {
            tern::migration::log::Level::Debug => LogLevel::Debug,
            tern::migration::log::Level::Info => LogLevel::Info,
            tern::migration::log::Level::Warn => LogLevel::Warn,
            tern::migration::log::Level::Error => LogLevel::Error,
        };
        host::log::log_message(&mut self.host, level, &message);
    }
}

/// Runtime for loading and executing migration WebAssembly components.
///
/// The runtime wraps a Wasmtime engine and provides methods to:
/// - Load components from bytes
/// - Extract metadata from components
/// - Execute migrations
pub struct MigrationRuntime {
    /// The Wasmtime engine.
    engine: Engine,
    /// The loaded WebAssembly component.
    component: Component,
    /// Runtime configuration.
    config: RuntimeConfig,
}

impl MigrationRuntime {
    /// Create a new runtime from component bytes.
    pub fn from_bytes(component_bytes: &[u8]) -> Result<Self, RuntimeError> {
        Self::from_bytes_with_config(component_bytes, RuntimeConfig::default())
    }

    /// Create a new runtime from component bytes with custom configuration.
    pub fn from_bytes_with_config(
        component_bytes: &[u8],
        config: RuntimeConfig,
    ) -> Result<Self, RuntimeError> {
        // Create Wasmtime configuration
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);

        if config.enable_fuel {
            wasmtime_config.consume_fuel(true);
        }

        if config.enable_epoch_interruption {
            wasmtime_config.epoch_interruption(true);
        }

        // Create engine
        let engine = Engine::new(&wasmtime_config).map_err(RuntimeError::engine_creation)?;

        // Load component
        let component =
            Component::new(&engine, component_bytes).map_err(RuntimeError::component_load)?;

        Ok(Self {
            engine,
            component,
            config,
        })
    }

    /// Create a store with the given host state.
    fn create_store(&self, host_state: HostState) -> Store<StoreState> {
        let state = StoreState::new(host_state);
        let mut store = Store::new(&self.engine, state);

        if self.config.enable_fuel {
            let _ = store.set_fuel(self.config.max_fuel);
        }

        store
    }

    /// Instantiate the component and get the migration interface.
    fn instantiate(&self, store: &mut Store<StoreState>) -> Result<TernMigration, RuntimeError> {
        let mut linker = wasmtime::component::Linker::<StoreState>::new(&self.engine);

        // Add host function implementations
        // Use HasSelf<T> to indicate the closure returns &mut T where T implements Host
        tern::migration::database::add_to_linker::<StoreState, HasSelf<StoreState>>(
            &mut linker,
            |s| s,
        )
        .map_err(RuntimeError::linking)?;
        tern::migration::log::add_to_linker::<StoreState, HasSelf<StoreState>>(&mut linker, |s| s)
            .map_err(RuntimeError::linking)?;

        // Instantiate
        TernMigration::instantiate(store, &self.component, &linker)
            .map_err(RuntimeError::instantiation)
    }

    /// Get migration metadata without executing.
    ///
    /// This calls the component's `describe` export to retrieve metadata.
    pub fn describe(&self) -> Result<MigrationMetadata, RuntimeError> {
        let mut store = self.create_store(HostState::dry_run());
        let instance = self.instantiate(&mut store)?;

        let metadata = instance
            .tern_migration_migration()
            .call_describe(&mut store)
            .map_err(|e| RuntimeError::function_call("describe", e))?;

        Ok(metadata.into())
    }

    /// Get all SQL statements without executing.
    ///
    /// This calls the component's `get-statements` export.
    pub fn get_statements(&self) -> Result<Vec<StatementInfo>, RuntimeError> {
        let mut store = self.create_store(HostState::dry_run());
        let instance = self.instantiate(&mut store)?;

        let statements = instance
            .tern_migration_migration()
            .call_get_statements(&mut store)
            .map_err(|e| RuntimeError::function_call("get-statements", e))?;

        Ok(statements.into_iter().map(Into::into).collect())
    }

    /// Execute the migration.
    ///
    /// This calls the component's `run` export with the given host state.
    pub fn run(&self, host_state: HostState) -> Result<HostState, RuntimeError> {
        let mut store = self.create_store(host_state);
        let instance = self.instantiate(&mut store)?;

        let result = instance
            .tern_migration_migration()
            .call_run(&mut store)
            .map_err(|e| RuntimeError::function_call("run", e))?;

        // Check result
        match result {
            Ok(()) => Ok(store.into_data().host),
            Err(err) => Err(RuntimeError::migration_failed(err)),
        }
    }

    /// Execute the migration in dry-run mode.
    ///
    /// Returns the list of SQL statements that would be executed.
    pub fn run_dry(&self) -> Result<Vec<String>, RuntimeError> {
        let host_state = HostState::dry_run();
        let result_state = self.run(host_state)?;
        Ok(result_state.dry_run_statements().to_vec())
    }
}

/// Builder for `MigrationRuntime`.
pub struct MigrationRuntimeBuilder {
    config: RuntimeConfig,
}

impl MigrationRuntimeBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: RuntimeConfig::default(),
        }
    }

    /// Enable fuel-based execution limits.
    pub fn with_fuel(mut self, max_fuel: u64) -> Self {
        self.config.enable_fuel = true;
        self.config.max_fuel = max_fuel;
        self
    }

    /// Enable epoch interruption.
    pub fn with_epoch_interruption(mut self) -> Self {
        self.config.enable_epoch_interruption = true;
        self
    }

    /// Build the runtime from component bytes.
    pub fn build(self, component_bytes: &[u8]) -> Result<MigrationRuntime, RuntimeError> {
        MigrationRuntime::from_bytes_with_config(component_bytes, self.config)
    }
}

impl Default for MigrationRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod runtime_config_tests {
        use super::*;

        #[test]
        fn default_config() {
            let config = RuntimeConfig::default();
            assert!(!config.enable_fuel);
            assert!(!config.enable_epoch_interruption);
            assert_eq!(config.max_fuel, 10_000_000_000);
        }
    }

    mod mitigation_strategy_tests {
        use super::*;

        #[test]
        fn from_u8_dual_write() {
            assert_eq!(
                MitigationStrategy::from_u8(0),
                MitigationStrategy::DualWrite
            );
        }

        #[test]
        fn from_u8_backfill() {
            assert_eq!(MitigationStrategy::from_u8(1), MitigationStrategy::Backfill);
        }

        #[test]
        fn from_u8_ratchet() {
            assert_eq!(MitigationStrategy::from_u8(2), MitigationStrategy::Ratchet);
        }

        #[test]
        fn from_u8_destructive() {
            assert_eq!(
                MitigationStrategy::from_u8(3),
                MitigationStrategy::Destructive
            );
        }

        #[test]
        fn from_u8_unknown_defaults_to_destructive() {
            assert_eq!(
                MitigationStrategy::from_u8(255),
                MitigationStrategy::Destructive
            );
        }

        #[test]
        fn as_str() {
            assert_eq!(MitigationStrategy::DualWrite.as_str(), "dual-write");
            assert_eq!(MitigationStrategy::Backfill.as_str(), "backfill");
            assert_eq!(MitigationStrategy::Ratchet.as_str(), "ratchet");
            assert_eq!(MitigationStrategy::Destructive.as_str(), "destructive");
        }

        #[test]
        fn display() {
            assert_eq!(format!("{}", MitigationStrategy::DualWrite), "dual-write");
        }
    }

    mod migration_metadata_tests {
        use super::*;

        fn create_metadata(breaking_changes: Vec<BreakingChangeInfo>) -> MigrationMetadata {
            MigrationMetadata {
                id: "test_id".to_string(),
                description: "Test migration".to_string(),
                breaking_changes,
                statement_count: 5,
                source_state_hash: "source".to_string(),
                target_state_hash: "target".to_string(),
                compiled_at: "2024-01-15T10:00:00Z".to_string(),
            }
        }

        #[test]
        fn has_breaking_changes_false_when_empty() {
            let metadata = create_metadata(vec![]);
            assert!(!metadata.has_breaking_changes());
        }

        #[test]
        fn has_breaking_changes_true_when_present() {
            let bc = BreakingChangeInfo {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::DualWrite,
                affected_sql: vec![],
            };
            let metadata = create_metadata(vec![bc]);
            assert!(metadata.has_breaking_changes());
        }

        #[test]
        fn has_destructive_changes_false_for_dual_write() {
            let bc = BreakingChangeInfo {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::DualWrite,
                affected_sql: vec![],
            };
            let metadata = create_metadata(vec![bc]);
            assert!(!metadata.has_destructive_changes());
        }

        #[test]
        fn has_destructive_changes_true_for_destructive() {
            let bc = BreakingChangeInfo {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::Destructive,
                affected_sql: vec![],
            };
            let metadata = create_metadata(vec![bc]);
            assert!(metadata.has_destructive_changes());
        }

        #[test]
        fn is_safe_true_when_no_breaking_changes() {
            let metadata = create_metadata(vec![]);
            assert!(metadata.is_safe());
        }

        #[test]
        fn is_safe_false_when_breaking_changes() {
            let bc = BreakingChangeInfo {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::DualWrite,
                affected_sql: vec![],
            };
            let metadata = create_metadata(vec![bc]);
            assert!(!metadata.is_safe());
        }
    }

    mod store_state_tests {
        use super::*;

        #[test]
        fn new_creates_state() {
            let host = HostState::new();
            let state = StoreState::new(host);
            assert!(state.host.is_dry_run());
        }
    }

    mod runtime_builder_tests {
        use super::*;

        #[test]
        fn new_creates_default_config() {
            let builder = MigrationRuntimeBuilder::new();
            assert!(!builder.config.enable_fuel);
            assert!(!builder.config.enable_epoch_interruption);
        }

        #[test]
        fn with_fuel_enables_fuel() {
            let builder = MigrationRuntimeBuilder::new().with_fuel(1000);
            assert!(builder.config.enable_fuel);
            assert_eq!(builder.config.max_fuel, 1000);
        }

        #[test]
        fn with_epoch_interruption_enables_epoch() {
            let builder = MigrationRuntimeBuilder::new().with_epoch_interruption();
            assert!(builder.config.enable_epoch_interruption);
        }

        #[test]
        fn default_trait() {
            let builder = MigrationRuntimeBuilder::default();
            assert!(!builder.config.enable_fuel);
        }
    }

    mod statement_info_tests {
        use super::*;

        #[test]
        fn fields_accessible() {
            let stmt = StatementInfo {
                sql: "CREATE TABLE t (id INT)".to_string(),
                description: "Create table".to_string(),
                sequence: 1,
            };
            assert_eq!(stmt.sql, "CREATE TABLE t (id INT)");
            assert_eq!(stmt.description, "Create table");
            assert_eq!(stmt.sequence, 1);
        }

        #[test]
        fn clone() {
            let stmt = StatementInfo {
                sql: "SELECT 1".to_string(),
                description: "Test".to_string(),
                sequence: 1,
            };
            let cloned = stmt.clone();
            assert_eq!(stmt.sql, cloned.sql);
        }

        #[test]
        fn debug() {
            let stmt = StatementInfo {
                sql: "SELECT 1".to_string(),
                description: "Test".to_string(),
                sequence: 1,
            };
            let debug_str = format!("{:?}", stmt);
            assert!(debug_str.contains("SELECT 1"));
        }
    }

    mod breaking_change_info_tests {
        use super::*;

        #[test]
        fn fields_accessible() {
            let bc = BreakingChangeInfo {
                description: "Dropping table".to_string(),
                mitigation: MitigationStrategy::Destructive,
                affected_sql: vec!["DROP TABLE users".to_string()],
            };
            assert_eq!(bc.description, "Dropping table");
            assert_eq!(bc.mitigation, MitigationStrategy::Destructive);
            assert_eq!(bc.affected_sql.len(), 1);
        }

        #[test]
        fn clone() {
            let bc = BreakingChangeInfo {
                description: "Test".to_string(),
                mitigation: MitigationStrategy::Backfill,
                affected_sql: vec![],
            };
            let cloned = bc.clone();
            assert_eq!(bc.description, cloned.description);
        }
    }
}
