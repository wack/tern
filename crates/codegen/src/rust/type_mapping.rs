//! PostgreSQL to Rust/SeaORM type mapping.
//!
//! This module handles the conversion of PostgreSQL types to their Rust equivalents
//! for use in SeaORM entity definitions.

use tern_ddl::TypeInfo;

/// Represents a Rust type with its SeaORM mapping and import requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustType {
    /// The Rust type annotation (e.g., "i32", "String", "chrono::DateTime<chrono::FixedOffset>").
    pub annotation: String,
    /// The SeaORM ColumnType expression (e.g., "ColumnType::Integer").
    pub column_type: String,
    /// Required SeaORM feature flags.
    pub required_features: Vec<&'static str>,
    /// Required imports (module path, type name).
    pub imports: Vec<RustImport>,
    /// Whether this type requires explicit column_type attribute in compact format.
    pub needs_column_type_attr: bool,
}

/// A Rust import statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RustImport {
    /// The full module path (e.g., "chrono", "serde_json").
    pub module: String,
    /// The type name to import (e.g., "DateTime", "Value").
    pub name: String,
}

impl RustImport {
    /// Creates a new Rust import.
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            name: name.into(),
        }
    }

    /// Creates a chrono import.
    pub fn chrono(name: &str) -> Self {
        Self::new("chrono", name)
    }

    /// Creates a serde_json import.
    pub fn serde_json(name: &str) -> Self {
        Self::new("serde_json", name)
    }

    /// Creates a uuid import.
    pub fn uuid() -> Self {
        Self::new("uuid", "Uuid")
    }

    /// Creates a rust_decimal import.
    pub fn rust_decimal() -> Self {
        Self::new("rust_decimal", "Decimal")
    }

    /// Creates an ipnetwork import.
    pub fn ipnetwork() -> Self {
        Self::new("ipnetwork", "IpNetwork")
    }

    /// Creates a mac_address import.
    pub fn mac_address() -> Self {
        Self::new("mac_address", "MacAddress")
    }
}

impl RustType {
    /// Creates a simple Rust type with no imports or special requirements.
    pub fn simple(annotation: &str, column_type: &str) -> Self {
        Self {
            annotation: annotation.to_string(),
            column_type: column_type.to_string(),
            required_features: Vec::new(),
            imports: Vec::new(),
            needs_column_type_attr: false,
        }
    }

    /// Creates a Rust type with imports.
    pub fn with_imports(
        annotation: &str,
        column_type: &str,
        imports: Vec<RustImport>,
        features: Vec<&'static str>,
    ) -> Self {
        Self {
            annotation: annotation.to_string(),
            column_type: column_type.to_string(),
            required_features: features,
            imports,
            needs_column_type_attr: false,
        }
    }

    /// Creates a Rust type that needs explicit column_type attribute.
    pub fn with_column_type_attr(
        annotation: &str,
        column_type: &str,
        imports: Vec<RustImport>,
        features: Vec<&'static str>,
    ) -> Self {
        Self {
            annotation: annotation.to_string(),
            column_type: column_type.to_string(),
            required_features: features,
            imports,
            needs_column_type_attr: true,
        }
    }

    /// Wraps this type in Option for nullable columns.
    pub fn as_optional(&self) -> Self {
        Self {
            annotation: format!("Option<{}>", self.annotation),
            column_type: self.column_type.clone(),
            required_features: self.required_features.clone(),
            imports: self.imports.clone(),
            needs_column_type_attr: self.needs_column_type_attr,
        }
    }
}

