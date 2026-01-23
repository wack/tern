//! Embedded WebAssembly components for migration compilation.
//!
//! This module provides access to pre-compiled WebAssembly components that are
//! embedded in the Tern binary at compile time. These components are used by
//! the [`ExecutableBuilder`](super::ExecutableBuilder) to create standalone
//! migration executables without invoking external tools.
//!
//! # Components
//!
//! Two components are embedded:
//!
//! - **Runner Component**: A WASI CLI application that orchestrates migration
//!   execution. It imports the migration interface and uses WASI for CLI args,
//!   networking (database connections), and I/O.
//!
//! - **Guest Component**: A generic migration executor that imports SQL data
//!   and exports the migration interface. At composition time, it is linked
//!   with a data component containing the actual SQL statements.
//!
//! # Build Process
//!
//! During `cargo build` for Tern:
//!
//! 1. `build.rs` compiles `tern-migration-runner` to `wasm32-wasip2`
//! 2. `build.rs` compiles `tern-migration-guest` to `wasm32-wasip2`
//! 3. Environment variables point to the compiled `.wasm` files
//! 4. This module uses `include_bytes!` to embed them
//!
//! # Future: Component Embedding
//!
//! Once the runner and guest crates are rewritten for WASI, this module will
//! use `include_bytes!` to embed the compiled components:
//!
//! ```ignore
//! pub const RUNNER_COMPONENT: &[u8] = include_bytes!(env!("TERN_RUNNER_WASM_PATH"));
//! pub const GUEST_COMPONENT: &[u8] = include_bytes!(env!("TERN_GUEST_WASM_PATH"));
//! ```

use super::CompileError;

// =============================================================================
// Embedded Component Constants
// =============================================================================

/// Pre-compiled migration runner component (wasm32-wasip2).
///
/// This component is a WASI CLI application that:
/// - Parses CLI arguments (--database-url, --dry-run, etc.)
/// - Establishes database connections via WASI sockets
/// - Imports and calls the migration interface
/// - Reports results to stdout/stderr
///
/// # Availability
///
/// This constant is `Some` when the WASI components have been compiled during
/// the build process. Use [`runner_component()`] to access it with proper error
/// handling.
#[cfg(feature = "embedded-wasi")]
pub const RUNNER_COMPONENT: Option<&[u8]> = Some(include_bytes!(env!("TERN_RUNNER_WASM_PATH")));

#[cfg(not(feature = "embedded-wasi"))]
pub const RUNNER_COMPONENT: Option<&[u8]> = None;

/// Pre-compiled migration guest component (wasm32-wasip2).
///
/// This component is a generic migration executor that:
/// - Imports the `migration-data` interface (SQL statements and metadata)
/// - Imports database and logging interfaces from the runner
/// - Exports the `migration` interface (describe, run, get-statements)
///
/// At composition time, it is linked with:
/// 1. A data component providing `migration-data` (generated at runtime)
/// 2. The runner component providing `database` and `log` interfaces
///
/// # Availability
///
/// This constant is `Some` when the WASI components have been compiled during
/// the build process. Use [`guest_component()`] to access it with proper error
/// handling.
#[cfg(feature = "embedded-wasi")]
pub const GUEST_COMPONENT: Option<&[u8]> = Some(include_bytes!(env!("TERN_GUEST_WASM_PATH")));

#[cfg(not(feature = "embedded-wasi"))]
pub const GUEST_COMPONENT: Option<&[u8]> = None;

// =============================================================================
// Component Accessors
// =============================================================================

/// Returns the embedded runner component bytes.
///
/// # Errors
///
/// Returns [`CompileError::ComponentNotAvailable`] if the runner component
/// is not yet available (pending WASI rewrite of `tern-migration-runner`).
pub fn runner_component() -> Result<&'static [u8], CompileError> {
    RUNNER_COMPONENT.ok_or_else(|| CompileError::component_not_available("runner"))
}

