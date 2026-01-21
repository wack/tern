//! Component composition for migration executables.
//!
//! This module handles composing WebAssembly components to create complete
//! migration executables. The composition process links components together
//! by satisfying imports with exports from other components.
//!
//! # Composition Pipeline
//!
//! ```text
//! ┌─────────────────┐   ┌─────────────────┐
//! │ Guest Component │ + │ Data Component  │
//! │ (embedded)      │   │ (generated)     │
//! └────────┬────────┘   └────────┬────────┘
//!          │                     │
//!          └──────────┬──────────┘
//!                     ▼
//!          ┌─────────────────────┐
//!          │ Migration Component │
//!          │ (exports migration) │
//!          └──────────┬──────────┘
//!                     │
//!                     │  ┌─────────────────┐
//!                     │  │ Runner Component│
//!                     │  │ (embedded)      │
//!                     │  └────────┬────────┘
//!                     │           │
//!                     └─────┬─────┘
//!                           ▼
//!               ┌───────────────────────┐
//!               │ Complete Component    │
//!               │ (WASI CLI application)│
//!               └───────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use tern::db::compile::composer::ComponentComposer;
//!
//! let composer = ComponentComposer::new()?;
//!
//! // Step 1: Compose guest + data → migration
//! let migration = composer.compose_migration(guest_bytes, data_bytes)?;
//!
//! // Step 2: Compose runner + migration → complete
//! let complete = composer.compose_executable(runner_bytes, &migration)?;
//! ```

use wasmtime::{Config, Engine};

use super::error::CompileError;

// =============================================================================
// Component Composer
// =============================================================================

/// Composes WebAssembly components using the component model.
///
/// The composer links components together by connecting exports to imports.
/// This is used to:
///
/// 1. Compose the guest component with a data component to create a
///    migration component that exports the `migration` interface.
///
/// 2. Compose the runner component with a migration component to create
///    a complete WASI CLI application.
pub struct ComponentComposer {
    /// Wasmtime engine configured for component model.
    engine: Engine,
}

impl ComponentComposer {
    /// Create a new component composer.
    ///
    /// # Errors
    ///
    /// Returns an error if the Wasmtime engine cannot be created.
    pub fn new() -> Result<Self, CompileError> {
        let mut config = Config::new();
        config.wasm_component_model(true);

        let engine = Engine::new(&config).map_err(CompileError::engine_creation)?;

        Ok(Self { engine })
    }

    /// Create a composer with a custom engine.
    ///
    /// The engine must have the component model enabled.
    pub fn with_engine(engine: Engine) -> Self {
        Self { engine }
    }

    /// Returns a reference to the underlying engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compose a guest component with a data component.
    ///
    /// The data component provides the `migration-data` interface that the
    /// guest component imports. The result is a migration component that
    /// exports the `migration` interface.
    ///
    /// # Arguments
    ///
    /// * `guest_bytes` - The pre-compiled guest component bytes
    /// * `data_bytes` - The generated data component bytes
    ///
    /// # Returns
    ///
    /// Returns the composed migration component bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Either component is invalid
    /// - The components cannot be linked (interface mismatch)
    /// - Composition fails for any other reason
    ///
    /// # Current Status
    ///
    /// This method returns a placeholder error until the guest component
    /// is rewritten for WASI (Phase 5). The actual composition will use
    /// the `wasm-compose` crate to link the components.
    pub fn compose_migration(
        &self,
        guest_bytes: &[u8],
        data_bytes: &[u8],
    ) -> Result<Vec<u8>, CompileError> {
        // Validate inputs
        self.validate_component_bytes(guest_bytes, "guest")?;
        self.validate_component_bytes(data_bytes, "data")?;

        // TODO: Implement actual composition using wasm-compose
        //
        // The composition process will:
        // 1. Parse both components
        // 2. Match data component's exports to guest's imports
        // 3. Produce a new component that:
        //    - Embeds the data component's implementation
        //    - Re-exports the guest's exports (migration interface)
        //
        // For now, return an error indicating the components aren't ready
        Err(CompileError::composition(
            "Guest component not yet available (pending WASI rewrite in Phase 5)",
        ))
    }