/// Maps a PostgreSQL type to a Rust type and SeaORM column type.
///
/// This function handles both the raw type name and the formatted type with modifiers.
/// The `formatted` field is used to extract precision/scale for numeric types.
pub fn map_pg_type(type_info: &TypeInfo) -> RustType {
    let type_name = type_info.name.as_ref();
    let formatted = &type_info.formatted;

    // Handle array types first
    if type_info.is_array {
        return map_array_type(type_name);
    }

    // Map based on type name (canonical PostgreSQL type names)
    match type_name {
        // Integer types
        "int2" | "smallint" => RustType::simple("i16", "ColumnType::SmallInteger"),
        "int4" | "integer" | "int" => RustType::simple("i32", "ColumnType::Integer"),
        "int8" | "bigint" => RustType::simple("i64", "ColumnType::BigInteger"),

        // Serial types (same Rust types as integers)
        "serial" | "serial4" => RustType::simple("i32", "ColumnType::Integer"),
        "bigserial" | "serial8" => RustType::simple("i64", "ColumnType::BigInteger"),
        "smallserial" | "serial2" => RustType::simple("i16", "ColumnType::SmallInteger"),

        // Floating point types
        "float4" | "real" => RustType::simple("f32", "ColumnType::Float"),
        "float8" | "double precision" => RustType::simple("f64", "ColumnType::Double"),

        // Numeric/decimal types
        "numeric" | "decimal" => {
            let column_type = extract_numeric_column_type(formatted);
            RustType::with_imports(
                "rust_decimal::Decimal",
                &column_type,
                vec![RustImport::rust_decimal()],
                vec!["with-rust_decimal"],
            )
        }

        // Money type
        "money" => {
            let column_type = "ColumnType::Money(Some((19, 2)))";
            RustType::with_imports(
                "rust_decimal::Decimal",
                column_type,
                vec![RustImport::rust_decimal()],
                vec!["with-rust_decimal"],
            )
        }

        // Boolean
        "bool" | "boolean" => RustType::simple("bool", "ColumnType::Boolean"),

        // Text types
        "text" => RustType::simple("String", "ColumnType::Text"),
        "varchar" | "character varying" => {
            let column_type = extract_varchar_column_type(formatted);
            RustType::simple("String", &column_type)
        }
        "char" | "character" | "bpchar" => {
            let column_type = extract_char_column_type(formatted);
            RustType::simple("String", &column_type)
        }
        "name" => RustType::simple("String", "ColumnType::String(StringLen::N(64))"),

        // Date/time types
        "date" => RustType::with_imports(
            "chrono::NaiveDate",
            "ColumnType::Date",
            vec![RustImport::chrono("NaiveDate")],
            vec!["with-chrono"],
        ),
        "time" | "time without time zone" => RustType::with_imports(
            "chrono::NaiveTime",
            "ColumnType::Time",
            vec![RustImport::chrono("NaiveTime")],
            vec!["with-chrono"],
        ),
        "timetz" | "time with time zone" => {
            // Time with timezone - SeaORM doesn't have great support, use NaiveTime
            RustType::with_imports(
                "chrono::NaiveTime",
                "ColumnType::Time",
                vec![RustImport::chrono("NaiveTime")],
                vec!["with-chrono"],
            )
        }
        "timestamp" | "timestamp without time zone" => RustType::with_imports(
            "chrono::NaiveDateTime",
            "ColumnType::DateTime",
            vec![RustImport::chrono("NaiveDateTime")],
            vec!["with-chrono"],
        ),
        "timestamptz" | "timestamp with time zone" => RustType::with_column_type_attr(
            "chrono::DateTime<chrono::FixedOffset>",
            "ColumnType::TimestampWithTimeZone",
            vec![
                RustImport::chrono("DateTime"),
                RustImport::chrono("FixedOffset"),
            ],
            vec!["with-chrono"],
        ),
        "interval" => {
            // Interval doesn't have a native Rust equivalent, use String
            RustType::with_column_type_attr(
                "String",
                "ColumnType::Interval(None, None)",
                vec![],
                vec![],
            )
        }

        // UUID
        "uuid" => RustType::with_imports(
            "uuid::Uuid",
            "ColumnType::Uuid",
            vec![RustImport::uuid()],
            vec!["with-uuid"],
        ),

        // JSON types
        "json" => RustType::with_column_type_attr(
            "serde_json::Value",
            "ColumnType::Json",
            vec![RustImport::serde_json("Value")],
            vec!["with-json"],
        ),
        "jsonb" => RustType::with_column_type_attr(
            "serde_json::Value",
            "ColumnType::JsonBinary",
            vec![RustImport::serde_json("Value")],
            vec!["with-json"],
        ),

        // Binary
        "bytea" => RustType::with_column_type_attr(
            "Vec<u8>",
            "ColumnType::Binary(BlobSize::Blob(None))",
            vec![],
            vec![],
        ),

        // Network types
        "inet" => RustType::with_imports(
            "ipnetwork::IpNetwork",
            "ColumnType::Inet",
            vec![RustImport::ipnetwork()],
            vec!["with-ipnetwork"],
        ),
        "cidr" => RustType::with_imports(
            "ipnetwork::IpNetwork",
            "ColumnType::Cidr",
            vec![RustImport::ipnetwork()],
            vec!["with-ipnetwork"],
        ),
        "macaddr" | "macaddr8" => RustType::with_imports(
            "mac_address::MacAddress",
            "ColumnType::MacAddr",
            vec![RustImport::mac_address()],
            vec!["with-mac_address"],
        ),

        // Geometric types (stored as strings)
        "point" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"point\".into())",
            vec![],
            vec![],
        ),
        "line" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"line\".into())",
            vec![],
            vec![],
        ),
        "lseg" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"lseg\".into())",
            vec![],
            vec![],
        ),
        "box" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"box\".into())",
            vec![],
            vec![],
        ),
        "path" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"path\".into())",
            vec![],
            vec![],
        ),
        "polygon" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"polygon\".into())",
            vec![],
            vec![],
        ),
        "circle" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"circle\".into())",
            vec![],
            vec![],
        ),

        // Bit strings
        "bit" | "varbit" | "bit varying" => {
            let column_type = extract_bit_column_type(formatted);
            RustType::with_column_type_attr("String", &column_type, vec![], vec![])
        }

        // XML
        "xml" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"xml\".into())",
            vec![],
            vec![],
        ),

        // Full-text search types
        "tsvector" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"tsvector\".into())",
            vec![],
            vec![],
        ),
        "tsquery" => RustType::with_column_type_attr(
            "String",
            "ColumnType::Custom(\"tsquery\".into())",
            vec![],
            vec![],
        ),

        // OID types (internal PostgreSQL types)
        "oid" | "regproc" | "regprocedure" | "regoper" | "regoperator" | "regclass" | "regtype"
        | "regrole" | "regnamespace" | "regconfig" | "regdictionary" => {
            RustType::simple("u32", "ColumnType::Unsigned")
        }

        // Range types (as string, complex to map)
        "int4range" | "int8range" | "numrange" | "tsrange" | "tstzrange" | "daterange" => {
            RustType::with_column_type_attr(
                "String",
                &format!("ColumnType::Custom(\"{type_name}\".into())"),
                vec![],
                vec![],
            )
        }

        // Default fallback - use String for unknown types
        _ => {
            // Check if it might be a user-defined enum
            // User-defined types typically don't have pg_catalog schema
            if type_info.schema.as_ref() != "pg_catalog" {
                // Likely a user-defined enum or composite type
                RustType::with_column_type_attr(
                    "String",
                    &format!("ColumnType::Custom(\"{type_name}\".into())"),
                    vec![],
                    vec![],
                )
            } else {
                // Unknown pg_catalog type, use String
                RustType::with_column_type_attr(
                    "String",
                    &format!("ColumnType::Custom(\"{type_name}\".into())"),
                    vec![],
                    vec![],
                )
            }
        }
    }
}