/// Returns the embedded guest component bytes.
///
/// # Errors
///
/// Returns [`CompileError::ComponentNotAvailable`] if the guest component
/// is not yet available (pending WASI rewrite of `tern-migration-guest`).
pub fn guest_component() -> Result<&'static [u8], CompileError> {
    GUEST_COMPONENT.ok_or_else(|| CompileError::component_not_available("guest"))
}

/// Returns the size of the embedded runner component in bytes.
///
/// Returns `0` if the component is not yet available.
pub fn runner_component_size() -> usize {
    RUNNER_COMPONENT.map(|c| c.len()).unwrap_or(0)
}

/// Returns the size of the embedded guest component in bytes.
///
/// Returns `0` if the component is not yet available.
pub fn guest_component_size() -> usize {
    GUEST_COMPONENT.map(|c| c.len()).unwrap_or(0)
}

/// Returns whether the embedded components are available.
///
/// Both runner and guest components must be available for the new
/// WASI-based compilation pipeline to work.
pub fn components_available() -> bool {
    RUNNER_COMPONENT.is_some() && GUEST_COMPONENT.is_some()
}

// =============================================================================
// Component Validation
// =============================================================================

/// Validates that component bytes are a valid WebAssembly module or component.
///
/// Checks:
/// - Magic number: `\0asm` (bytes 0-3)
/// - Version: 1 for modules, or component layer version
///
/// # Arguments
///
/// * `bytes` - The WebAssembly bytes to validate
/// * `name` - Name of the component (for error messages)
///
/// # Errors
///
/// Returns an error if the bytes are not valid WebAssembly.
pub fn validate_wasm_bytes(bytes: &[u8], name: &str) -> Result<(), CompileError> {
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

    // Check version (bytes 4-7)
    // Version 1 = core module, version 0x0d (13) = component (layer)
    let version = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    if version != 1 && version != 0x0d00_0001 {
        return Err(CompileError::invalid_component(
            name,
            format!("unsupported version: {:#x}", version),
        ));
    }

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod availability_tests {
        use super::*;

        #[test]
        fn runner_component_is_available() {
            assert!(RUNNER_COMPONENT.is_some());
            assert!(runner_component().is_ok());
        }

        #[test]
        fn guest_component_is_available() {
            assert!(GUEST_COMPONENT.is_some());
            assert!(guest_component().is_ok());
        }

        #[test]
        fn components_are_available() {
            assert!(components_available());
        }

        #[test]
        fn component_sizes_nonzero() {
            assert!(runner_component_size() > 0);
            assert!(guest_component_size() > 0);
        }

        #[test]
        fn runner_component_has_wasm_magic() {
            let bytes = runner_component().expect("runner component should be available");
            assert!(bytes.len() >= 4);
            assert_eq!(
                &bytes[0..4],
                b"\0asm",
                "runner should have WASM magic number"
            );
        }

        #[test]
        fn guest_component_has_wasm_magic() {
            let bytes = guest_component().expect("guest component should be available");
            assert!(bytes.len() >= 4);
            assert_eq!(
                &bytes[0..4],
                b"\0asm",
                "guest should have WASM magic number"
            );
        }
    }

    mod validation_tests {
        use super::*;

        #[test]
        fn validate_rejects_empty_bytes() {
            let result = validate_wasm_bytes(&[], "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_rejects_too_small() {
            let result = validate_wasm_bytes(&[0, 0, 0, 0], "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_rejects_invalid_magic() {
            let bytes = [0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00];
            let result = validate_wasm_bytes(&bytes, "test");
            assert!(result.is_err());
        }

        #[test]
        fn validate_accepts_core_module() {
            // Valid core module header: \0asm + version 1
            let bytes = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
            let result = validate_wasm_bytes(&bytes, "test");
            assert!(result.is_ok());
        }

        #[test]
        fn validate_accepts_component() {
            // Valid component header: \0asm + component version
            let bytes = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x0d];
            let result = validate_wasm_bytes(&bytes, "test");
            assert!(result.is_ok());
        }
    }
}
