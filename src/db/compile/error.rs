//! Compilation error types for migration component generation.
//!
//! This module defines the error types that can occur during the compilation
//! of migrations into WebAssembly components and standalone executables.

// False positive from thiserror macro expansion on enum variant fields
#![allow(unused_assignments)]

use std::path::PathBuf;

use miette::Diagnostic;
use thiserror::Error;

use super::Target;

/// Errors that can occur during migration compilation.
#[derive(Debug, Error, Diagnostic)]
pub enum CompileError {
    /// Failed to render operations to SQL.
    #[error("failed to render operations to SQL: {message}")]
    RenderError {
        /// The error message.
        message: String,
    },

    /// Invalid migration data.
    #[error("invalid migration data: {message}")]
    InvalidMigration {
        /// The error message.
        message: String,
    },

    /// Failed to generate source code.
    #[error("failed to generate source code: {message}")]
    CodegenError {
        /// The error message.
        message: String,
    },

    /// Failed to write generated file.
    #[error("failed to write file: {path}")]
    #[diagnostic(code(tern::compile::io_error))]
    IoError {
        /// The file path that failed.
        path: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Template rendering failed.
    #[error("template error: {message}")]
    TemplateError {
        /// The error message.
        message: String,
    },

    /// Failed to create temporary directory for build.
    #[error("failed to create temporary build directory")]
    #[diagnostic(code(tern::compile::temp_dir_error))]
    TempDirError {
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// cargo-component build failed.
    #[error("cargo-component build failed: {message}")]
    #[diagnostic(
        code(tern::compile::cargo_component_error),
        help("Ensure cargo-component is installed: cargo install cargo-component")
    )]
    CargoComponentError {
        /// The error message.
        message: String,
        /// Standard error output, if available.
        stderr: Option<String>,
    },

    /// Rust toolchain build failed.
    #[error("cargo build failed: {message}")]
    #[diagnostic(code(tern::compile::cargo_build_error))]
    CargoBuildError {
        /// The error message.
        message: String,
        /// Standard error output, if available.
        stderr: Option<String>,
    },

    /// Cross-compilation target not installed.
    #[error("cross-compilation target '{target}' is not installed")]
    #[diagnostic(
        code(tern::compile::target_not_installed),
        help("Install the target with: rustup target add {target}")
    )]
    TargetNotInstalled {
        /// The target triple that is not installed.
        target: String,
    },

    /// Unsupported target platform.
    #[error("unsupported target platform: {target:?}")]
    #[diagnostic(code(tern::compile::unsupported_target))]
    UnsupportedTarget {
        /// The target platform.
        target: Target,
    },

    /// Runner crate not found.
    #[error("runner crate not found at: {path}")]
    #[diagnostic(
        code(tern::compile::runner_not_found),
        help("The runner crate path may be misconfigured")
    )]
    RunnerCrateNotFound {
        /// The path that was searched.
        path: PathBuf,
    },

    /// Failed to read Wasm component bytes.
    #[error("failed to read Wasm component: {path}")]
    #[diagnostic(code(tern::compile::wasm_read_error))]
    WasmReadError {
        /// The path to the Wasm component.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Failed to write output executable.
    #[error("failed to write output executable: {path}")]
    #[diagnostic(code(tern::compile::output_write_error))]
    OutputWriteError {
        /// The output path.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },

    /// Schema diff produced no changes.
    #[error("schema diff produced no changes")]
    #[diagnostic(
        code(tern::compile::no_changes),
        help("The source and target schemas are identical")
    )]
    NoChanges,

    /// Embedded component not available.
    #[error("embedded {component} component is not available")]
    #[diagnostic(
        code(tern::compile::component_not_available),
        help("The {component} component requires the WASI rewrite to be completed")
    )]
    ComponentNotAvailable {
        /// The name of the component.
        component: String,
    },

    /// Invalid WebAssembly component.
    #[error("invalid {component} component: {reason}")]
    #[diagnostic(code(tern::compile::invalid_component))]
    InvalidComponent {
        /// The name of the component.
        component: String,
        /// The reason the component is invalid.
        reason: String,
    },

    /// Wasmtime engine creation failed.
    #[error("failed to create Wasmtime engine: {message}")]
    #[diagnostic(code(tern::compile::engine_error))]
    EngineCreationError {
        /// The error message.
        message: String,
    },

    /// Failed to load WebAssembly component.
    #[error("failed to load WebAssembly component: {message}")]
    #[diagnostic(code(tern::compile::component_load_error))]
    ComponentLoadError {
        /// The error message.
        message: String,
    },

    /// Failed to serialize component for AOT compilation.
    #[error("failed to serialize component: {message}")]
    #[diagnostic(code(tern::compile::serialization_error))]
    SerializationError {
        /// The error message.
        message: String,
    },

    /// Component composition failed.
    #[error("failed to compose components: {message}")]
    #[diagnostic(code(tern::compile::composition_error))]
    CompositionError {
        /// The error message.
        message: String,
    },

    /// Data component generation failed.
    #[error("failed to generate data component: {message}")]
    #[diagnostic(code(tern::compile::data_component_error))]
    DataComponentError {
        /// The error message.
        message: String,
    },

    /// AOT compilation failed.
    #[error("AOT compilation failed: {message}")]
    #[diagnostic(code(tern::compile::aot_error))]
    AotError {
        /// The error message.
        message: String,
    },

    /// OCI image generation failed.
    #[error("OCI image generation failed: {message}")]
    #[diagnostic(code(tern::compile::oci_error))]
    OciGenerationError {
        /// The error message.
        message: String,
    },
}

