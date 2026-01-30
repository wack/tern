//! Rust identifier naming and sanitization.
//!
//! This module handles conversion of PostgreSQL identifiers to valid Rust identifiers,
//! including handling of reserved words, invalid characters, and naming conventions.

use super::ReservedWordStrategy;

/// Rust reserved keywords that cannot be used as identifiers.
///
/// These include strict keywords and reserved keywords from all editions.
const RUST_KEYWORDS: &[&str] = &[
    // Strict keywords
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
    // Reserved keywords (may become keywords in future)
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try", "typeof",
    "unsized", "virtual", "yield", // 2018+ edition keywords
    "union",
];

/// Rust weak keywords that are reserved in certain contexts.
const RUST_WEAK_KEYWORDS: &[&str] = &[
    // These are only keywords in specific contexts
    "macro_rules",
    "raw",
    "safe", // safe is reserved for future use
];

/// Result of sanitizing an identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedName {
    /// The sanitized identifier safe for use in Rust code.
    pub identifier: String,
    /// Whether a `column_name` or `rename` attribute is needed because the
    /// identifier differs from the original database name.
    pub needs_rename_attr: bool,
    /// The original database name.
    pub original: String,
}

impl SanitizedName {
    /// Creates a new sanitized name where no renaming is needed.
    pub fn unchanged(name: &str) -> Self {
        Self {
            identifier: name.to_string(),
            needs_rename_attr: false,
            original: name.to_string(),
        }
    }

    /// Creates a new sanitized name with renaming.
    pub fn renamed(identifier: String, original: &str) -> Self {
        Self {
            identifier,
            needs_rename_attr: true,
            original: original.to_string(),
        }
    }
}

/// Checks if a name is a Rust keyword.
pub fn is_rust_keyword(name: &str) -> bool {
    RUST_KEYWORDS.contains(&name)
}

/// Checks if a name is a Rust weak keyword.
pub fn is_rust_weak_keyword(name: &str) -> bool {
    RUST_WEAK_KEYWORDS.contains(&name)
}

/// Checks if a name is any kind of Rust reserved word.
pub fn is_reserved_word(name: &str) -> bool {
    is_rust_keyword(name) || is_rust_weak_keyword(name)
}

/// Sanitizes a database identifier for use as a Rust identifier.
///
/// This function:
/// 1. Replaces invalid characters with underscores
/// 2. Ensures the name doesn't start with a digit
/// 3. Handles reserved words according to the strategy
/// 4. Returns the sanitized name with information about whether renaming is needed
pub fn sanitize_identifier(name: &str, strategy: &ReservedWordStrategy) -> SanitizedName {
    if name.is_empty() {
        return SanitizedName::renamed("_empty".to_string(), name);
    }

    let mut sanitized = String::with_capacity(name.len() + 1);
    let mut needs_rename = false;

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
                needs_rename = true;
            } else {
                // Replace invalid first character with underscore
                sanitized.push('_');
                needs_rename = true;
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            sanitized.push(c);
        } else {
            // Replace invalid characters with underscore
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

    // Remove trailing underscores (unless it's the only character or was in original)
    while sanitized.len() > 1 && sanitized.ends_with('_') && !name.ends_with('_') {
        sanitized.pop();
        needs_rename = true;
    }

    // Handle reserved words
    if is_reserved_word(&sanitized) {
        needs_rename = true;
        sanitized = apply_reserved_word_strategy(&sanitized, strategy);
    }

    // Final validation - ensure we have a valid identifier
    if sanitized.is_empty() || sanitized == "_" {
        return SanitizedName::renamed("_field".to_string(), name);
    }

    if needs_rename {
        SanitizedName::renamed(sanitized, name)
    } else {
        SanitizedName::unchanged(name)
    }
}

/// Applies the reserved word strategy to transform a reserved word.
fn apply_reserved_word_strategy(name: &str, strategy: &ReservedWordStrategy) -> String {
    match strategy {
        ReservedWordStrategy::AppendUnderscore => format!("{name}_"),
        ReservedWordStrategy::RawIdentifier => format!("r#{name}"),
        ReservedWordStrategy::PrependPrefix(prefix) => format!("{prefix}{name}"),
    }
}