    /// Compose a runner component with a migration component.
    ///
    /// The migration component provides the `migration` interface that the
    /// runner component imports. The result is a complete WASI CLI application
    /// that can be AOT compiled to a native executable.
    ///
    /// # Arguments
    ///
    /// * `runner_bytes` - The pre-compiled runner component bytes
    /// * `migration_bytes` - The composed migration component bytes
    ///
    /// # Returns
    ///
    /// Returns the composed executable component bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Either component is invalid
    /// - The components cannot be linked (interface mismatch)
    /// - Composition fails for any other reason
    ///
    /// # Current Status
    ///
    /// This method returns a placeholder error until the runner component
    /// is rewritten for WASI (Phase 4). The actual composition will use
    /// the `wasm-compose` crate to link the components.
    pub fn compose_executable(
        &self,
        runner_bytes: &[u8],
        migration_bytes: &[u8],
    ) -> Result<Vec<u8>, CompileError> {
        // Validate inputs
        self.validate_component_bytes(runner_bytes, "runner")?;
        self.validate_component_bytes(migration_bytes, "migration")?;

        // TODO: Implement actual composition using wasm-compose
        //
        // The composition process will:
        // 1. Parse both components
        // 2. Match migration component's exports to runner's imports
        // 3. Produce a new component that:
        //    - Has only WASI imports (satisfied by the runtime)
        //    - Exports the WASI CLI entry point
        //
        // For now, return an error indicating the components aren't ready
        Err(CompileError::composition(
            "Runner component not yet available (pending WASI rewrite in Phase 4)",
        ))
    }

    /// Validate that bytes represent a valid WebAssembly component.
    fn validate_component_bytes(&self, bytes: &[u8], name: &str) -> Result<(), CompileError> {
        // Check minimum size
        if bytes.len() < 8 {
            return Err(CompileError::invalid_component(
                name,
                "too small to be valid WebAssembly",
            ));
        }

        // Check magic number: \0asm
        if &bytes[0..4] != b"\0asm" {
            return Err(CompileError::invalid_component(
                name,
                "invalid magic number (expected \\0asm)",
            ));
        }

        // For components, we could do more validation here using wasmparser
        // For now, basic validation is sufficient

        Ok(())
    }
}

// =============================================================================
// Composition Configuration
// =============================================================================

/// Configuration options for component composition.
#[derive(Debug, Clone)]
pub struct CompositionConfig {
    /// Whether to validate components before composition.
    pub validate: bool,
    /// Whether to optimize the output component.
    pub optimize: bool,
}

impl Default for CompositionConfig {
    fn default() -> Self {
        Self {
            validate: true,
            optimize: false,
        }
    }
}

impl CompositionConfig {
    /// Create a new configuration with validation enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable component validation.
    pub fn with_validation(mut self, validate: bool) -> Self {
        self.validate = validate;
        self
    }

