//! PostgreSQL to Python type mapping.
//!
//! This module handles the conversion of PostgreSQL types to their Python equivalents
//! for use in SQLModel field definitions.

use tern_ddl::TypeInfo;

/// Represents a Python type with its import requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonType {
    /// The Python type annotation (e.g., "int", "str", "datetime").
    pub annotation: String,
    /// Required imports for this type (module path, name).
    pub imports: Vec<PythonImport>,
    /// SQLAlchemy type for `sa_type` parameter if needed (e.g., for JSON, ARRAY).
    pub sa_type: Option<String>,
    /// SQLAlchemy imports required for sa_type.
    pub sa_imports: Vec<PythonImport>,
}

/// A Python import statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PythonImport {
    /// The module to import from (e.g., "datetime", "typing", "uuid").
    pub module: String,
    /// The name to import (e.g., "datetime", "Any", "UUID").
    pub name: String,
}

impl PythonImport {
    /// Creates a new Python import.
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            name: name.into(),
        }
    }

    /// Creates an import from the datetime module.
    pub fn datetime(name: &str) -> Self {
        Self::new("datetime", name)
    }

    /// Creates an import from the typing module.
    pub fn typing(name: &str) -> Self {
        Self::new("typing", name)
    }

    /// Creates an import from the decimal module.
    pub fn decimal() -> Self {
        Self::new("decimal", "Decimal")
    }

    /// Creates an import from the uuid module.
    pub fn uuid() -> Self {
        Self::new("uuid", "UUID")
    }

    /// Creates an import from SQLAlchemy.
    pub fn sqlalchemy(name: &str) -> Self {
        Self::new("sqlalchemy", name)
    }
}

impl PythonType {
    /// Creates a simple Python type with no imports.
    pub fn simple(annotation: impl Into<String>) -> Self {
        Self {
            annotation: annotation.into(),
            imports: Vec::new(),
            sa_type: None,
            sa_imports: Vec::new(),
        }
    }

    /// Creates a Python type with imports.
    pub fn with_imports(annotation: impl Into<String>, imports: Vec<PythonImport>) -> Self {
        Self {
            annotation: annotation.into(),
            imports,
            sa_type: None,
            sa_imports: Vec::new(),
        }
    }

    /// Creates a Python type with an SQLAlchemy type.
    pub fn with_sa_type(
        annotation: impl Into<String>,
        imports: Vec<PythonImport>,
        sa_type: impl Into<String>,
        sa_imports: Vec<PythonImport>,
    ) -> Self {
        Self {
            annotation: annotation.into(),
            imports,
            sa_type: Some(sa_type.into()),
            sa_imports,
        }
    }
}

