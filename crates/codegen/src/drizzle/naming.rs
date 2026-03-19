//! JavaScript/TypeScript identifier naming and sanitization.
//!
//! This module handles conversion of PostgreSQL identifiers to valid JavaScript
//! identifiers, including handling of reserved words and camelCase conversion.

/// JavaScript reserved words (keywords and future reserved words).
const JS_KEYWORDS: &[&str] = &[
    "abstract",
    "arguments",
    "await",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "double",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "function",
    "goto",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "int",
    "interface",
    "let",
    "long",
    "native",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "volatile",
    "while",
    "with",
    "yield",
];

/// Checks if a name is a JavaScript reserved word.
pub fn is_js_reserved(name: &str) -> bool {
    JS_KEYWORDS.contains(&name)
}

/// Sanitizes a database identifier for use as a JavaScript identifier.
///
/// Returns `(sanitized_name, needs_explicit_name)` where `needs_explicit_name`
/// is true if the identifier was modified and the original DB name must be
/// passed explicitly.
pub fn sanitize_js_identifier(name: &str) -> (String, bool) {
    if name.is_empty() {
        return ("_empty".to_string(), true);
    }

    let mut sanitized = String::with_capacity(name.len() + 1);
    let mut needs_rename = false;

    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            if c.is_ascii_alphabetic() || c == '_' || c == '$' {
                sanitized.push(c);
            } else if c.is_ascii_digit() {
                sanitized.push('_');
                sanitized.push(c);
                needs_rename = true;
            } else {
                sanitized.push('_');
                needs_rename = true;
            }
        } else if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            sanitized.push(c);
        } else {
            sanitized.push('_');
            needs_rename = true;
        }
    }

    // Collapse multiple consecutive underscores
    let mut collapsed = String::with_capacity(sanitized.len());
    let mut prev_underscore = false;
    for c in sanitized.chars() {
        if c == '_' {
            if !prev_underscore {
                collapsed.push(c);
            } else {
                needs_rename = true;
            }
            prev_underscore = true;
        } else {
            collapsed.push(c);
            prev_underscore = false;
        }
    }
    sanitized = collapsed;

    // Remove trailing underscores unless the original had them
    while sanitized.len() > 1 && sanitized.ends_with('_') && !name.ends_with('_') {
        sanitized.pop();
        needs_rename = true;
    }

    // Handle reserved words
    if is_js_reserved(&sanitized) {
        needs_rename = true;
        sanitized.push('_');
    }

    if sanitized.is_empty() || sanitized == "_" {
        return ("_field".to_string(), true);
    }

    (sanitized, needs_rename)
}

