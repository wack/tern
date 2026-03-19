//! PostgreSQL to Drizzle ORM type mapping.
//!
//! This module handles the conversion of PostgreSQL types to their Drizzle ORM
//! column builder equivalents for use in `drizzle-orm/pg-core` schema definitions.

use tern_ddl::TypeInfo;

/// Represents a Drizzle column type with its builder function and import requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrizzleColumnType {
    /// The builder function name (e.g., "serial", "varchar", "timestamp").
    pub builder: String,
    /// Additional config object for the builder (e.g., `{ length: 255 }`).
    pub config: Option<String>,
    /// Required import names from "drizzle-orm/pg-core".
    pub imports: Vec<String>,
    /// Warning if this type mapping is approximate.
    pub warning: Option<String>,
}

impl DrizzleColumnType {
    /// Creates a simple column type with no config.
    fn simple(builder: &str) -> Self {
        Self {
            builder: builder.to_string(),
            config: None,
            imports: vec![builder.to_string()],
            warning: None,
        }
    }

    /// Creates a column type with a config object.
    fn with_config(builder: &str, config: &str) -> Self {
        Self {
            builder: builder.to_string(),
            config: Some(config.to_string()),
            imports: vec![builder.to_string()],
            warning: None,
        }
    }

    /// Creates a column type with a warning.
    fn with_warning(builder: &str, warning: &str) -> Self {
        Self {
            builder: builder.to_string(),
            config: None,
            imports: vec![builder.to_string()],
            warning: Some(warning.to_string()),
        }
    }
}

/// Maps a PostgreSQL type to a Drizzle column type.
pub fn map_pg_type(type_info: &TypeInfo) -> DrizzleColumnType {
    let type_name = type_info.name.as_ref();
    let formatted = &type_info.formatted;

    // Handle array types
    if type_info.is_array {
        return map_array_type(type_name);
    }

    match type_name {
        // Integer types
        "int2" | "smallint" => DrizzleColumnType::simple("smallint"),
        "int4" | "integer" | "int" => DrizzleColumnType::simple("integer"),
        "int8" | "bigint" => DrizzleColumnType::simple("bigint"),

        // Serial types
        "serial" | "serial4" => DrizzleColumnType::simple("serial"),
        "bigserial" | "serial8" => DrizzleColumnType::simple("bigserial"),
        "smallserial" | "serial2" => DrizzleColumnType::simple("smallserial"),

        // Floating point types
        "float4" | "real" => DrizzleColumnType::simple("real"),
        "float8" | "double precision" => DrizzleColumnType::simple("doublePrecision"),

        // Numeric/decimal types
        "numeric" | "decimal" => {
            if let Some(config) = extract_numeric_config(formatted) {
                DrizzleColumnType::with_config("numeric", &config)
            } else {
                DrizzleColumnType::simple("numeric")
            }
        }

        // Boolean
        "bool" | "boolean" => DrizzleColumnType::simple("boolean"),

        // Text types
        "text" => DrizzleColumnType::simple("text"),
        "varchar" | "character varying" => {
            if let Some(length) = extract_string_length(formatted) {
                DrizzleColumnType::with_config("varchar", &format!("{{ length: {length} }}"))
            } else {
                DrizzleColumnType::simple("varchar")
            }
        }
        "char" | "character" | "bpchar" => {
            if let Some(length) = extract_string_length(formatted) {
                DrizzleColumnType::with_config("char", &format!("{{ length: {length} }}"))
            } else {
                DrizzleColumnType::simple("char")
            }
        }
        "name" => DrizzleColumnType::with_config("varchar", "{ length: 64 }"),

        // Date/time types
        "date" => DrizzleColumnType::simple("date"),
        "time" | "time without time zone" => DrizzleColumnType::simple("time"),
        "timetz" | "time with time zone" => {
            DrizzleColumnType::with_config("time", "{ withTimezone: true }")
        }
        "timestamp" | "timestamp without time zone" => DrizzleColumnType::simple("timestamp"),
        "timestamptz" | "timestamp with time zone" => {
            DrizzleColumnType::with_config("timestamp", "{ withTimezone: true }")
        }
        "interval" => DrizzleColumnType::simple("interval"),

        // UUID
        "uuid" => DrizzleColumnType::simple("uuid"),

        // JSON types
        "json" => DrizzleColumnType::simple("json"),
        "jsonb" => DrizzleColumnType::simple("jsonb"),

        // Binary
        "bytea" => DrizzleColumnType::with_warning(
            "text",
            "bytea type has no direct Drizzle equivalent; using text",
        ),

        // Network types
        "inet" => DrizzleColumnType::simple("inet"),
        "cidr" => DrizzleColumnType::with_warning(
            "text",
            "cidr type has no direct Drizzle equivalent; using text",
        ),
        "macaddr" | "macaddr8" => DrizzleColumnType::simple("macaddr"),

        // Geometric types
        "point" | "line" | "lseg" | "box" | "path" | "polygon" | "circle" => {
            DrizzleColumnType::with_warning(
                "text",
                &format!("{type_name} type has no direct Drizzle equivalent; using text"),
            )
        }

        // Bit strings
        "bit" | "varbit" | "bit varying" => DrizzleColumnType::with_warning(
            "text",
            "bit string type has no direct Drizzle equivalent; using text",
        ),

        // Money
        "money" => DrizzleColumnType::with_warning(
            "text",
            "money type has no direct Drizzle equivalent; using text",
        ),

        // XML
        "xml" => DrizzleColumnType::with_warning(
            "text",
            "xml type has no direct Drizzle equivalent; using text",
        ),

        // OID types
        "oid" | "regproc" | "regprocedure" | "regoper" | "regoperator" | "regclass" | "regtype"
        | "regrole" | "regnamespace" | "regconfig" | "regdictionary" => {
            DrizzleColumnType::simple("integer")
        }

        // Range types
        "int4range" | "int8range" | "numrange" | "tsrange" | "tstzrange" | "daterange" => {
            DrizzleColumnType::with_warning(
                "text",
                &format!("{type_name} range type has no direct Drizzle equivalent; using text"),
            )
        }

        // Full-text search
        "tsvector" | "tsquery" => DrizzleColumnType::with_warning(
            "text",
            &format!("{type_name} type has no direct Drizzle equivalent; using text"),
        ),

        // Default fallback
        _ => DrizzleColumnType::with_warning(
            "text",
            &format!("unknown PostgreSQL type '{type_name}'; using text"),
        ),
    }
}