/// Maps a PostgreSQL array type to a Rust Vec type.
fn map_array_type(element_type_name: &str) -> RustType {
    // Get the element type first
    let element_type = match element_type_name {
        "int2" | "smallint" => ("i16", "ColumnType::SmallInteger"),
        "int4" | "integer" | "int" => ("i32", "ColumnType::Integer"),
        "int8" | "bigint" => ("i64", "ColumnType::BigInteger"),
        "float4" | "real" => ("f32", "ColumnType::Float"),
        "float8" | "double precision" => ("f64", "ColumnType::Double"),
        "bool" | "boolean" => ("bool", "ColumnType::Boolean"),
        "text" => ("String", "ColumnType::Text"),
        "varchar" | "character varying" => ("String", "ColumnType::String(StringLen::None)"),
        "uuid" => ("uuid::Uuid", "ColumnType::Uuid"),
        _ => ("String", "ColumnType::Text"),
    };

    let annotation = format!("Vec<{}>", element_type.0);
    let column_type = format!("ColumnType::Array(RcOrArc::new({}))", element_type.1);

    let mut imports = Vec::new();
    let mut features = Vec::new();

    if element_type.0 == "uuid::Uuid" {
        imports.push(RustImport::uuid());
        features.push("with-uuid");
    }

    RustType {
        annotation,
        column_type,
        required_features: features,
        imports,
        needs_column_type_attr: true,
    }
}

