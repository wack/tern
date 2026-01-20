//! Identifier quoting utilities for SQL generation.

use serde::{Deserialize, Serialize};

/// Strategy for quoting SQL identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IdentifierQuoting {
    /// Always quote identifiers with double quotes.
    Always,
    /// Never quote identifiers (may fail for reserved words).
    Never,
    /// Quote only when needed (reserved words, special characters, case sensitivity).
    #[default]
    WhenNeeded,
}

impl IdentifierQuoting {
    /// Quote an identifier according to this strategy.
    #[must_use]
    pub fn quote(&self, name: &str) -> String {
        match self {
            Self::Always => format!("\"{}\"", escape_identifier(name)),
            Self::Never => name.to_string(),
            Self::WhenNeeded => {
                if needs_quoting(name) {
                    format!("\"{}\"", escape_identifier(name))
                } else {
                    name.to_string()
                }
            }
        }
    }

    /// Format a qualified name (schema.object).
    #[must_use]
    pub fn qualified(&self, schema: &str, name: &str) -> String {
        format!("{}.{}", self.quote(schema), self.quote(name))
    }
}

/// Escape an identifier for use within double quotes.
///
/// In PostgreSQL, double quotes inside an identifier are escaped by doubling them.
fn escape_identifier(name: &str) -> String {
    name.replace('"', "\"\"")
}

/// Check if an identifier needs quoting.
///
/// An identifier needs quoting if:
/// - It's a PostgreSQL reserved word
/// - It contains special characters (anything other than a-z, 0-9, _)
/// - It starts with a digit
/// - It contains uppercase letters (PostgreSQL folds to lowercase without quotes)
fn needs_quoting(name: &str) -> bool {
    // Empty names always need quoting
    if name.is_empty() {
        return true;
    }

    // Check first character: must be letter or underscore
    let first = name.chars().next().unwrap();
    if !first.is_ascii_lowercase() && first != '_' {
        return true;
    }

    // Check remaining characters: must be letter, digit, underscore, or $
    for c in name.chars() {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' && c != '$' {
            return true;
        }
    }

    // Check for reserved words
    if is_reserved_word(name) {
        return true;
    }

    false
}

/// Check if a name is a PostgreSQL reserved word.
///
/// This is a subset of common reserved words. The full list is much longer.
fn is_reserved_word(name: &str) -> bool {
    // Convert to lowercase for comparison
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "all"
            | "analyse"
            | "analyze"
            | "and"
            | "any"
            | "array"
            | "as"
            | "asc"
            | "asymmetric"
            | "authorization"
            | "between"
            | "binary"
            | "both"
            | "case"
            | "cast"
            | "check"
            | "collate"
            | "collation"
            | "column"
            | "concurrently"
            | "constraint"
            | "create"
            | "cross"
            | "current_catalog"
            | "current_date"
            | "current_role"
            | "current_schema"
            | "current_time"
            | "current_timestamp"
            | "current_user"
            | "default"
            | "deferrable"
            | "desc"
            | "distinct"
            | "do"
            | "else"
            | "end"
            | "except"
            | "false"
            | "fetch"
            | "for"
            | "foreign"
            | "freeze"
            | "from"
            | "full"
            | "grant"
            | "group"
            | "having"
            | "ilike"
            | "in"
            | "index"
            | "initially"
            | "inner"
            | "intersect"
            | "into"
            | "is"
            | "isnull"
            | "join"
            | "lateral"
            | "leading"
            | "left"
            | "like"
            | "limit"
            | "localtime"
            | "localtimestamp"
            | "natural"
            | "not"
            | "notnull"
            | "null"
            | "offset"
            | "on"
            | "only"
            | "or"
            | "order"
            | "outer"
            | "overlaps"
            | "placing"
            | "primary"
            | "references"
            | "returning"
            | "right"
            | "select"
            | "session_user"
            | "similar"
            | "some"
            | "symmetric"
            | "table"
            | "tablesample"
            | "then"
            | "to"
            | "trailing"
            | "true"
            | "union"
            | "unique"
            | "user"
            | "using"
            | "variadic"
            | "verbose"
            | "when"
            | "where"
            | "window"
            | "with"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_always() {
        let q = IdentifierQuoting::Always;
        assert_eq!(q.quote("users"), "\"users\"");
        assert_eq!(q.quote("MyTable"), "\"MyTable\"");
    }

    #[test]
    fn quote_never() {
        let q = IdentifierQuoting::Never;
        assert_eq!(q.quote("users"), "users");
        assert_eq!(q.quote("MyTable"), "MyTable");
    }

    #[test]
    fn quote_when_needed_simple() {
        let q = IdentifierQuoting::WhenNeeded;
        // Simple lowercase identifiers don't need quoting
        assert_eq!(q.quote("users"), "users");
        assert_eq!(q.quote("user_accounts"), "user_accounts");
        assert_eq!(q.quote("table1"), "table1");
    }

    #[test]
    fn quote_when_needed_uppercase() {
        let q = IdentifierQuoting::WhenNeeded;
        // Uppercase letters require quoting
        assert_eq!(q.quote("MyTable"), "\"MyTable\"");
        assert_eq!(q.quote("USER"), "\"USER\"");
    }

    #[test]
    fn quote_when_needed_reserved_word() {
        let q = IdentifierQuoting::WhenNeeded;
        // Reserved words require quoting
        assert_eq!(q.quote("select"), "\"select\"");
        assert_eq!(q.quote("table"), "\"table\"");
        assert_eq!(q.quote("user"), "\"user\"");
    }

    #[test]
    fn quote_when_needed_special_chars() {
        let q = IdentifierQuoting::WhenNeeded;
        // Special characters require quoting
        assert_eq!(q.quote("my-table"), "\"my-table\"");
        assert_eq!(q.quote("my table"), "\"my table\"");
    }

    #[test]
    fn quote_when_needed_starts_with_digit() {
        let q = IdentifierQuoting::WhenNeeded;
        assert_eq!(q.quote("1table"), "\"1table\"");
    }

    #[test]
    fn escape_embedded_quotes() {
        let q = IdentifierQuoting::Always;
        assert_eq!(q.quote("my\"table"), "\"my\"\"table\"");
    }

    #[test]
    fn qualified_name() {
        let q = IdentifierQuoting::WhenNeeded;
        assert_eq!(q.qualified("public", "users"), "public.users");
        assert_eq!(q.qualified("public", "User"), "public.\"User\"");
        assert_eq!(q.qualified("MySchema", "users"), "\"MySchema\".users");
    }
}