impl CompileError {
    /// Create a render error.
    pub fn render(message: impl Into<String>) -> Self {
        Self::RenderError {
            message: message.into(),
        }
    }

    /// Create an invalid migration error.
    pub fn invalid_migration(message: impl Into<String>) -> Self {
        Self::InvalidMigration {
            message: message.into(),
        }
    }

    /// Create a codegen error.
    pub fn codegen(message: impl Into<String>) -> Self {
        Self::CodegenError {
            message: message.into(),
        }
    }

    /// Create an IO error.
    pub fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::IoError {
            path: path.into(),
            source,
        }
    }

    /// Create a template error.
    pub fn template(message: impl Into<String>) -> Self {
        Self::TemplateError {
            message: message.into(),
        }
    }

    /// Create a temp directory error.
    pub fn temp_dir(source: std::io::Error) -> Self {
        Self::TempDirError { source }
    }

    /// Create a cargo-component error.
    pub fn cargo_component(message: impl Into<String>, stderr: Option<String>) -> Self {
        Self::CargoComponentError {
            message: message.into(),
            stderr,
        }
    }

    /// Create a cargo build error.
    pub fn cargo_build(message: impl Into<String>, stderr: Option<String>) -> Self {
        Self::CargoBuildError {
            message: message.into(),
            stderr,
        }
    }

    /// Create a target not installed error.
    pub fn target_not_installed(target: impl Into<String>) -> Self {
        Self::TargetNotInstalled {
            target: target.into(),
        }
    }

    /// Create an unsupported target error.
    pub fn unsupported_target(target: Target) -> Self {
        Self::UnsupportedTarget { target }
    }

    /// Create a runner crate not found error.
    pub fn runner_not_found(path: impl Into<PathBuf>) -> Self {
        Self::RunnerCrateNotFound { path: path.into() }
    }

    /// Create a Wasm read error.
    pub fn wasm_read(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::WasmReadError {
            path: path.into(),
            source,
        }
    }

    /// Create an output write error.
    pub fn output_write(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::OutputWriteError {
            path: path.into(),
            source,
        }
    }

    /// Create a no changes error.
    pub fn no_changes() -> Self {
        Self::NoChanges
    }

    /// Create a component not available error.
    pub fn component_not_available(component: impl Into<String>) -> Self {
        Self::ComponentNotAvailable {
            component: component.into(),
        }
    }

    /// Create an invalid component error.
    pub fn invalid_component(component: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidComponent {
            component: component.into(),
            reason: reason.into(),
        }
    }

    /// Create an engine creation error.
    pub fn engine_creation(error: impl std::fmt::Display) -> Self {
        Self::EngineCreationError {
            message: error.to_string(),
        }
    }

    /// Create a component load error.
    pub fn component_load(error: impl std::fmt::Display) -> Self {
        Self::ComponentLoadError {
            message: error.to_string(),
        }
    }

    /// Create a serialization error.
    pub fn serialization(error: impl std::fmt::Display) -> Self {
        Self::SerializationError {
            message: error.to_string(),
        }
    }

    /// Create a composition error.
    pub fn composition(message: impl Into<String>) -> Self {
        Self::CompositionError {
            message: message.into(),
        }
    }

    /// Create a data component generation error.
    pub fn data_component(message: impl Into<String>) -> Self {
        Self::DataComponentError {
            message: message.into(),
        }
    }

    /// Create an AOT compilation error.
    pub fn aot(message: impl Into<String>) -> Self {
        Self::AotError {
            message: message.into(),
        }
    }

    /// Create an OCI image generation error.
    pub fn oci_generation(message: impl Into<String>) -> Self {
        Self::OciGenerationError {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_error_display() {
        let err = CompileError::render("test error");
        assert_eq!(
            format!("{}", err),
            "failed to render operations to SQL: test error"
        );
    }

    #[test]
    fn invalid_migration_error_display() {
        let err = CompileError::invalid_migration("missing operations");
        assert_eq!(
            format!("{}", err),
            "invalid migration data: missing operations"
        );
    }

    #[test]
    fn codegen_error_display() {
        let err = CompileError::codegen("syntax error");
        assert_eq!(
            format!("{}", err),
            "failed to generate source code: syntax error"
        );
    }

    #[test]
    fn template_error_display() {
        let err = CompileError::template("invalid template");
        assert_eq!(format!("{}", err), "template error: invalid template");
    }

    #[test]
    fn cargo_component_error_display() {
        let err = CompileError::cargo_component("build failed", Some("error details".to_string()));
        assert_eq!(
            format!("{}", err),
            "cargo-component build failed: build failed"
        );
    }

    #[test]
    fn cargo_build_error_display() {
        let err = CompileError::cargo_build("compilation error", None);
        assert_eq!(format!("{}", err), "cargo build failed: compilation error");
    }

    #[test]
    fn target_not_installed_error_display() {
        let err = CompileError::target_not_installed("x86_64-unknown-linux-musl");
        assert_eq!(
            format!("{}", err),
            "cross-compilation target 'x86_64-unknown-linux-musl' is not installed"
        );
    }

    #[test]
    fn unsupported_target_error_display() {
        let err = CompileError::unsupported_target(Target::X86_64Windows);
        assert!(format!("{}", err).contains("unsupported target platform"));
    }

    #[test]
    fn runner_not_found_error_display() {
        let err = CompileError::runner_not_found("/path/to/runner");
        assert!(format!("{}", err).contains("runner crate not found"));
    }

    #[test]
    fn no_changes_error_display() {
        let err = CompileError::no_changes();
        assert_eq!(format!("{}", err), "schema diff produced no changes");
    }

    #[test]
    fn component_not_available_error_display() {
        let err = CompileError::component_not_available("runner");
        assert_eq!(
            format!("{}", err),
            "embedded runner component is not available"
        );
    }

    #[test]
    fn invalid_component_error_display() {
        let err = CompileError::invalid_component("guest", "invalid magic number");
        assert_eq!(
            format!("{}", err),
            "invalid guest component: invalid magic number"
        );
    }

    #[test]
    fn engine_creation_error_display() {
        let err = CompileError::engine_creation("configuration error");
        assert_eq!(
            format!("{}", err),
            "failed to create Wasmtime engine: configuration error"
        );
    }

    #[test]
    fn component_load_error_display() {
        let err = CompileError::component_load("parse error");
        assert_eq!(
            format!("{}", err),
            "failed to load WebAssembly component: parse error"
        );
    }

    #[test]
    fn serialization_error_display() {
        let err = CompileError::serialization("encoding failed");
        assert_eq!(
            format!("{}", err),
            "failed to serialize component: encoding failed"
        );
    }

    #[test]
    fn composition_error_display() {
        let err = CompileError::composition("interface mismatch");
        assert_eq!(
            format!("{}", err),
            "failed to compose components: interface mismatch"
        );
    }

    #[test]
    fn data_component_error_display() {
        let err = CompileError::data_component("invalid metadata");
        assert_eq!(
            format!("{}", err),
            "failed to generate data component: invalid metadata"
        );
    }

    #[test]
    fn aot_error_display() {
        let err = CompileError::aot("serialization failed");
        assert_eq!(
            format!("{}", err),
            "AOT compilation failed: serialization failed"
        );
    }

    #[test]
    fn oci_generation_error_display() {
        let err = CompileError::oci_generation("manifest creation failed");
        assert_eq!(
            format!("{}", err),
            "OCI image generation failed: manifest creation failed"
        );
    }
}
