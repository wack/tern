//! AOT (Ahead-of-Time) Compilation for Migration Executables
//!
//! This module provides functionality to compile WebAssembly components to
//! native machine code using Wasmtime's AOT compilation capabilities.
//!
//! # Overview
//!
//! The AOT compiler takes a composed WASI component (runner + migration) and
//! produces a serialized format that can be quickly loaded at runtime. This
//! serialized format contains precompiled machine code for the target platform.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    AOT Compilation Flow                     │
//! │                                                             │
//! │  Input: Composed WASI Component (Wasm bytes)                │
//! │         ↓                                                   │
//! │  1. Create Wasmtime Engine with target config               │
//! │         ↓                                                   │
//! │  2. Load component into engine (compiles to native)         │
//! │         ↓                                                   │
//! │  3. Serialize compiled component                            │
//! │         ↓                                                   │
//! │  Output: Serialized native code (cwasm format)              │
//! │                                                             │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use tern::db::compile::aot::{AotCompiler, AotTarget};
//!
//! // Create compiler for native platform
//! let compiler = AotCompiler::new()?;
//!
//! // Or for a specific target
//! let compiler = AotCompiler::for_target(AotTarget::Aarch64Linux)?;
//!
//! // Compile component to native
//! let result = compiler.compile(&component_bytes)?;
//!
//! // Save to file
//! result.save_to_file(Path::new("migration.cwasm"))?;
//! ```
//!
//! # Standalone Executables
//!
//! True standalone executables (that don't require the Wasmtime runtime to be
//! installed) require bundling the serialized code with a minimal loader. This
//! is planned for future implementation. Currently, the output is a `.cwasm`
//! file that can be loaded with `Component::deserialize()`.

use std::path::Path;

use wasmtime::{Config, Engine};

use super::error::CompileError;

// =============================================================================
// Target Platform
// =============================================================================

/// Target platform for AOT compilation.
///
/// Wasmtime supports cross-compilation to different target architectures.
/// The compiled output is specific to the target platform and cannot be
/// used on other platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AotTarget {
    /// Native platform (current host). This is the default.
    #[default]
    Native,
    /// Linux x86_64 (GNU libc).
    X86_64Linux,
    /// Linux aarch64 (ARM64, GNU libc).
    Aarch64Linux,
    /// macOS x86_64 (Intel).
    X86_64MacOS,
    /// macOS aarch64 (Apple Silicon).
    Aarch64MacOS,
    /// Windows x86_64 (MSVC).
    X86_64Windows,
}

impl AotTarget {
    /// Get the Wasmtime target triple string.
    ///
    /// Returns `None` for `Native` (uses default configuration).
    pub fn triple(&self) -> Option<&'static str> {
        match self {
            Self::Native => None,
            Self::X86_64Linux => Some("x86_64-unknown-linux-gnu"),
            Self::Aarch64Linux => Some("aarch64-unknown-linux-gnu"),
            Self::X86_64MacOS => Some("x86_64-apple-darwin"),
            Self::Aarch64MacOS => Some("aarch64-apple-darwin"),
            Self::X86_64Windows => Some("x86_64-pc-windows-msvc"),
        }
    }

    /// Get a human-readable name for this target.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::X86_64Linux => "x86_64-linux",
            Self::Aarch64Linux => "aarch64-linux",
            Self::X86_64MacOS => "x86_64-macos",
            Self::Aarch64MacOS => "aarch64-macos",
            Self::X86_64Windows => "x86_64-windows",
        }
    }

    /// Get the typical file extension for executables on this target.
    pub fn executable_extension(&self) -> &'static str {
        match self {
            Self::X86_64Windows => ".exe",
            _ => "",
        }
    }

    /// Parse a target from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" => Some(Self::Native),
            "x86_64-linux" | "x86_64-unknown-linux-gnu" => Some(Self::X86_64Linux),
            "aarch64-linux" | "aarch64-unknown-linux-gnu" => Some(Self::Aarch64Linux),
            "x86_64-macos" | "x86_64-apple-darwin" => Some(Self::X86_64MacOS),
            "aarch64-macos" | "aarch64-apple-darwin" => Some(Self::Aarch64MacOS),
            "x86_64-windows" | "x86_64-pc-windows-msvc" => Some(Self::X86_64Windows),
            _ => None,
        }
    }

    /// Get all available targets.
    pub fn all() -> &'static [Self] {
        &[
            Self::Native,
            Self::X86_64Linux,
            Self::Aarch64Linux,
            Self::X86_64MacOS,
            Self::Aarch64MacOS,
            Self::X86_64Windows,
        ]
    }
}

