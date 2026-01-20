//! SQL queries for PostgreSQL catalog introspection.
//!
//! These queries extract schema metadata from PostgreSQL's system catalogs.
//! All queries are designed to work with PostgreSQL 12+.

/// Query to fetch a namespace (schema) by name.
///
/// Parameters: $1 = schema name
/// Returns: oid, nspname
pub const NAMESPACE_BY_NAME: &str = r#"
SELECT
    n.oid,
    n.nspname,
    obj_description(n.oid, 'pg_namespace') AS comment
FROM pg_namespace n
WHERE n.nspname = $1
"#;

/// Query to fetch all tables in a namespace.
///
/// Parameters: $1 = namespace OID
/// Returns: table metadata including OID, name, kind, and comment
pub const TABLES_IN_NAMESPACE: &str = r#"
SELECT
    c.oid,
    c.relname,
    c.relkind,
    obj_description(c.oid, 'pg_class') AS comment
FROM pg_class c
WHERE c.relnamespace = $1
  AND c.relkind IN ('r', 'p')  -- regular and partitioned tables
ORDER BY c.relname
"#;

/// Query to fetch all columns for a table.
///
/// Parameters: $1 = table OID
/// Returns: column metadata including position, name, type, nullability, defaults, etc.
pub const COLUMNS_FOR_TABLE: &str = r#"
SELECT
    a.attnum AS position,
    a.attname AS name,
    t.typname AS type_name,
    tn.nspname AS type_schema,
    format_type(a.atttypid, a.atttypmod) AS formatted_type,
    t.typtype = 'b' AND t.typelem != 0 AS is_array,
    NOT a.attnotnull AS is_nullable,
    pg_get_expr(d.adbin, d.adrelid) AS default_expr,
    a.attgenerated AS generated_kind,
    pg_get_expr(d.adbin, d.adrelid) AS generated_expr,
    a.attidentity AS identity_kind,
    COALESCE(cn.nspname, 'pg_catalog') AS collation_schema,
    COALESCE(co.collname, 'default') AS collation_name,
    col_description(a.attrelid, a.attnum) AS comment
FROM pg_attribute a
JOIN pg_type t ON a.atttypid = t.oid
JOIN pg_namespace tn ON t.typnamespace = tn.oid
LEFT JOIN pg_attrdef d ON a.attrelid = d.adrelid AND a.attnum = d.adnum
LEFT JOIN pg_collation co ON a.attcollation = co.oid AND a.attcollation != 0
LEFT JOIN pg_namespace cn ON co.collnamespace = cn.oid
WHERE a.attrelid = $1
  AND a.attnum > 0           -- exclude system columns
  AND NOT a.attisdropped     -- exclude dropped columns
ORDER BY a.attnum
"#;

/// Query to fetch all constraints for a table.
///
/// Parameters: $1 = table OID
/// Returns: constraint metadata
pub const CONSTRAINTS_FOR_TABLE: &str = r#"
SELECT
    con.oid,
    con.conname,
    con.contype,
    con.conkey,                              -- array of column numbers for this table
    con.confkey,                             -- array of column numbers for referenced table (FK only)
    con.confrelid,                           -- referenced table OID (FK only)
    ref_ns.nspname AS ref_schema,            -- referenced table schema (FK only)
    ref_class.relname AS ref_table,          -- referenced table name (FK only)
    con.confdeltype,                         -- ON DELETE action (FK only)
    con.confupdtype,                         -- ON UPDATE action (FK only)
    con.condeferrable,                       -- is constraint deferrable?
    con.condeferred,                         -- is constraint initially deferred?
    pg_get_expr(con.conbin, con.conrelid) AS check_expr,  -- CHECK expression
    con.connoinherit,                        -- is NO INHERIT?
    i.relname AS index_name,                 -- backing index name (PK/UNIQUE/EXCLUSION)
    am.amname AS index_method,               -- index method (for EXCLUSION)
    pg_get_expr(idx.indpred, idx.indrelid) AS index_predicate,  -- partial index predicate
    obj_description(con.oid, 'pg_constraint') AS comment
FROM pg_constraint con
LEFT JOIN pg_class ref_class ON con.confrelid = ref_class.oid
LEFT JOIN pg_namespace ref_ns ON ref_class.relnamespace = ref_ns.oid
LEFT JOIN pg_class i ON con.conindid = i.oid
LEFT JOIN pg_index idx ON i.oid = idx.indexrelid
LEFT JOIN pg_am am ON i.relam = am.oid
WHERE con.conrelid = $1
ORDER BY
    CASE con.contype
        WHEN 'p' THEN 1  -- primary key first
        WHEN 'u' THEN 2  -- then unique
        WHEN 'f' THEN 3  -- then foreign key
        WHEN 'c' THEN 4  -- then check
        WHEN 'x' THEN 5  -- then exclusion
    END,
    con.conname
"#;

