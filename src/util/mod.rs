//! Utility functions for terminal output styling.
//!
//! This module provides helper functions for colorizing terminal output using
//! `owo_colors`. The functions are designed to work with `anstream`, which
//! automatically handles color support detection and respects the `--enable-colors`
//! CLI flag.
//!
//! ## Print Helpers
//!
//! Use these macros for all terminal output instead of importing `anstream` directly:
//!
//! - [`info!`] - Informational messages (dimmed)
//! - [`warn!`] - Warning messages to stderr (yellow, bold)
//! - [`output!`] - Plain output for data/results
//! - [`newline!`] - Print a blank line

use owo_colors::{OwoColorize, Style};
use std::fmt::{Display, Formatter, Result as FmtResult};

// =============================================================================
// Print Helper Macros
// =============================================================================

/// Prints an informational message to stdout with dimmed styling.
///
/// Use for progress messages like "Connecting to database..." or "Loading schema...".
///
/// # Examples
/// ```ignore
/// info!("Connecting to database...");
/// info!("Loading schema '{}'...", schema_name);
/// ```
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        anstream::println!("{}", $crate::util::dim(format!($($arg)*)));
    }};
}

/// Prints a warning message to stderr with yellow bold styling.
///
/// Use for warnings that don't prevent operation but need attention.
///
/// # Examples
/// ```ignore
/// warn!("Schema drift detected");
/// warn!("WARNING: {} breaking change(s) detected", count);
/// ```
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        anstream::eprintln!("{}", $crate::util::warning(format!($($arg)*)));
    }};
}

/// Prints plain output to stdout without styling.
///
/// Use for actual program output like formatted data, SQL statements, etc.
///
/// # Examples
/// ```ignore
/// output!("{}", formatted_output);
/// output!("{};", sql_statement);
/// ```
#[macro_export]
macro_rules! output {
    ($($arg:tt)*) => {{
        anstream::println!($($arg)*);
    }};
}

/// Prints a blank line to stdout.
///
/// Use for visual separation in output.
#[macro_export]
macro_rules! newline {
    () => {{
        anstream::println!();
    }};
}

// =============================================================================
// Styled Wrapper
// =============================================================================

/// A wrapper that applies a style to a value when displayed.
pub struct Styled<T> {
    value: T,
    style: Style,
}

impl<T: Display> Display for Styled<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.value.style(self.style))
    }
}

// =============================================================================
// Success / Warning / Error Styles
// =============================================================================

/// Styles text as a success message (green).
///
/// Use for positive outcomes like "Initialized successfully" or "Verification passed".
pub fn success<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().green(),
    }
}

/// Styles text as a warning message (yellow, bold).
///
/// Use for warnings that don't prevent operation but need attention,
/// such as breaking changes or schema drift.
pub fn warning<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().yellow().bold(),
    }
}

/// Styles text as an error message (red, bold).
///
/// Use for failures or critical issues that prevent successful completion.
pub fn error<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().red().bold(),
    }
}

// =============================================================================
// Structural Styles
// =============================================================================

/// Styles text as a header/title (cyan, bold).
///
/// Use for section headers like "Migration History" or "Schema Diff".
pub fn header<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().cyan().bold(),
    }
}

/// Styles text as a label/key (bold).
///
/// Use for field labels like "ID:", "Description:", "State hash:".
pub fn label<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().bold(),
    }
}

/// Styles text as dimmed/secondary (dimmed).
///
/// Use for less important information like hash values or progress messages.
pub fn dim<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().dimmed(),
    }
}

// =============================================================================
// Diff Indicator Styles
// =============================================================================

/// Styles text as an added item (green).
///
/// Use for diff output showing additions (+).
pub fn added<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().green(),
    }
}

/// Styles text as a removed item (red).
///
/// Use for diff output showing removals (-).
pub fn removed<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().red(),
    }
}

/// Styles text as a modified item (yellow).
///
/// Use for diff output showing modifications (~).
pub fn modified<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().yellow(),
    }
}

/// Styles text as a potential rename (magenta).
///
/// Use for diff output showing potential renames (?).
pub fn renamed<T: Display>(text: T) -> Styled<T> {
    Styled {
        value: text,
        style: Style::new().magenta(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_formats_text() {
        let styled = success("done");
        let output = format!("{styled}");
        // The output contains ANSI codes when colors are enabled
        assert!(output.contains("done"));
    }

    #[test]
    fn warning_formats_text() {
        let styled = warning("caution");
        let output = format!("{styled}");
        assert!(output.contains("caution"));
    }

    #[test]
    fn error_formats_text() {
        let styled = error("failed");
        let output = format!("{styled}");
        assert!(output.contains("failed"));
    }

    #[test]
    fn header_formats_text() {
        let styled = header("Title");
        let output = format!("{styled}");
        assert!(output.contains("Title"));
    }

    #[test]
    fn label_formats_text() {
        let styled = label("ID:");
        let output = format!("{styled}");
        assert!(output.contains("ID:"));
    }

    #[test]
    fn dim_formats_text() {
        let styled = dim("hash123");
        let output = format!("{styled}");
        assert!(output.contains("hash123"));
    }

    #[test]
    fn added_formats_text() {
        let styled = added("+ new item");
        let output = format!("{styled}");
        assert!(output.contains("+ new item"));
    }

    #[test]
    fn removed_formats_text() {
        let styled = removed("- old item");
        let output = format!("{styled}");
        assert!(output.contains("- old item"));
    }

    #[test]
    fn modified_formats_text() {
        let styled = modified("~ changed");
        let output = format!("{styled}");
        assert!(output.contains("~ changed"));
    }

    #[test]
    fn renamed_formats_text() {
        let styled = renamed("? maybe renamed");
        let output = format!("{styled}");
        assert!(output.contains("? maybe renamed"));
    }
}