impl std::fmt::Display for AotTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// =============================================================================
// AOT Compiler
// =============================================================================

/// AOT compiler using Wasmtime.
///
/// The compiler creates native machine code from WebAssembly components.
/// The output is in Wasmtime's serialized format (`.cwasm`) which can be
/// quickly loaded without recompilation.
pub struct AotCompiler {
    engine: Engine,
    target: AotTarget,
}

impl AotCompiler {
    /// Create a new AOT compiler for the native platform.
    pub fn new() -> Result<Self, CompileError> {
        Self::for_target(AotTarget::Native)
    }

    /// Create an AOT compiler for a specific target platform.
    pub fn for_target(target: AotTarget) -> Result<Self, CompileError> {
        let mut config = Config::new();

        // Enable component model support
        config.wasm_component_model(true);

        // Enable Cranelift optimizations
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);

        // Configure target if not native
        if let Some(triple) = target.triple() {
            config.target(triple).map_err(|e| CompileError::AotError {
                message: format!("Failed to set target '{}': {}", triple, e),
            })?;
        }

        let engine = Engine::new(&config).map_err(|e| CompileError::EngineCreationError {
            message: format!("Failed to create Wasmtime engine: {}", e),
        })?;

        Ok(Self { engine, target })
    }

    /// Get the target platform for this compiler.
    pub fn target(&self) -> AotTarget {
        self.target
    }

    /// Get a reference to the Wasmtime engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Compile a WebAssembly component to native code.
    ///
    /// This loads the component into the engine (which compiles it to native
    /// machine code) and then serializes the compiled code.
    pub fn compile(&self, component_bytes: &[u8]) -> Result<AotOutput, CompileError> {
        // Load the component (this compiles it to native code)
        let component = wasmtime::component::Component::new(&self.engine, component_bytes)
            .map_err(|e| CompileError::ComponentLoadError {
                message: format!("Failed to load component: {}", e),
            })?;

        // Serialize the compiled component
        let serialized = component.serialize().map_err(|e| CompileError::AotError {
            message: format!("Failed to serialize component: {}", e),
        })?;

        Ok(AotOutput {
            target: self.target,
            serialized,
            input_size: component_bytes.len(),
        })
    }

    /// Compile a component and save directly to a file.
    ///
    /// This is a convenience method that combines `compile()` and `save_to_file()`.
    pub fn compile_to_file(
        &self,
        component_bytes: &[u8],
        output_path: &Path,
    ) -> Result<AotResult, CompileError> {
        let output = self.compile(component_bytes)?;
        output.save_to_file(output_path)?;

        Ok(AotResult {
            output_path: output_path.to_path_buf(),
            target: self.target,
            input_size: output.input_size,
            output_size: output.serialized.len(),
        })
    }
}

// =============================================================================
// AOT Output
// =============================================================================

/// Output of AOT compilation.
///
/// Contains the serialized native code that can be saved to a file or
/// loaded back with `Component::deserialize()`.
pub struct AotOutput {
    /// Target platform.
    target: AotTarget,
    /// Serialized component (precompiled native code).
    serialized: Vec<u8>,
    /// Size of the input component in bytes.
    input_size: usize,
}

impl AotOutput {
    /// Get the target platform.
    pub fn target(&self) -> AotTarget {
        self.target
    }

    /// Get the serialized bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.serialized
    }

    /// Get the size of the serialized output.
    pub fn output_size(&self) -> usize {
        self.serialized.len()
    }

    /// Get the size of the input component.
    pub fn input_size(&self) -> usize {
        self.input_size
    }

    /// Calculate the compression ratio (output / input).
    pub fn size_ratio(&self) -> f64 {
        if self.input_size == 0 {
            0.0
        } else {
            self.serialized.len() as f64 / self.input_size as f64
        }
    }

    /// Save the serialized component to a file.
    ///
    /// The output file can be loaded with `Component::deserialize()` on the
    /// same target platform.
    pub fn save_to_file(&self, path: &Path) -> Result<(), CompileError> {
        std::fs::write(path, &self.serialized).map_err(|e| CompileError::AotError {
            message: format!("Failed to write AOT output to '{}': {}", path.display(), e),
        })
    }

    /// Consume and return the serialized bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.serialized
    }
}

// =============================================================================
// AOT Result
// =============================================================================