/// Converts a snake_case string to camelCase.
///
/// Examples:
/// - "user_id" -> "userId"
/// - "created_at" -> "createdAt"
/// - "id" -> "id"
/// - "already_camel" -> "alreadyCamel"
pub fn to_camel_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = false;

    for (i, c) in s.chars().enumerate() {
        if c == '_' {
            if i > 0 && !result.is_empty() {
                capitalize_next = true;
            }
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result
}

/// Converts a column name to a JavaScript variable name.
///
/// If `camel_case` is true, converts snake_case to camelCase.
/// Returns `(js_name, needs_explicit_db_name)`.
pub fn to_column_js_name(column_name: &str, camel_case: bool) -> (String, bool) {
    let (sanitized, was_modified) = sanitize_js_identifier(column_name);

    if camel_case {
        let camel = to_camel_case(&sanitized);
        // Need explicit DB name if the camelCase version differs from the original
        let needs_explicit = camel != column_name || was_modified;
        (camel, needs_explicit)
    } else {
        (sanitized, was_modified)
    }
}

/// Converts a table name to a JavaScript variable name.
///
/// Table variables in Drizzle typically use the original DB name as the variable
/// name (e.g., `export const users = pgTable("users", ...)`).
pub fn to_table_js_name(table_name: &str) -> (String, bool) {
    sanitize_js_identifier(table_name)
}

/// Derives a relation name from a foreign key column name.
///
/// Examples:
/// - "author_id" -> "author"
/// - "user_id" -> "user"
/// - "parent_category_id" -> "parentCategory"
/// - "owner" -> "owner"
pub fn to_relation_name(column_name: &str) -> String {
    let base = column_name.strip_suffix("_id").unwrap_or(column_name);
    to_camel_case(base)
}

/// Derives a plural relation name for has-many relations.
///
/// Examples:
/// - "posts" -> "posts"
/// - "user_accounts" -> "userAccounts"
pub fn to_plural_relation_name(table_name: &str) -> String {
    to_camel_case(table_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_js_reserved() {
        assert!(is_js_reserved("class"));
        assert!(is_js_reserved("default"));
        assert!(is_js_reserved("delete"));
        assert!(is_js_reserved("export"));
        assert!(is_js_reserved("import"));
        assert!(is_js_reserved("new"));
        assert!(is_js_reserved("return"));
        assert!(is_js_reserved("this"));
        assert!(is_js_reserved("typeof"));
        assert!(!is_js_reserved("users"));
        assert!(!is_js_reserved("name"));
    }

    #[test]
    fn test_sanitize_js_identifier_valid() {
        let (name, needs) = sanitize_js_identifier("user_id");
        assert_eq!(name, "user_id");
        assert!(!needs);
    }

    #[test]
    fn test_sanitize_js_identifier_reserved() {
        let (name, needs) = sanitize_js_identifier("class");
        assert_eq!(name, "class_");
        assert!(needs);
    }

    #[test]
    fn test_sanitize_js_identifier_starts_with_digit() {
        let (name, needs) = sanitize_js_identifier("1column");
        assert_eq!(name, "_1column");
        assert!(needs);
    }

    #[test]
    fn test_sanitize_js_identifier_special_chars() {
        let (name, needs) = sanitize_js_identifier("column-name");
        assert_eq!(name, "column_name");
        assert!(needs);
    }

    #[test]
    fn test_sanitize_js_identifier_dollar_sign() {
        let (name, needs) = sanitize_js_identifier("$value");
        assert_eq!(name, "$value");
        assert!(!needs);
    }

    #[test]
    fn test_sanitize_js_identifier_empty() {
        let (name, needs) = sanitize_js_identifier("");
        assert_eq!(name, "_empty");
        assert!(needs);
    }

    #[test]
    fn test_sanitize_js_identifier_multiple_underscores() {
        let (name, needs) = sanitize_js_identifier("a__b");
        assert_eq!(name, "a_b");
        assert!(needs);
    }

    #[test]
    fn test_to_camel_case() {
        assert_eq!(to_camel_case("user_id"), "userId");
        assert_eq!(to_camel_case("created_at"), "createdAt");
        assert_eq!(to_camel_case("id"), "id");
        assert_eq!(to_camel_case("some_long_name"), "someLongName");
        assert_eq!(to_camel_case("already"), "already");
    }

    #[test]
    fn test_to_column_js_name_camel() {
        let (name, needs) = to_column_js_name("user_id", true);
        assert_eq!(name, "userId");
        assert!(needs); // camelCase differs from original

        let (name, needs) = to_column_js_name("id", true);
        assert_eq!(name, "id");
        assert!(!needs); // no change
    }

    #[test]
    fn test_to_column_js_name_no_camel() {
        let (name, needs) = to_column_js_name("user_id", false);
        assert_eq!(name, "user_id");
        assert!(!needs);
    }

    #[test]
    fn test_to_table_js_name() {
        let (name, needs) = to_table_js_name("users");
        assert_eq!(name, "users");
        assert!(!needs);

        let (name, needs) = to_table_js_name("order_items");
        assert_eq!(name, "order_items");
        assert!(!needs);
    }

    #[test]
    fn test_to_relation_name() {
        assert_eq!(to_relation_name("author_id"), "author");
        assert_eq!(to_relation_name("user_id"), "user");
        assert_eq!(to_relation_name("parent_category_id"), "parentCategory");
        assert_eq!(to_relation_name("owner"), "owner");
    }

    #[test]
    fn test_to_plural_relation_name() {
        assert_eq!(to_plural_relation_name("posts"), "posts");
        assert_eq!(to_plural_relation_name("user_accounts"), "userAccounts");
    }
}
