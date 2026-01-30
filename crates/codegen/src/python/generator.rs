//! Python SQLModel code generator implementation.
//!
//! This module provides the main `PythonCodegen` struct that implements the `Codegen` trait
//! for generating Python SQLModel models from PostgreSQL table definitions.

use std::collections::HashMap;

use tern_ddl::Table;

use super::imports::ImportCollector;
use super::model::{ModelInfo, format_model, generate_model};
use super::naming::to_module_name;
use super::{OutputMode, PythonCodegenConfig};
use crate::Codegen;

/// Python SQLModel code generator.
///
/// Generates Python SQLModel model definitions from PostgreSQL table schemas.
///
/// # Example
///
/// ```ignore
/// use tern_codegen::python::{PythonCodegen, PythonCodegenConfig};
/// use tern_codegen::Codegen;
///
/// let codegen = PythonCodegen::new(PythonCodegenConfig::default());
/// let tables = vec![/* ... */];
/// let output = codegen.generate(tables);
///
/// // output["models.py"] contains the generated code
/// ```
#[derive(Debug, Clone)]
pub struct PythonCodegen {
    config: PythonCodegenConfig,
}

impl PythonCodegen {
    /// Creates a new Python code generator with the given configuration.
    pub fn new(config: PythonCodegenConfig) -> Self {
        Self { config }
    }

    /// Creates a new Python code generator with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(PythonCodegenConfig::default())
    }

    /// Generates a single models.py file containing all models.
    fn generate_single_file(&self, tables: Vec<Table>) -> HashMap<String, String> {
        let mut output = HashMap::new();

        if tables.is_empty() {
            output.insert("models.py".to_string(), generate_empty_models_file());
            return output;
        }

        // Generate all models
        let models: Vec<ModelInfo> = tables
            .iter()
            .map(|t| generate_model(t, &self.config))
            .collect();

        // Collect all imports
        let mut imports = ImportCollector::new();
        for model in &models {
            imports.merge(&model.imports);
        }

        // Build the file content
        let mut content = Vec::new();

        // Module docstring
        content.push(MODULE_DOCSTRING.to_string());
        content.push(String::new());

        // Imports
        let import_block = imports.generate();
        if !import_block.is_empty() {
            content.push(import_block);
            content.push(String::new());
        }

        // Models
        for (i, model) in models.iter().enumerate() {
            if i > 0 {
                content.push(String::new());
                content.push(String::new());
            }
            content.push(format_model(model));
        }

        // Ensure file ends with newline
        content.push(String::new());

        output.insert("models.py".to_string(), content.join("\n"));
        output
    }

    /// Generates multiple files, one per model, with a shared types module.
    fn generate_multi_file(&self, tables: Vec<Table>) -> HashMap<String, String> {
        let mut output = HashMap::new();

        if tables.is_empty() {
            output.insert("__init__.py".to_string(), generate_empty_init_file());
            return output;
        }

        // Generate all models
        let models: Vec<ModelInfo> = tables
            .iter()
            .map(|t| generate_model(t, &self.config))
            .collect();

        // Generate individual model files
        let mut all_class_names = Vec::new();
        let mut all_module_names = Vec::new();

        for model in &models {
            let module_name = to_module_name(&model.table_name);
            let filename = format!("{module_name}.py");

            // Build imports for this file
            let mut imports = ImportCollector::new();
            imports.merge(&model.imports);

            // Build file content
            let mut content = Vec::new();
            content.push(format!(
                "\"\"\"SQLModel definition for {}.\"\"\"\n",
                model.class_name
            ));

            let import_block = imports.generate();
            if !import_block.is_empty() {
                content.push(import_block);
                content.push(String::new());
            }

            content.push(format_model(model));
            content.push(String::new());

            output.insert(filename, content.join("\n"));

            all_class_names.push(model.class_name.clone());
            all_module_names.push(module_name);
        }

        // Generate __init__.py with re-exports
        let init_content = generate_init_file(&all_module_names, &all_class_names);
        output.insert("__init__.py".to_string(), init_content);

        output
    }
}