/// Result of AOT compilation to a file.
#[derive(Debug)]
pub struct AotResult {
    /// Path to the output file.
    pub output_path: std::path::PathBuf,
    /// Target platform.
    pub target: AotTarget,
    /// Size of the input Wasm component in bytes.
    pub input_size: usize,
    /// Size of the output serialized file in bytes.
    pub output_size: usize,
}

impl AotResult {
    /// Calculate the compression ratio (output / input).
    pub fn size_ratio(&self) -> f64 {
        if self.input_size == 0 {
            0.0
        } else {
            self.output_size as f64 / self.input_size as f64
        }
    }
}

// =============================================================================
// Standalone Executable (Future)
// =============================================================================

/// Marker for future standalone executable support.
///
/// Creating true standalone executables (that don't require Wasmtime to be
/// installed) requires bundling the serialized component with a minimal
/// loader/runtime. This is planned for future implementation.
///
/// Current options being considered:
///
/// 1. **wasmtime run --invoke**: Ship serialized `.cwasm` with instructions
///    to run via `wasmtime run`.
///
/// 2. **Custom loader**: Generate a small C stub that embeds the serialized
///    component and links against Wasmtime's C API, then compile with the
///    system C compiler.
///
/// 3. **Self-extracting archive**: Bundle the serialized component with a
///    pre-compiled loader binary for each target platform.
///
/// For now, we output `.cwasm` files that can be loaded with Wasmtime's
/// `Component::deserialize()` or run with `wasmtime run`.
#[allow(dead_code)]
mod standalone_future {
    /// Placeholder for standalone executable builder.
    pub struct StandaloneBuilder {
        _private: (),
    }