/// Maps a PostgreSQL type to a Python type.
///
/// This function handles both the raw type name and the formatted type with modifiers.
/// The `formatted` field is preferred as it contains the full type specification.
pub fn map_pg_type(type_info: &TypeInfo) -> PythonType {
    let type_name = type_info.name.as_ref();
    let formatted = &type_info.formatted;

    // Handle array types first
    if type_info.is_array {
        return map_array_type(type_name, formatted);
    }

    // Map based on type name (canonical PostgreSQL type names)
    match type_name {
        // Integer types
        "int2" | "smallint" => PythonType::simple("int"),
        "int4" | "integer" | "int" => PythonType::simple("int"),
        "int8" | "bigint" => PythonType::simple("int"),
        "serial" | "serial4" => PythonType::simple("int"),
        "bigserial" | "serial8" => PythonType::simple("int"),
        "smallserial" | "serial2" => PythonType::simple("int"),

        // Floating point types
        "float4" | "real" => PythonType::simple("float"),
        "float8" | "double precision" => PythonType::simple("float"),

        // Numeric/decimal types
        "numeric" | "decimal" => PythonType::with_imports("Decimal", vec![PythonImport::decimal()]),

        // Boolean
        "bool" | "boolean" => PythonType::simple("bool"),

        // Text types
        "text" => PythonType::simple("str"),
        "varchar" | "character varying" => PythonType::simple("str"),
        "char" | "character" | "bpchar" => PythonType::simple("str"),
        "name" => PythonType::simple("str"),

        // Date/time types
        "date" => PythonType::with_imports("date", vec![PythonImport::datetime("date")]),
        "time" | "time without time zone" => {
            PythonType::with_imports("time", vec![PythonImport::datetime("time")])
        }
        "timetz" | "time with time zone" => {
            PythonType::with_imports("time", vec![PythonImport::datetime("time")])
        }
        "timestamp" | "timestamp without time zone" => {
            PythonType::with_imports("datetime", vec![PythonImport::datetime("datetime")])
        }
        "timestamptz" | "timestamp with time zone" => {
            PythonType::with_imports("datetime", vec![PythonImport::datetime("datetime")])
        }
        "interval" => {
            PythonType::with_imports("timedelta", vec![PythonImport::datetime("timedelta")])
        }

        // UUID
        "uuid" => PythonType::with_imports("UUID", vec![PythonImport::uuid()]),

        // JSON types
        "json" | "jsonb" => PythonType::with_sa_type(
            "dict[str, Any]",
            vec![PythonImport::typing("Any")],
            "JSON",
            vec![PythonImport::sqlalchemy("JSON")],
        ),

        // Binary
        "bytea" => PythonType::simple("bytes"),

        // Network types
        "inet" | "cidr" | "macaddr" | "macaddr8" => PythonType::simple("str"),

        // Geometric types (stored as strings)
        "point" | "line" | "lseg" | "box" | "path" | "polygon" | "circle" => {
            PythonType::simple("str")
        }

        // Bit strings
        "bit" | "varbit" | "bit varying" => PythonType::simple("str"),

        // Money (stored as string to preserve formatting)
        "money" => PythonType::simple("str"),

        // XML
        "xml" => PythonType::simple("str"),

        // OID types (internal PostgreSQL types)
        "oid" | "regproc" | "regprocedure" | "regoper" | "regoperator" | "regclass" | "regtype"
        | "regrole" | "regnamespace" | "regconfig" | "regdictionary" => PythonType::simple("int"),

        // Range types
        "int4range" | "int8range" | "numrange" | "tsrange" | "tstzrange" | "daterange" => {
            // Range types are complex; represent as string for now
            PythonType::simple("str")
        }

        // TSVector/TSQuery (full text search)
        "tsvector" | "tsquery" => PythonType::simple("str"),

        // Default fallback - use Any for unknown types
        _ => {
            // Check if it looks like a user-defined type (enum, composite, etc.)
            // For now, map unknown types to Any
            PythonType::with_imports("Any", vec![PythonImport::typing("Any")])
        }
    }
}

/// Maps a PostgreSQL array type to a Python list type.
fn map_array_type(element_type_name: &str, formatted: &str) -> PythonType {
    // Get the element type first
    let element_type = map_pg_type(&TypeInfo {
        name: tern_ddl::TypeName::try_new(element_type_name.to_string())
            .unwrap_or_else(|_| tern_ddl::TypeName::try_new("text".to_string()).unwrap()),
        schema: tern_ddl::SchemaName::try_new("pg_catalog".to_string()).unwrap(),
        formatted: strip_array_suffix(formatted),
        is_array: false,
    });

    // Determine the SQLAlchemy array element type
    let sa_element_type = match element_type_name {
        "int2" | "smallint" => "SmallInteger",
        "int4" | "integer" | "int" => "Integer",
        "int8" | "bigint" => "BigInteger",
        "float4" | "real" => "Float",
        "float8" | "double precision" => "Float",
        "numeric" | "decimal" => "Numeric",
        "bool" | "boolean" => "Boolean",
        "text" => "Text",
        "varchar" | "character varying" => "String",
        "char" | "character" | "bpchar" => "String",
        "uuid" => "UUID",
        "timestamp" | "timestamp without time zone" => "DateTime",
        "timestamptz" | "timestamp with time zone" => "DateTime",
        "date" => "Date",
        "time" | "time without time zone" => "Time",
        "json" | "jsonb" => "JSON",
        _ => "String", // Default to String for unknown types
    };

    let annotation = format!("list[{}]", element_type.annotation);
    let sa_type = format!("ARRAY({sa_element_type})");

    let mut imports = element_type.imports;
    let mut sa_imports = vec![PythonImport::sqlalchemy("ARRAY")];

    // Add the element type import for SQLAlchemy
    // All ARRAY element types require their corresponding SQLAlchemy type import
    sa_imports.push(PythonImport::sqlalchemy(sa_element_type));

    // Merge any existing SA imports from element type
    imports.extend(element_type.sa_imports);

    PythonType {
        annotation,
        imports,
        sa_type: Some(sa_type),
        sa_imports,
    }
}

