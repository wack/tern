//! Python identifier naming and sanitization.
//!
//! This module handles conversion of PostgreSQL identifiers to valid Python identifiers,
//! including handling of reserved words, invalid characters, and naming conventions.

use super::ReservedWordStrategy;

/// Python reserved words (keywords).
///
/// These cannot be used as identifiers in Python without modification.
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// Python soft keywords (context-dependent keywords introduced in Python 3.10+).
const PYTHON_SOFT_KEYWORDS: &[&str] = &["match", "case", "_", "type"];

/// Python built-in names that should be avoided to prevent shadowing.
/// Kept for future use to warn about shadowing built-in names.
#[allow(dead_code)]
const PYTHON_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "ascii",
    "bin",
    "bool",
    "breakpoint",
    "bytearray",
    "bytes",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "exec",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "print",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
];

/// Checks if a name is a Python keyword.
pub fn is_python_keyword(name: &str) -> bool {
    PYTHON_KEYWORDS.contains(&name)
}

/// Checks if a name is a Python soft keyword.
pub fn is_python_soft_keyword(name: &str) -> bool {
    PYTHON_SOFT_KEYWORDS.contains(&name)
}

/// Checks if a name is a Python builtin.
/// Kept for future use to warn about shadowing built-in names.
#[allow(dead_code)]
pub fn is_python_builtin(name: &str) -> bool {
    PYTHON_BUILTINS.contains(&name)
}

/// Checks if a name is a reserved word (keyword or soft keyword).
pub fn is_reserved_word(name: &str) -> bool {
    is_python_keyword(name) || is_python_soft_keyword(name)
}

/// Sanitizes a database identifier for use as a Python identifier.
///
/// This function:
/// 1. Replaces invalid characters with underscores
/// 2. Ensures the name doesn't start with a digit
/// 3. Handles reserved words according to the strategy
/// 4. Returns the sanitized name and whether aliasing is needed
///
/// Returns `(sanitized_name, needs_alias)` where `needs_alias` is true if
/// the name was modified and needs a Field(alias="original_name") declaration.
pub fn sanitize_identifier(name: &str, strategy: &ReservedWordStrategy) -> (String, bool) {
    if name.is_empty() {
        return ("_empty".to_string(), true);
    }

    let mut sanitized = String::with_capacity(name.len() + 1);
    let mut needs_alias = false;

    // Process each character
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            // First character must be a letter or underscore
            if c.is_ascii_alphabetic() || c == '_' {
                sanitized.push(c);
            } else if c.is_ascii_digit() {
                // Prefix with underscore if starts with digit
                sanitized.push('_');
                sanitized.push(c);
                needs_alias = true;
            } else {
                // Replace invalid first character with underscore
                sanitized.push('_');
                needs_alias = true;
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            sanitized.push(c);
        } else {
            // Replace invalid characters with underscore
            sanitized.push('_');
            needs_alias = true;
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
                needs_alias = true;
            }
            prev_underscore = true;
        } else {
            collapsed.push(c);
            prev_underscore = false;
        }
    }
    sanitized = collapsed;

    // Remove trailing underscores (unless it's the only character)
    while sanitized.len() > 1 && sanitized.ends_with('_') && !name.ends_with('_') {
        sanitized.pop();
        needs_alias = true;
    }

    // Handle reserved words
    if is_reserved_word(&sanitized) {
        needs_alias = true;
        sanitized = apply_reserved_word_strategy(&sanitized, strategy);
    }

    // Final validation - ensure we have a valid identifier
    if sanitized.is_empty() || sanitized == "_" {
        return ("_field".to_string(), true);
    }

    (sanitized, needs_alias)
}

/// Applies the reserved word strategy to transform a reserved word.
fn apply_reserved_word_strategy(name: &str, strategy: &ReservedWordStrategy) -> String {
    match strategy {
        ReservedWordStrategy::AppendUnderscore => format!("{name}_"),
        ReservedWordStrategy::PrependPrefix(prefix) => format!("{prefix}{name}"),
    }
}

/// Converts a table name to a Python class name (PascalCase).
///
/// Examples:
/// - "users" -> "User"
/// - "user_accounts" -> "UserAccount"
/// - "order_items" -> "OrderItem"
pub fn to_class_name(table_name: &str) -> String {
    let mut result = String::with_capacity(table_name.len());
    let mut capitalize_next = true;

    // Check for pluralization - handle "ies" -> "y" first (e.g., "categories" -> "category")
    let name = if table_name.ends_with("ies") && table_name.len() > 3 {
        let base = &table_name[..table_name.len() - 3];
        return to_pascal_case(base) + "y";
    } else if table_name.ends_with('s')
        && !table_name.ends_with("ss")
        && !table_name.ends_with("us")
        && !table_name.ends_with("is")
    {
        // Simple plural: strip trailing 's'
        &table_name[..table_name.len() - 1]
    } else {
        table_name
    };

    // Convert to PascalCase
    for c in name.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }

    // Handle edge case where result is empty
    if result.is_empty() {
        result = "Model".to_string();
    }

    result
}

/// Converts a string to PascalCase.
fn to_pascal_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }

    result
}