    impl StandaloneBuilder {
        /// Create a standalone executable from serialized component.
        ///
        /// This is not yet implemented. Returns an error with a message
        /// explaining the current state.
        pub fn create_executable(
            _serialized: &[u8],
            _output_path: &std::path::Path,
        ) -> Result<(), super::CompileError> {
            Err(super::CompileError::AotError {
                message: "Standalone executable creation is not yet implemented. \
                         The output .cwasm file can be run with 'wasmtime run'."
                    .to_string(),
            })
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    mod target_tests {
        use super::*;

        #[test]
        fn native_has_no_triple() {
            assert_eq!(AotTarget::Native.triple(), None);
        }

        #[test]
        fn x86_64_linux_triple() {
            assert_eq!(
                AotTarget::X86_64Linux.triple(),
                Some("x86_64-unknown-linux-gnu")
            );
        }

        #[test]
        fn aarch64_linux_triple() {
            assert_eq!(
                AotTarget::Aarch64Linux.triple(),
                Some("aarch64-unknown-linux-gnu")
            );
        }

        #[test]
        fn x86_64_macos_triple() {
            assert_eq!(AotTarget::X86_64MacOS.triple(), Some("x86_64-apple-darwin"));
        }

        #[test]
        fn aarch64_macos_triple() {
            assert_eq!(
                AotTarget::Aarch64MacOS.triple(),
                Some("aarch64-apple-darwin")
            );
        }

        #[test]
        fn x86_64_windows_triple() {
            assert_eq!(
                AotTarget::X86_64Windows.triple(),
                Some("x86_64-pc-windows-msvc")
            );
        }

        #[test]
        fn target_names() {
            assert_eq!(AotTarget::Native.name(), "native");
            assert_eq!(AotTarget::X86_64Linux.name(), "x86_64-linux");
            assert_eq!(AotTarget::Aarch64Linux.name(), "aarch64-linux");
            assert_eq!(AotTarget::X86_64MacOS.name(), "x86_64-macos");
            assert_eq!(AotTarget::Aarch64MacOS.name(), "aarch64-macos");
            assert_eq!(AotTarget::X86_64Windows.name(), "x86_64-windows");
        }

        #[test]
        fn executable_extensions() {
            assert_eq!(AotTarget::Native.executable_extension(), "");
            assert_eq!(AotTarget::X86_64Linux.executable_extension(), "");
            assert_eq!(AotTarget::X86_64Windows.executable_extension(), ".exe");
        }

        #[test]
        fn parse_native() {
            assert_eq!(AotTarget::parse("native"), Some(AotTarget::Native));
        }

        #[test]
        fn parse_x86_64_linux() {
            assert_eq!(
                AotTarget::parse("x86_64-linux"),
                Some(AotTarget::X86_64Linux)
            );
            assert_eq!(
                AotTarget::parse("x86_64-unknown-linux-gnu"),
                Some(AotTarget::X86_64Linux)
            );
        }

        #[test]
        fn parse_aarch64_linux() {
            assert_eq!(
                AotTarget::parse("aarch64-linux"),
                Some(AotTarget::Aarch64Linux)
            );
            assert_eq!(
                AotTarget::parse("aarch64-unknown-linux-gnu"),
                Some(AotTarget::Aarch64Linux)
            );
        }

        #[test]
        fn parse_macos() {
            assert_eq!(
                AotTarget::parse("x86_64-macos"),
                Some(AotTarget::X86_64MacOS)
            );
            assert_eq!(
                AotTarget::parse("aarch64-macos"),
                Some(AotTarget::Aarch64MacOS)
            );
        }

        #[test]
        fn parse_windows() {
            assert_eq!(
                AotTarget::parse("x86_64-windows"),
                Some(AotTarget::X86_64Windows)
            );
        }

        #[test]
        fn parse_unknown() {
            assert_eq!(AotTarget::parse("unknown"), None);
            assert_eq!(AotTarget::parse("riscv64-linux"), None);
        }

        #[test]
        fn all_targets_included() {
            let all = AotTarget::all();
            assert!(all.contains(&AotTarget::Native));
            assert!(all.contains(&AotTarget::X86_64Linux));
            assert!(all.contains(&AotTarget::Aarch64Linux));
            assert!(all.contains(&AotTarget::X86_64MacOS));
            assert!(all.contains(&AotTarget::Aarch64MacOS));
            assert!(all.contains(&AotTarget::X86_64Windows));
        }

        #[test]
        fn display_format() {
            assert_eq!(format!("{}", AotTarget::Native), "native");
            assert_eq!(format!("{}", AotTarget::X86_64Linux), "x86_64-linux");
        }

        #[test]
        fn default_is_native() {
            assert_eq!(AotTarget::default(), AotTarget::Native);
        }
    }

    mod compiler_tests {
        use super::*;

        #[test]
        fn create_native_compiler() {
            let compiler = AotCompiler::new();
            assert!(compiler.is_ok());

            let compiler = compiler.unwrap();
            assert_eq!(compiler.target(), AotTarget::Native);
        }

        #[test]
        fn create_compiler_for_target() {
            // Native should always work
            let compiler = AotCompiler::for_target(AotTarget::Native);
            assert!(compiler.is_ok());
        }

        #[test]
        fn compiler_has_engine() {
            let compiler = AotCompiler::new().unwrap();
            // Just verify we can access the engine
            let _ = compiler.engine();
        }
    }

    mod output_tests {
        use super::*;

        fn test_output() -> AotOutput {
            AotOutput {
                target: AotTarget::Native,
                serialized: vec![0; 1000],
                input_size: 500,
            }
        }

        #[test]
        fn output_sizes() {
            let output = test_output();
            assert_eq!(output.input_size(), 500);
            assert_eq!(output.output_size(), 1000);
        }

        #[test]
        fn output_size_ratio() {
            let output = test_output();
            assert!((output.size_ratio() - 2.0).abs() < 0.001);
        }

        #[test]
        fn output_size_ratio_zero_input() {
            let output = AotOutput {
                target: AotTarget::Native,
                serialized: vec![0; 100],
                input_size: 0,
            };
            assert_eq!(output.size_ratio(), 0.0);
        }

        #[test]
        fn output_bytes() {
            let output = test_output();
            assert_eq!(output.bytes().len(), 1000);
        }

        #[test]
        fn output_into_bytes() {
            let output = test_output();
            let bytes = output.into_bytes();
            assert_eq!(bytes.len(), 1000);
        }

        #[test]
        fn output_target() {
            let output = test_output();
            assert_eq!(output.target(), AotTarget::Native);
        }
    }

    mod result_tests {
        use super::*;

        #[test]
        fn result_size_ratio() {
            let result = AotResult {
                output_path: std::path::PathBuf::from("test.cwasm"),
                target: AotTarget::Native,
                input_size: 1000,
                output_size: 2000,
            };
            assert!((result.size_ratio() - 2.0).abs() < 0.001);
        }

        #[test]
        fn result_size_ratio_zero_input() {
            let result = AotResult {
                output_path: std::path::PathBuf::from("test.cwasm"),
                target: AotTarget::Native,
                input_size: 0,
                output_size: 100,
            };
            assert_eq!(result.size_ratio(), 0.0);
        }
    }
}