/// Converts a table name to a Rust struct name (PascalCase, singular).
///
/// Examples:
/// - "users" -> "User"
/// - "user_accounts" -> "UserAccount"
/// - "order_items" -> "OrderItem"
/// - "categories" -> "Category"
pub fn to_struct_name(table_name: &str) -> String {
    // First singularize, then convert to PascalCase
    let singular = singularize(table_name);
    to_pascal_case(&singular)
}

/// Converts a table name to a Rust module name (snake_case, singular).
///
/// Examples:
/// - "users" -> "user"
/// - "user_accounts" -> "user_account"
/// - "OrderItems" -> "order_item"
pub fn to_module_name(table_name: &str) -> String {
    let snake = to_snake_case(table_name);
    singularize(&snake)
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

    if result.is_empty() {
        result = "Model".to_string();
    }

    result
}

/// Converts a string to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    let mut prev_was_upper = false;
    let mut prev_was_underscore = true;

    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 && !prev_was_upper && !prev_was_underscore {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_was_upper = true;
            prev_was_underscore = false;
        } else if c == '-' || c == ' ' {
            if !prev_was_underscore {
                result.push('_');
            }
            prev_was_upper = false;
            prev_was_underscore = true;
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

    result
}

/// Simple singularization of English words.
fn singularize(s: &str) -> String {
    // Handle "ies" -> "y" (e.g., "categories" -> "category")
    if s.ends_with("ies") && s.len() > 3 {
        let base = &s[..s.len() - 3];
        return format!("{base}y");
    }

    // Handle "es" -> "" for certain endings (e.g., "addresses" -> "address")
    if s.ends_with("sses") && s.len() > 4 {
        return s[..s.len() - 2].to_string();
    }
    if s.ends_with("xes") && s.len() > 3 {
        return s[..s.len() - 2].to_string();
    }
    if s.ends_with("ches") && s.len() > 4 {
        return s[..s.len() - 2].to_string();
    }
    if s.ends_with("shes") && s.len() > 4 {
        return s[..s.len() - 2].to_string();
    }

    // Handle simple "s" -> "" (e.g., "users" -> "user")
    if s.ends_with('s')
        && !s.ends_with("ss")
        && !s.ends_with("us")
        && !s.ends_with("is")
        && !s.ends_with("as")
        && s.len() > 1
    {
        return s[..s.len() - 1].to_string();
    }

    s.to_string()
}

/// Converts a column name to a Rust field name.
///
/// PostgreSQL column names are typically already in snake_case, but this
/// handles edge cases like mixed case or reserved words.
pub fn to_field_name(column_name: &str, strategy: &ReservedWordStrategy) -> SanitizedName {
    // First sanitize the identifier
    let sanitized = sanitize_identifier(column_name, strategy);

    // Convert to snake_case if not already
    let snake = to_snake_case(&sanitized.identifier);

    if snake != sanitized.identifier || sanitized.needs_rename_attr {
        SanitizedName::renamed(snake, column_name)
    } else {
        sanitized
    }
}

/// Converts a column name to a Rust enum variant name (PascalCase).
///
/// Examples:
/// - "user_id" -> "UserId"
/// - "created_at" -> "CreatedAt"
pub fn to_enum_variant(column_name: &str) -> String {
    to_pascal_case(column_name)
}

/// Converts a foreign key relation to a relation enum variant name.
///
/// Examples:
/// - FK from "posts" to "users" via "user_id" -> "User"
/// - FK from "comments" to "posts" via "post_id" -> "Post"
pub fn to_relation_name(target_table: &str) -> String {
    to_pascal_case(&singularize(target_table))
}