/// Strips the array suffix (e.g., "[]") from a formatted type string.
fn strip_array_suffix(formatted: &str) -> String {
    formatted
        .trim_end_matches("[]")
        .trim_end_matches(" ARRAY")
        .to_string()
}

/// Extracts the varchar/char length constraint from a formatted type.
///
/// Returns `Some(length)` for types like "character varying(255)" or "character(10)".
pub fn extract_string_length(formatted: &str) -> Option<u32> {
    // Match patterns like "character varying(255)" or "character(10)" or "varchar(100)"
    let formatted_lower = formatted.to_lowercase();

    if formatted_lower.starts_with("character varying(")
        || formatted_lower.starts_with("varchar(")
        || formatted_lower.starts_with("character(")
        || formatted_lower.starts_with("char(")
    {
        if let Some(start) = formatted.find('(') {
            if let Some(end) = formatted.find(')') {
                let len_str = &formatted[start + 1..end];
                return len_str.parse().ok();
            }
        }
    }

    None
}

/// Extracts numeric precision and scale from a formatted type.
///
/// Returns `Some((precision, scale))` for types like "numeric(10,2)".
/// Kept for future use to add Decimal precision validation in Field().
#[allow(dead_code)]
pub fn extract_numeric_precision(formatted: &str) -> Option<(u32, u32)> {
    let formatted_lower = formatted.to_lowercase();

    if formatted_lower.starts_with("numeric(") || formatted_lower.starts_with("decimal(") {
        if let Some(start) = formatted.find('(') {
            if let Some(end) = formatted.find(')') {
                let params = &formatted[start + 1..end];
                let parts: Vec<&str> = params.split(',').collect();
                if parts.len() == 2 {
                    let precision: u32 = parts[0].trim().parse().ok()?;
                    let scale: u32 = parts[1].trim().parse().ok()?;
                    return Some((precision, scale));
                } else if parts.len() == 1 {
                    let precision: u32 = parts[0].trim().parse().ok()?;
                    return Some((precision, 0));
                }
            }
        }
    }

    None
}