impl Default for PythonCodegen {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl Codegen for PythonCodegen {
    fn generate(&self, tables: Vec<Table>) -> HashMap<String, String> {
        match self.config.output_mode {
            OutputMode::SingleFile => self.generate_single_file(tables),
            OutputMode::MultiFile => self.generate_multi_file(tables),
        }
    }
}

/// Module docstring for generated files.
const MODULE_DOCSTRING: &str = "\"\"\"SQLModel definitions generated by Tern.

This file was automatically generated. Do not edit manually.
\"\"\"";

/// Generates an empty models.py file.
fn generate_empty_models_file() -> String {
    format!(
        "{}\n\nfrom sqlmodel import SQLModel\n\n# No tables to generate\n",
        MODULE_DOCSTRING
    )
}

/// Generates an empty __init__.py file.
fn generate_empty_init_file() -> String {
    "\"\"\"SQLModel definitions generated by Tern.\"\"\"\n\n# No models to export\n".to_string()
}

/// Generates the __init__.py file with re-exports.
fn generate_init_file(module_names: &[String], class_names: &[String]) -> String {
    let mut content = Vec::new();
    content.push("\"\"\"SQLModel definitions generated by Tern.\n".to_string());
    content.push("This file was automatically generated. Do not edit manually.".to_string());
    content.push("\"\"\"".to_string());
    content.push(String::new());

    // Import statements
    for (module, class) in module_names.iter().zip(class_names.iter()) {
        content.push(format!("from .{module} import {class}"));
    }

    content.push(String::new());

    // __all__ list
    let all_list: Vec<String> = class_names.iter().map(|c| format!("\"{c}\"")).collect();
    if all_list.len() <= 3 {
        content.push(format!("__all__ = [{}]", all_list.join(", ")));
    } else {
        content.push("__all__ = [".to_string());
        for item in &all_list {
            content.push(format!("    {item},"));
        }
        content.push("]".to_string());
    }

    content.push(String::new());
    content.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_ddl::types::QualifiedCollationName;
    use tern_ddl::{
        Column, ColumnName, Constraint, ConstraintKind, ConstraintName, IndexName, Oid,
        PrimaryKeyConstraint, SchemaName, TableKind, TableName, TypeInfo, TypeName,
    };

    fn make_column(name: &str, type_name: &str, is_nullable: bool) -> Column {
        Column {
            name: ColumnName::try_new(name.to_string()).unwrap(),
            position: 1,
            type_info: TypeInfo {
                name: TypeName::try_new(type_name.to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: type_name.to_string(),
                is_array: false,
            },
            is_nullable,
            default: None,
            generated: None,
            identity: None,
            collation: QualifiedCollationName::new(
                SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                tern_ddl::CollationName::try_new("default".to_string()).unwrap(),
            ),
            comment: None,
        }
    }

    fn make_table_with_pk(name: &str, columns: Vec<Column>, pk_columns: &[&str]) -> Table {
        let pk = Constraint {
            name: ConstraintName::try_new(format!("{}_pkey", name)).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: pk_columns
                    .iter()
                    .map(|c| ColumnName::try_new(c.to_string()).unwrap())
                    .collect(),
                index_name: IndexName::try_new(format!("{}_pkey", name)).unwrap(),
            }),
            comment: None,
        };

        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns,
            constraints: vec![pk],
            indexes: vec![],
            comment: None,
        }
    }

    #[test]
    fn test_generate_empty_tables() {
        let codegen = PythonCodegen::with_defaults();
        let output = codegen.generate(vec![]);

        assert!(output.contains_key("models.py"));
        let content = &output["models.py"];
        assert!(content.contains("SQLModel"));
        assert!(content.contains("No tables to generate"));
    }

    #[test]
    fn test_generate_single_table() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
            make_column("email", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let codegen = PythonCodegen::with_defaults();
        let output = codegen.generate(vec![table]);

        assert!(output.contains_key("models.py"));
        let content = &output["models.py"];
        assert!(content.contains("class User(SQLModel, table=True):"));
        assert!(content.contains("from sqlmodel import"));
        assert!(content.contains("__tablename__ = \"users\""));
    }

    #[test]
    fn test_generate_multiple_tables() {
        let user_columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
        ];
        let user_table = make_table_with_pk("users", user_columns, &["id"]);

        let post_columns = vec![
            make_column("id", "int4", false),
            make_column("title", "text", false),
            make_column("user_id", "int4", true),
        ];
        let post_table = make_table_with_pk("posts", post_columns, &["id"]);

        let codegen = PythonCodegen::with_defaults();
        let output = codegen.generate(vec![user_table, post_table]);

        assert!(output.contains_key("models.py"));
        let content = &output["models.py"];
        assert!(content.contains("class User(SQLModel, table=True):"));
        assert!(content.contains("class Post(SQLModel, table=True):"));
    }

    #[test]
    fn test_generate_multi_file_mode() {
        let user_columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
        ];
        let user_table = make_table_with_pk("users", user_columns, &["id"]);

        let post_columns = vec![
            make_column("id", "int4", false),
            make_column("title", "text", false),
        ];
        let post_table = make_table_with_pk("posts", post_columns, &["id"]);

        let config = PythonCodegenConfig {
            output_mode: OutputMode::MultiFile,
            ..Default::default()
        };
        let codegen = PythonCodegen::new(config);
        let output = codegen.generate(vec![user_table, post_table]);

        assert!(output.contains_key("__init__.py"));
        assert!(output.contains_key("user.py"));
        assert!(output.contains_key("post.py"));

        // Check __init__.py content
        let init = &output["__init__.py"];
        assert!(init.contains("from .user import User"));
        assert!(init.contains("from .post import Post"));
        assert!(init.contains("__all__"));

        // Check individual files
        let user_file = &output["user.py"];
        assert!(user_file.contains("class User(SQLModel, table=True):"));

        let post_file = &output["post.py"];
        assert!(post_file.contains("class Post(SQLModel, table=True):"));
    }

    #[test]
    fn test_generate_with_datetime_import() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("created_at", "timestamptz", false),
        ];
        let table = make_table_with_pk("events", columns, &["id"]);

        let codegen = PythonCodegen::with_defaults();
        let output = codegen.generate(vec![table]);

        let content = &output["models.py"];
        assert!(content.contains("from datetime import datetime"));
        assert!(content.contains("created_at: datetime"));
    }

    #[test]
    fn test_generate_with_uuid_import() {
        let columns = vec![
            make_column("id", "uuid", false),
            make_column("name", "text", false),
        ];
        let table = make_table_with_pk("items", columns, &["id"]);

        let codegen = PythonCodegen::with_defaults();
        let output = codegen.generate(vec![table]);

        let content = &output["models.py"];
        assert!(content.contains("from uuid import UUID"));
        assert!(content.contains("id:") || content.contains("id :"));
    }

    #[test]
    fn test_default_codegen_implements_trait() {
        let codegen = PythonCodegen::default();
        let output = codegen.generate(vec![]);
        assert!(output.contains_key("models.py"));
    }
}