    /// Enable or disable output optimization.
    pub fn with_optimization(mut self, optimize: bool) -> Self {
        self.optimize = optimize;
        self
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a minimal valid Wasm module (not component)
    fn minimal_wasm_module() -> Vec<u8> {
        vec![
            0x00, 0x61, 0x73, 0x6d, // \0asm magic
            0x01, 0x00, 0x00, 0x00, // version 1
        ]
    }

    // Helper to create a minimal component
    fn minimal_wasm_component() -> Vec<u8> {
        vec![
            0x00, 0x61, 0x73, 0x6d, // \0asm magic
            0x01, 0x00, 0x00, 0x0d, // component version
        ]
    }

    mod composer_tests {
        use super::*;

        #[test]
        fn new_creates_composer() {
            let composer = ComponentComposer::new();
            assert!(composer.is_ok());
        }

        #[test]
        fn with_engine_creates_composer() {
            let mut config = Config::new();
            config.wasm_component_model(true);
            let engine = Engine::new(&config).unwrap();

            let _composer = ComponentComposer::with_engine(engine);
            // Engine is successfully created with component model enabled
        }

        #[test]
        fn engine_returns_reference() {
            let composer = ComponentComposer::new().unwrap();
            let _engine = composer.engine();
        }
    }

    mod validation_tests {
        use super::*;

        #[test]
        fn validate_rejects_empty_bytes() {
            let composer = ComponentComposer::new().unwrap();
            let result = composer.validate_component_bytes(&[], "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_rejects_too_small() {
            let composer = ComponentComposer::new().unwrap();
            let result = composer.validate_component_bytes(&[0, 0, 0, 0], "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_rejects_invalid_magic() {
            let composer = ComponentComposer::new().unwrap();
            let bytes = vec![0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
            let result = composer.validate_component_bytes(&bytes, "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_accepts_valid_module() {
            let composer = ComponentComposer::new().unwrap();
            let bytes = minimal_wasm_module();
            let result = composer.validate_component_bytes(&bytes, "test");
            assert!(result.is_ok());
        }

        #[test]
        fn validate_accepts_valid_component() {
            let composer = ComponentComposer::new().unwrap();
            let bytes = minimal_wasm_component();
            let result = composer.validate_component_bytes(&bytes, "test");
            assert!(result.is_ok());
        }
    }

    mod compose_migration_tests {
        use super::*;

        #[test]
        fn compose_migration_validates_guest() {
            let composer = ComponentComposer::new().unwrap();
            let data = minimal_wasm_component();

            let result = composer.compose_migration(&[], &data);
            assert!(result.is_err());

            let err = result.unwrap_err();
            assert!(err.to_string().contains("guest"));
        }

        #[test]
        fn compose_migration_validates_data() {
            let composer = ComponentComposer::new().unwrap();
            let guest = minimal_wasm_component();

            let result = composer.compose_migration(&guest, &[]);
            assert!(result.is_err());

            let err = result.unwrap_err();
            assert!(err.to_string().contains("data"));
        }

        #[test]
        fn compose_migration_not_yet_implemented() {
            let composer = ComponentComposer::new().unwrap();
            let guest = minimal_wasm_component();
            let data = minimal_wasm_component();

            let result = composer.compose_migration(&guest, &data);
            assert!(result.is_err());

            // Should indicate pending WASI rewrite
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Phase 5"));
        }
    }

    mod compose_executable_tests {
        use super::*;

        #[test]
        fn compose_executable_validates_runner() {
            let composer = ComponentComposer::new().unwrap();
            let migration = minimal_wasm_component();

            let result = composer.compose_executable(&[], &migration);
            assert!(result.is_err());

            let err = result.unwrap_err();
            assert!(err.to_string().contains("runner"));
        }

        #[test]
        fn compose_executable_validates_migration() {
            let composer = ComponentComposer::new().unwrap();
            let runner = minimal_wasm_component();

            let result = composer.compose_executable(&runner, &[]);
            assert!(result.is_err());

            let err = result.unwrap_err();
            assert!(err.to_string().contains("migration"));
        }

        #[test]
        fn compose_executable_not_yet_implemented() {
            let composer = ComponentComposer::new().unwrap();
            let runner = minimal_wasm_component();
            let migration = minimal_wasm_component();

            let result = composer.compose_executable(&runner, &migration);
            assert!(result.is_err());

            // Should indicate pending WASI rewrite
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Phase 4"));
        }
    }

    mod config_tests {
        use super::*;

        #[test]
        fn default_config_validates() {
            let config = CompositionConfig::default();
            assert!(config.validate);
            assert!(!config.optimize);
        }

        #[test]
        fn new_creates_default() {
            let config = CompositionConfig::new();
            assert!(config.validate);
        }

        #[test]
        fn with_validation_sets_flag() {
            let config = CompositionConfig::new().with_validation(false);
            assert!(!config.validate);
        }

        #[test]
        fn with_optimization_sets_flag() {
            let config = CompositionConfig::new().with_optimization(true);
            assert!(config.optimize);
        }

        #[test]
        fn builder_pattern_chains() {
            let config = CompositionConfig::new()
                .with_validation(false)
                .with_optimization(true);

            assert!(!config.validate);
            assert!(config.optimize);
        }
    }
}
