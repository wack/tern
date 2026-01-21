//! Compilation error types for migration component generation.
//!
//! This module defines the error types that can occur during the compilation
//! of migrations into WebAssembly components.

use miette::Diagnostic;
use thiserror::Error;

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
}