/// Extracts numeric column type with precision and scale from formatted type.
fn extract_numeric_column_type(formatted: &str) -> String {
    if let Some((precision, scale)) = extract_numeric_precision(formatted) {
        format!("ColumnType::Decimal(Some(({precision}, {scale})))")
    } else {
        "ColumnType::Decimal(None)".to_string()
    }
}

/// Extracts varchar column type with length from formatted type.
fn extract_varchar_column_type(formatted: &str) -> String {
    if let Some(length) = extract_string_length(formatted) {
        format!("ColumnType::String(StringLen::N({length}))")
    } else {
        "ColumnType::String(StringLen::None)".to_string()
    }
}

/// Extracts char column type with length from formatted type.
fn extract_char_column_type(formatted: &str) -> String {
    if let Some(length) = extract_string_length(formatted) {
        format!("ColumnType::Char(Some({length}))")
    } else {
        "ColumnType::Char(None)".to_string()
    }
}

/// Extracts bit column type with length from formatted type.
fn extract_bit_column_type(formatted: &str) -> String {
    if let Some(length) = extract_bit_length(formatted) {
        format!("ColumnType::Bit(Some({length}))")
    } else {
        "ColumnType::Bit(None)".to_string()
    }
}

/// Extracts the varchar/char length constraint from a formatted type.
///
/// Returns `Some(length)` for types like "character varying(255)" or "character(10)".
pub fn extract_string_length(formatted: &str) -> Option<u32> {
    let formatted_lower = formatted.to_lowercase();

    if formatted_lower.starts_with("character varying(")
        || formatted_lower.starts_with("varchar(")
        || formatted_lower.starts_with("character(")
        || formatted_lower.starts_with("char(")
    {
        extract_paren_number(formatted)
    } else {
        None
    }
}

/// Extracts the bit length from a formatted type.
fn extract_bit_length(formatted: &str) -> Option<u32> {
    let formatted_lower = formatted.to_lowercase();

    if formatted_lower.starts_with("bit(") || formatted_lower.starts_with("bit varying(") {
        extract_paren_number(formatted)
    } else {
        None
    }
}

/// Extracts numeric precision and scale from a formatted type.
///
/// Returns `Some((precision, scale))` for types like "numeric(10,2)".
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

/// Extracts a number from parentheses in a type string.
fn extract_paren_number(s: &str) -> Option<u32> {
    if let Some(start) = s.find('(') {
        if let Some(end) = s.find(')') {
            let num_str = &s[start + 1..end];
            return num_str.trim().parse().ok();
        }
    }
    None
}