/// Checks if a type is a fixed-length character type (char/character).
pub fn is_fixed_length_char(type_name: &str) -> bool {
    matches!(type_name, "char" | "character" | "bpchar")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::{SchemaName, TypeName};

    fn make_type_info(name: &str, formatted: &str, is_array: bool) -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new(name.to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: formatted.to_string(),
            is_array,
        }
    }

    #[test]
    fn test_integer_types() {
        let type_info = make_type_info("int4", "integer", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "int");
        assert!(py_type.imports.is_empty());

        let type_info = make_type_info("int8", "bigint", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "int");

        let type_info = make_type_info("int2", "smallint", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "int");
    }

    #[test]
    fn test_float_types() {
        let type_info = make_type_info("float8", "double precision", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "float");

        let type_info = make_type_info("float4", "real", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "float");
    }

    #[test]
    fn test_numeric_type() {
        let type_info = make_type_info("numeric", "numeric(10,2)", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "Decimal");
        assert_eq!(py_type.imports.len(), 1);
        assert_eq!(py_type.imports[0].module, "decimal");
        assert_eq!(py_type.imports[0].name, "Decimal");
    }

    #[test]
    fn test_boolean_type() {
        let type_info = make_type_info("bool", "boolean", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "bool");
    }

    #[test]
    fn test_text_types() {
        let type_info = make_type_info("text", "text", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "str");

        let type_info = make_type_info("varchar", "character varying(255)", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "str");
    }

    #[test]
    fn test_datetime_types() {
        let type_info = make_type_info("timestamp", "timestamp without time zone", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "datetime");
        assert_eq!(py_type.imports.len(), 1);
        assert_eq!(py_type.imports[0].module, "datetime");
        assert_eq!(py_type.imports[0].name, "datetime");

        let type_info = make_type_info("timestamptz", "timestamp with time zone", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "datetime");

        let type_info = make_type_info("date", "date", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "date");
        assert_eq!(py_type.imports[0].name, "date");

        let type_info = make_type_info("time", "time without time zone", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "time");
    }

    #[test]
    fn test_uuid_type() {
        let type_info = make_type_info("uuid", "uuid", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "UUID");
        assert_eq!(py_type.imports.len(), 1);
        assert_eq!(py_type.imports[0].module, "uuid");
        assert_eq!(py_type.imports[0].name, "UUID");
    }

    #[test]
    fn test_json_types() {
        let type_info = make_type_info("jsonb", "jsonb", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "dict[str, Any]");
        assert!(py_type.imports.iter().any(|i| i.name == "Any"));
        assert_eq!(py_type.sa_type, Some("JSON".to_string()));
    }

    #[test]
    fn test_bytea_type() {
        let type_info = make_type_info("bytea", "bytea", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "bytes");
    }

    #[test]
    fn test_array_type() {
        let type_info = make_type_info("int4", "integer[]", true);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "list[int]");
        assert!(py_type.sa_type.is_some());
        assert!(py_type.sa_type.as_ref().unwrap().contains("ARRAY"));
    }

    #[test]
    fn test_text_array_type() {
        let type_info = make_type_info("text", "text[]", true);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "list[str]");
        // Text arrays use ARRAY(Text) - Text is the correct SQLAlchemy type for pg text
        assert!(py_type.sa_type.as_ref().unwrap().contains("ARRAY(Text)"));
    }

    #[test]
    fn test_extract_string_length() {
        assert_eq!(extract_string_length("character varying(255)"), Some(255));
        assert_eq!(extract_string_length("varchar(100)"), Some(100));
        assert_eq!(extract_string_length("character(10)"), Some(10));
        assert_eq!(extract_string_length("char(5)"), Some(5));
        assert_eq!(extract_string_length("text"), None);
    }

    #[test]
    fn test_extract_numeric_precision() {
        assert_eq!(extract_numeric_precision("numeric(10,2)"), Some((10, 2)));
        assert_eq!(extract_numeric_precision("numeric(5)"), Some((5, 0)));
        assert_eq!(extract_numeric_precision("decimal(8,3)"), Some((8, 3)));
        assert_eq!(extract_numeric_precision("integer"), None);
    }

    #[test]
    fn test_is_fixed_length_char() {
        assert!(is_fixed_length_char("char"));
        assert!(is_fixed_length_char("character"));
        assert!(is_fixed_length_char("bpchar"));
        assert!(!is_fixed_length_char("varchar"));
        assert!(!is_fixed_length_char("text"));
    }

    #[test]
    fn test_network_types() {
        let type_info = make_type_info("inet", "inet", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "str");

        let type_info = make_type_info("cidr", "cidr", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "str");
    }

    #[test]
    fn test_interval_type() {
        let type_info = make_type_info("interval", "interval", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "timedelta");
        assert_eq!(py_type.imports[0].module, "datetime");
        assert_eq!(py_type.imports[0].name, "timedelta");
    }

    #[test]
    fn test_unknown_type_fallback() {
        let type_info = make_type_info("my_custom_type", "my_custom_type", false);
        let py_type = map_pg_type(&type_info);
        assert_eq!(py_type.annotation, "Any");
        assert!(py_type.imports.iter().any(|i| i.name == "Any"));
    }
}