/// Converts a table name to a plural relation name for has_many.
///
/// Examples:
/// - "users" -> "Posts" (for has_many from users to posts)
/// - "categories" -> "Products" (for has_many)
pub fn to_plural_relation_name(target_table: &str) -> String {
    let pascal = to_pascal_case(target_table);
    // If already plural, return as-is; otherwise add 's'
    if target_table.ends_with('s') || target_table.ends_with("es") {
        pascal
    } else {
        format!("{pascal}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_rust_keyword() {
        assert!(is_rust_keyword("type"));
        assert!(is_rust_keyword("fn"));
        assert!(is_rust_keyword("struct"));
        assert!(is_rust_keyword("async"));
        assert!(is_rust_keyword("self"));
        assert!(is_rust_keyword("Self"));
        assert!(!is_rust_keyword("user"));
        assert!(!is_rust_keyword("name"));
    }

    #[test]
    fn test_is_reserved_word() {
        assert!(is_reserved_word("type"));
        assert!(is_reserved_word("abstract"));
        assert!(!is_reserved_word("user"));
    }

    #[test]
    fn test_sanitize_identifier_valid() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("user_id", &strategy);
        assert_eq!(result.identifier, "user_id");
        assert!(!result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_reserved_word() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("type", &strategy);
        assert_eq!(result.identifier, "type_");
        assert!(result.needs_rename_attr);
        assert_eq!(result.original, "type");
    }

    #[test]
    fn test_sanitize_identifier_raw_identifier() {
        let strategy = ReservedWordStrategy::RawIdentifier;
        let result = sanitize_identifier("type", &strategy);
        assert_eq!(result.identifier, "r#type");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_prefix() {
        let strategy = ReservedWordStrategy::PrependPrefix("field_".to_string());
        let result = sanitize_identifier("type", &strategy);
        assert_eq!(result.identifier, "field_type");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_starts_with_digit() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("1column", &strategy);
        assert_eq!(result.identifier, "_1column");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_special_characters() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("column-name", &strategy);
        assert_eq!(result.identifier, "column_name");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_multiple_underscores() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("column__name", &strategy);
        assert_eq!(result.identifier, "column_name");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_sanitize_identifier_empty() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = sanitize_identifier("", &strategy);
        assert_eq!(result.identifier, "_empty");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_to_struct_name_simple() {
        assert_eq!(to_struct_name("users"), "User");
        assert_eq!(to_struct_name("user"), "User");
    }

    #[test]
    fn test_to_struct_name_compound() {
        assert_eq!(to_struct_name("user_accounts"), "UserAccount");
        assert_eq!(to_struct_name("order_items"), "OrderItem");
    }

    #[test]
    fn test_to_struct_name_categories() {
        assert_eq!(to_struct_name("categories"), "Category");
    }

    #[test]
    fn test_to_struct_name_preserves_non_plural() {
        assert_eq!(to_struct_name("status"), "Status");
        assert_eq!(to_struct_name("address"), "Address");
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
    fn test_to_field_name_simple() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = to_field_name("user_id", &strategy);
        assert_eq!(result.identifier, "user_id");
        assert!(!result.needs_rename_attr);
    }

    #[test]
    fn test_to_field_name_reserved() {
        let strategy = ReservedWordStrategy::AppendUnderscore;
        let result = to_field_name("type", &strategy);
        assert_eq!(result.identifier, "type_");
        assert!(result.needs_rename_attr);
    }

    #[test]
    fn test_to_enum_variant() {
        assert_eq!(to_enum_variant("user_id"), "UserId");
        assert_eq!(to_enum_variant("created_at"), "CreatedAt");
        assert_eq!(to_enum_variant("id"), "Id");
    }

    #[test]
    fn test_to_relation_name() {
        assert_eq!(to_relation_name("users"), "User");
        assert_eq!(to_relation_name("user_accounts"), "UserAccount");
        assert_eq!(to_relation_name("categories"), "Category");
    }

    #[test]
    fn test_to_plural_relation_name() {
        assert_eq!(to_plural_relation_name("post"), "Posts");
        assert_eq!(to_plural_relation_name("posts"), "Posts");
        assert_eq!(to_plural_relation_name("user_account"), "UserAccounts");
    }

    #[test]
    fn test_singularize() {
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("addresses"), "address");
        assert_eq!(singularize("boxes"), "box");
        assert_eq!(singularize("status"), "status");
        assert_eq!(singularize("analysis"), "analysis");
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("user_account"), "UserAccount");
        assert_eq!(to_pascal_case("some-thing"), "SomeThing");
        assert_eq!(to_pascal_case("camelCase"), "Camelcase");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("UserAccount"), "user_account");
        assert_eq!(to_snake_case("someThing"), "some_thing");
        assert_eq!(to_snake_case("already_snake"), "already_snake");
    }
}