/// Returns the SeaORM column type attribute value for use in `#[sea_orm(column_type = "...")]`.
///
/// This returns the short form used in derive macro attributes.
#[allow(dead_code)]
pub fn get_column_type_attr(type_info: &TypeInfo) -> Option<String> {
    let rust_type = map_pg_type(type_info);
    if rust_type.needs_column_type_attr {
        // Extract the short form from ColumnType::X
        let col_type = &rust_type.column_type;
        if col_type.starts_with("ColumnType::") {
            let short_form = &col_type["ColumnType::".len()..];
            // For simple types, just return the variant name
            if let Some(paren_pos) = short_form.find('(') {
                let variant = &short_form[..paren_pos];
                // Return just the variant name for the attribute
                Some(variant.to_string())
            } else {
                Some(short_form.to_string())
            }
        } else {
            None
        }
    } else {
        None
    }
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
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "i32");
        assert_eq!(rust_type.column_type, "ColumnType::Integer");
        assert!(rust_type.imports.is_empty());
        assert!(rust_type.required_features.is_empty());

        let type_info = make_type_info("int8", "bigint", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "i64");
        assert_eq!(rust_type.column_type, "ColumnType::BigInteger");

        let type_info = make_type_info("int2", "smallint", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "i16");
        assert_eq!(rust_type.column_type, "ColumnType::SmallInteger");
    }

    #[test]
    fn test_float_types() {
        let type_info = make_type_info("float8", "double precision", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "f64");
        assert_eq!(rust_type.column_type, "ColumnType::Double");

        let type_info = make_type_info("float4", "real", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "f32");
        assert_eq!(rust_type.column_type, "ColumnType::Float");
    }

    #[test]
    fn test_numeric_type() {
        let type_info = make_type_info("numeric", "numeric(10,2)", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "rust_decimal::Decimal");
        assert_eq!(rust_type.column_type, "ColumnType::Decimal(Some((10, 2)))");
        assert_eq!(rust_type.required_features, vec!["with-rust_decimal"]);
        assert_eq!(rust_type.imports.len(), 1);
        assert_eq!(rust_type.imports[0].module, "rust_decimal");
    }

    #[test]
    fn test_boolean_type() {
        let type_info = make_type_info("bool", "boolean", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "bool");
        assert_eq!(rust_type.column_type, "ColumnType::Boolean");
    }

    #[test]
    fn test_text_types() {
        let type_info = make_type_info("text", "text", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "String");
        assert_eq!(rust_type.column_type, "ColumnType::Text");

        let type_info = make_type_info("varchar", "character varying(255)", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "String");
        assert_eq!(
            rust_type.column_type,
            "ColumnType::String(StringLen::N(255))"
        );
    }

    #[test]
    fn test_datetime_types() {
        let type_info = make_type_info("timestamp", "timestamp without time zone", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "chrono::NaiveDateTime");
        assert_eq!(rust_type.column_type, "ColumnType::DateTime");
        assert_eq!(rust_type.required_features, vec!["with-chrono"]);

        let type_info = make_type_info("timestamptz", "timestamp with time zone", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(
            rust_type.annotation,
            "chrono::DateTime<chrono::FixedOffset>"
        );
        assert_eq!(rust_type.column_type, "ColumnType::TimestampWithTimeZone");
        assert!(rust_type.needs_column_type_attr);

        let type_info = make_type_info("date", "date", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "chrono::NaiveDate");
        assert_eq!(rust_type.column_type, "ColumnType::Date");
    }

    #[test]
    fn test_uuid_type() {
        let type_info = make_type_info("uuid", "uuid", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "uuid::Uuid");
        assert_eq!(rust_type.column_type, "ColumnType::Uuid");
        assert_eq!(rust_type.required_features, vec!["with-uuid"]);
    }

    #[test]
    fn test_json_types() {
        let type_info = make_type_info("jsonb", "jsonb", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "serde_json::Value");
        assert_eq!(rust_type.column_type, "ColumnType::JsonBinary");
        assert!(rust_type.needs_column_type_attr);
        assert_eq!(rust_type.required_features, vec!["with-json"]);

        let type_info = make_type_info("json", "json", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "serde_json::Value");
        assert_eq!(rust_type.column_type, "ColumnType::Json");
    }

    #[test]
    fn test_bytea_type() {
        let type_info = make_type_info("bytea", "bytea", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "Vec<u8>");
        assert!(rust_type.needs_column_type_attr);
    }

    #[test]
    fn test_array_type() {
        let type_info = make_type_info("int4", "integer[]", true);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "Vec<i32>");
        assert!(rust_type.column_type.contains("Array"));
        assert!(rust_type.needs_column_type_attr);
    }

    #[test]
    fn test_text_array_type() {
        let type_info = make_type_info("text", "text[]", true);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "Vec<String>");
        assert!(rust_type.column_type.contains("Array"));
    }

    #[test]
    fn test_network_types() {
        let type_info = make_type_info("inet", "inet", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "ipnetwork::IpNetwork");
        assert_eq!(rust_type.required_features, vec!["with-ipnetwork"]);

        let type_info = make_type_info("macaddr", "macaddr", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "mac_address::MacAddress");
        assert_eq!(rust_type.required_features, vec!["with-mac_address"]);
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
    fn test_optional_type() {
        let type_info = make_type_info("int4", "integer", false);
        let rust_type = map_pg_type(&type_info);
        let optional = rust_type.as_optional();
        assert_eq!(optional.annotation, "Option<i32>");
    }

    #[test]
    fn test_geometric_types() {
        let type_info = make_type_info("point", "point", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "String");
        assert!(rust_type.column_type.contains("Custom"));
    }

    #[test]
    fn test_interval_type() {
        let type_info = make_type_info("interval", "interval", false);
        let rust_type = map_pg_type(&type_info);
        assert_eq!(rust_type.annotation, "String");
        assert!(rust_type.column_type.contains("Interval"));
    }
}