/// Query to get column names by their attribute numbers for a table.
///
/// Parameters: $1 = table OID, $2 = array of attribute numbers
/// Returns: attribute number -> column name mapping
pub const COLUMN_NAMES_BY_ATTNUM: &str = r#"
SELECT a.attnum, a.attname
FROM pg_attribute a
WHERE a.attrelid = $1
  AND a.attnum = ANY($2)
ORDER BY a.attnum
"#;

/// Query to fetch all indexes for a table.
///
/// Parameters: $1 = table OID
/// Returns: index metadata
pub const INDEXES_FOR_TABLE: &str = r#"
SELECT
    i.indexrelid AS oid,
    ic.relname AS name,
    am.amname AS method,
    i.indisunique AS is_unique,
    EXISTS (
        SELECT 1 FROM pg_constraint c
        WHERE c.conindid = i.indexrelid
    ) AS is_constraint_index,
    i.indkey AS column_nums,             -- array of attribute numbers (0 for expressions)
    i.indoption AS column_options,       -- array of per-column flags
    pg_get_expr(i.indexprs, i.indrelid) AS expressions,  -- expression text for expression indexes
    pg_get_expr(i.indpred, i.indrelid) AS predicate,     -- partial index predicate
    obj_description(i.indexrelid, 'pg_class') AS comment
FROM pg_index i
JOIN pg_class ic ON i.indexrelid = ic.oid
JOIN pg_am am ON ic.relam = am.oid
WHERE i.indrelid = $1
  AND NOT i.indisprimary  -- primary key indexes are handled via constraints
ORDER BY ic.relname
"#;

/// Query to fetch index column details.
///
/// Parameters: $1 = index OID
/// Returns: per-column details for the index
pub const INDEX_COLUMNS: &str = r#"
SELECT
    a.attnum,
    a.attname AS column_name,
    pg_get_indexdef(i.indexrelid, a.attnum::int, false) AS expression,
    (i.indoption[a.attnum - 1] & 1) != 0 AS is_desc,
    (i.indoption[a.attnum - 1] & 2) != 0 AS nulls_first
FROM pg_index i
JOIN pg_attribute a ON a.attrelid = i.indexrelid AND a.attnum > 0
WHERE i.indexrelid = $1
ORDER BY a.attnum
"#;

/// Query to fetch all views in a namespace.
///
/// Parameters: $1 = namespace OID
/// Returns: view metadata
pub const VIEWS_IN_NAMESPACE: &str = r#"
SELECT
    c.oid,
    c.relname,
    c.relkind = 'm' AS is_materialized,
    pg_get_viewdef(c.oid, true) AS definition,
    obj_description(c.oid, 'pg_class') AS comment
FROM pg_class c
WHERE c.relnamespace = $1
  AND c.relkind IN ('v', 'm')  -- views and materialized views
ORDER BY c.relname
"#;

/// Query to fetch all sequences in a namespace.
///
/// Parameters: $1 = namespace OID
/// Returns: sequence metadata
pub const SEQUENCES_IN_NAMESPACE: &str = r#"
SELECT
    c.oid,
    c.relname,
    s.seqtypid,
    t.typname AS type_name,
    tn.nspname AS type_schema,
    format_type(s.seqtypid, NULL) AS formatted_type,
    s.seqstart AS start_value,
    s.seqincrement AS increment,
    s.seqmin AS min_value,
    s.seqmax AS max_value,
    s.seqcache AS cache_size,
    s.seqcycle AS is_cyclic,
    obj_description(c.oid, 'pg_class') AS comment
FROM pg_class c
JOIN pg_sequence s ON c.oid = s.seqrelid
JOIN pg_type t ON s.seqtypid = t.oid
JOIN pg_namespace tn ON t.typnamespace = tn.oid
WHERE c.relnamespace = $1
ORDER BY c.relname
"#;

/// Query to fetch all enum types in a namespace.
///
/// Parameters: $1 = namespace OID
/// Returns: enum type metadata
pub const ENUMS_IN_NAMESPACE: &str = r#"
SELECT
    t.oid,
    t.typname,
    array_agg(e.enumlabel ORDER BY e.enumsortorder) AS values,
    obj_description(t.oid, 'pg_type') AS comment
FROM pg_type t
JOIN pg_enum e ON t.oid = e.enumtypid
WHERE t.typnamespace = $1
  AND t.typtype = 'e'
GROUP BY t.oid, t.typname
ORDER BY t.typname
"#;

/// Query to fetch exclusion constraint elements.
///
/// Parameters: $1 = constraint OID
/// Returns: exclusion constraint element details
pub const EXCLUSION_ELEMENTS: &str = r#"
SELECT
    pg_get_indexdef(con.conindid, ordinality::int, false) AS expression,
    op.oprname AS operator
FROM pg_constraint con,
     unnest(con.conexclop) WITH ORDINALITY AS elem(opoid, ordinality)
JOIN pg_operator op ON elem.opoid = op.oid
WHERE con.oid = $1
ORDER BY elem.ordinality
"#;

/// Query to check PostgreSQL server version.
#[allow(dead_code)]
pub const SERVER_VERSION: &str = "SHOW server_version_num";
