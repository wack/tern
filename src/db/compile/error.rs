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
}