/// Maps a PostgreSQL array type to a Drizzle column type.
fn map_array_type(element_type_name: &str) -> DrizzleColumnType {
    DrizzleColumnType::with_warning(
        "text",
        &format!("{element_type_name}[] array type has no direct Drizzle equivalent; using text"),
    )
}

/// Extracts varchar/char length from a formatted type string.
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

/// Extracts numeric precision and scale config from a formatted type.
fn extract_numeric_config(formatted: &str) -> Option<String> {
    let formatted_lower = formatted.to_lowercase();

    if formatted_lower.starts_with("numeric(") || formatted_lower.starts_with("decimal(") {
        if let Some(start) = formatted.find('(') {
            if let Some(end) = formatted.find(')') {
                let params = &formatted[start + 1..end];
                let parts: Vec<&str> = params.split(',').collect();
                if parts.len() == 2 {
                    let precision: u32 = parts[0].trim().parse().ok()?;
                    let scale: u32 = parts[1].trim().parse().ok()?;
                    return Some(format!("{{ precision: {precision}, scale: {scale} }}"));
                } else if parts.len() == 1 {
                    let precision: u32 = parts[0].trim().parse().ok()?;
                    return Some(format!("{{ precision: {precision} }}"));
                }
            }
        }
    }

    None
}

/// Extracts a number from parentheses in a type string.
fn extract_paren_number(s: &str) -> Option<u32> {
    let start = s.find('(')?;
    let end = s.find(')')?;
    s[start + 1..end].trim().parse().ok()
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
        let ti = make_type_info("int4", "integer", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "integer");
        assert!(dt.config.is_none());
        assert_eq!(dt.imports, vec!["integer"]);

        let ti = make_type_info("int8", "bigint", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "bigint");

        let ti = make_type_info("int2", "smallint", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "smallint");
    }

    #[test]
    fn test_serial_types() {
        let ti = make_type_info("serial", "serial", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "serial");

        let ti = make_type_info("bigserial", "bigserial", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "bigserial");

        let ti = make_type_info("smallserial", "smallserial", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "smallserial");
    }

    #[test]
    fn test_float_types() {
        let ti = make_type_info("float4", "real", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "real");

        let ti = make_type_info("float8", "double precision", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "doublePrecision");
    }

    #[test]
    fn test_numeric_types() {
        let ti = make_type_info("numeric", "numeric(10,2)", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "numeric");
        assert_eq!(dt.config, Some("{ precision: 10, scale: 2 }".to_string()));

        let ti = make_type_info("numeric", "numeric", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "numeric");
        assert!(dt.config.is_none());
    }

    #[test]
    fn test_boolean_type() {
        let ti = make_type_info("bool", "boolean", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "boolean");
    }

    #[test]
    fn test_text_types() {
        let ti = make_type_info("text", "text", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "text");

        let ti = make_type_info("varchar", "character varying(255)", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "varchar");
        assert_eq!(dt.config, Some("{ length: 255 }".to_string()));

        let ti = make_type_info("varchar", "character varying", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "varchar");
        assert!(dt.config.is_none());
    }

    #[test]
    fn test_datetime_types() {
        let ti = make_type_info("timestamp", "timestamp without time zone", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "timestamp");
        assert!(dt.config.is_none());

        let ti = make_type_info("timestamptz", "timestamp with time zone", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "timestamp");
        assert_eq!(dt.config, Some("{ withTimezone: true }".to_string()));

        let ti = make_type_info("date", "date", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "date");

        let ti = make_type_info("time", "time without time zone", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "time");
        assert!(dt.config.is_none());

        let ti = make_type_info("timetz", "time with time zone", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "time");
        assert_eq!(dt.config, Some("{ withTimezone: true }".to_string()));
    }

    #[test]
    fn test_uuid_type() {
        let ti = make_type_info("uuid", "uuid", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "uuid");
    }

    #[test]
    fn test_json_types() {
        let ti = make_type_info("json", "json", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "json");

        let ti = make_type_info("jsonb", "jsonb", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "jsonb");
    }

    #[test]
    fn test_interval_type() {
        let ti = make_type_info("interval", "interval", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "interval");
    }

    #[test]
    fn test_inet_type() {
        let ti = make_type_info("inet", "inet", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "inet");
    }

    #[test]
    fn test_bytea_fallback() {
        let ti = make_type_info("bytea", "bytea", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "text");
        assert!(dt.warning.is_some());
    }

    #[test]
    fn test_array_fallback() {
        let ti = make_type_info("int4", "integer[]", true);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "text");
        assert!(dt.warning.is_some());
        assert!(dt.warning.unwrap().contains("array"));
    }

    #[test]
    fn test_unknown_type_fallback() {
        let ti = make_type_info("my_custom_type", "my_custom_type", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "text");
        assert!(dt.warning.is_some());
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
    fn test_extract_numeric_config() {
        assert_eq!(
            extract_numeric_config("numeric(10,2)"),
            Some("{ precision: 10, scale: 2 }".to_string())
        );
        assert_eq!(
            extract_numeric_config("numeric(5)"),
            Some("{ precision: 5 }".to_string())
        );
        assert_eq!(extract_numeric_config("numeric"), None);
    }

    #[test]
    fn test_char_with_length() {
        let ti = make_type_info("char", "character(10)", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "char");
        assert_eq!(dt.config, Some("{ length: 10 }".to_string()));
    }

    #[test]
    fn test_geometric_types_fallback() {
        for type_name in &["point", "line", "lseg", "box", "path", "polygon", "circle"] {
            let ti = make_type_info(type_name, type_name, false);
            let dt = map_pg_type(&ti);
            assert_eq!(dt.builder, "text");
            assert!(dt.warning.is_some());
        }
    }

    #[test]
    fn test_macaddr_type() {
        let ti = make_type_info("macaddr", "macaddr", false);
        let dt = map_pg_type(&ti);
        assert_eq!(dt.builder, "macaddr");
        assert!(dt.warning.is_none());
    }
}