/// Converts a table name to a Python module name (snake_case, singular).
///
/// Examples:
/// - "users" -> "user"
/// - "user_accounts" -> "user_account"
/// - "OrderItems" -> "order_item"
pub fn to_module_name(table_name: &str) -> String {
    let mut result = String::with_capacity(table_name.len());
    let mut prev_was_upper = false;

    for (i, c) in table_name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !prev_was_upper {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_was_upper = true;
        } else if c == '-' || c == ' ' {
            result.push('_');
            prev_was_upper = false;
        } else {
            result.push(c);
            prev_was_upper = false;
        }
    }

    // Remove plural 's' suffix
    if result.ends_with('s')
        && !result.ends_with("ss")
        && !result.ends_with("us")
        && !result.ends_with("is")
        && result.len() > 1
    {
        result.pop();
    } else if result.ends_with("ies") && result.len() > 3 {
        // Handle "ies" -> "y"
        result.truncate(result.len() - 3);
        result.push('y');
    }

    result
}

/// Converts a column name to a Python attribute name (snake_case).
///
/// PostgreSQL column names are typically already in snake_case, but this
/// handles edge cases like mixed case or special characters.
pub fn to_attribute_name(column_name: &str, strategy: &ReservedWordStrategy) -> (String, bool) {
    // First sanitize the identifier
    let (sanitized, needs_alias) = sanitize_identifier(column_name, strategy);

    // Convert to snake_case if needed (handle camelCase input)
    let mut result = String::with_capacity(sanitized.len() + 4);
    let mut prev_was_upper = false;
    let mut prev_was_underscore = true; // Treat start as after underscore

    for c in sanitized.chars() {
        if c.is_ascii_uppercase() {
            if !prev_was_upper && !prev_was_underscore {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_was_upper = true;
            prev_was_underscore = false;
        } else if c == '_' {
            if !prev_was_underscore {
                result.push(c);
            }
            prev_was_upper = false;
            prev_was_underscore = true;
        } else {
            result.push(c);
            prev_was_upper = false;
            prev_was_underscore = false;
        }
    }

    // Check if conversion changed the name
    let conversion_changed = result != sanitized;

    (result, needs_alias || conversion_changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_python_keyword() {
        assert!(is_python_keyword("class"));
        assert!(is_python_keyword("def"));
        assert!(is_python_keyword("if"));
        assert!(is_python_keyword("True"));
        assert!(is_python_keyword("None"));
        assert!(!is_python_keyword("user"));
        assert!(!is_python_keyword("name"));
    }

    #[test]
    fn test_is_python_soft_keyword() {
        assert!(is_python_soft_keyword("match"));
        assert!(is_python_soft_keyword("case"));
        assert!(!is_python_soft_keyword("class"));
    }

    #[test]
    fn test_is_reserved_word() {
        assert!(is_reserved_word("class"));
        assert!(is_reserved_word("match"));
        assert!(!is_reserved_word("user"));
    }

    #[test]
    fn test_sanitize_identifier_valid() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("user_id", &strategy);
        assert_eq!(name, "user_id");
        assert!(!needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_reserved_word() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("class", &strategy);
        assert_eq!(name, "class_");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_reserved_word_prefix() {
        let strategy = ReservedWordStrategy::PrependPrefix("field_".to_string());
        let (name, needs_alias) = sanitize_identifier("class", &strategy);
        assert_eq!(name, "field_class");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_starts_with_digit() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("1column", &strategy);
        assert_eq!(name, "_1column");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_special_characters() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("column-name", &strategy);
        assert_eq!(name, "column_name");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_spaces() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("column name", &strategy);
        assert_eq!(name, "column_name");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_multiple_underscores() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("column__name", &strategy);
        assert_eq!(name, "column_name");
        assert!(needs_alias);
    }

    #[test]
    fn test_sanitize_identifier_empty() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = sanitize_identifier("", &strategy);
        assert_eq!(name, "_empty");
        assert!(needs_alias);
    }

    #[test]
    fn test_to_class_name_simple() {
        assert_eq!(to_class_name("users"), "User");
        assert_eq!(to_class_name("user"), "User");
    }

    #[test]
    fn test_to_class_name_compound() {
        assert_eq!(to_class_name("user_accounts"), "UserAccount");
        assert_eq!(to_class_name("order_items"), "OrderItem");
    }

    #[test]
    fn test_to_class_name_categories() {
        assert_eq!(to_class_name("categories"), "Category");
    }

    #[test]
    fn test_to_class_name_preserves_non_plural() {
        assert_eq!(to_class_name("status"), "Status");
        assert_eq!(to_class_name("address"), "Address");
    }

    #[test]
    fn test_to_module_name_simple() {
        assert_eq!(to_module_name("users"), "user");
        assert_eq!(to_module_name("User"), "user");
    }

    #[test]
    fn test_to_module_name_compound() {
        assert_eq!(to_module_name("user_accounts"), "user_account");
        assert_eq!(to_module_name("OrderItems"), "order_item");
    }

    #[test]
    fn test_to_attribute_name_simple() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = to_attribute_name("user_id", &strategy);
        assert_eq!(name, "user_id");
        assert!(!needs_alias);
    }

    #[test]
    fn test_to_attribute_name_camel_case() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = to_attribute_name("userId", &strategy);
        assert_eq!(name, "user_id");
        assert!(needs_alias);
    }

    #[test]
    fn test_to_attribute_name_reserved() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let (name, needs_alias) = to_attribute_name("from", &strategy);
        assert_eq!(name, "from_");
        assert!(needs_alias);
    }

    #[test]
    fn test_python_builtins() {
        assert!(is_python_builtin("list"));
        assert!(is_python_builtin("dict"));
        assert!(is_python_builtin("str"));
        assert!(is_python_builtin("int"));
        assert!(!is_python_builtin("user"));
    }
}
