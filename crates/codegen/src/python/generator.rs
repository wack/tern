//! Python SQLModel code generator implementation.
//!
//! This module provides the main `PythonCodegen` struct that implements the `Codegen` trait
//! for generating Python SQLModel models from PostgreSQL table definitions.

use std::collections::HashMap;

use tern_ddl::Table;

use super::imports::ImportCollector;
use super::model::{ModelInfo, format_base_model, format_model, generate_model};
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
        // Pass all tables if we need to generate relationships
        let all_tables = if self.config.generate_relationships {
            Some(tables.as_slice())
        } else {
            None
        };

        let models: Vec<ModelInfo> = tables
            .iter()
            .map(|t| generate_model(t, &self.config, all_tables))
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

        // Base models (if configured)
        // Output base models first so they can be referenced by the table models
        let has_base_models = models.iter().any(|m| m.base_model.is_some());
        if has_base_models {
            for model in &models {
                if let Some(ref base_model) = model.base_model {
                    content.push(format_base_model(base_model));
                    content.push(String::new());
                    content.push(String::new());
                }
            }
        }

        // Table models
        for (i, model) in models.iter().enumerate() {
            if i > 0 || has_base_models {
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
        // Pass all tables if we need to generate relationships
        let all_tables = if self.config.generate_relationships {
            Some(tables.as_slice())
        } else {
            None
        };

        let models: Vec<ModelInfo> = tables
            .iter()
            .map(|t| generate_model(t, &self.config, all_tables))
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

            // Include base model in docstring if present
            let docstring = if model.base_model.is_some() {
                format!(
                    "\"\"\"SQLModel and base model definitions for {}.\"\"\"\n",
                    model.class_name
                )
            } else {
                format!(
                    "\"\"\"SQLModel definition for {}.\"\"\"\n",
                    model.class_name
                )
            };
            content.push(docstring);

            let import_block = imports.generate();
            if !import_block.is_empty() {
                content.push(import_block);
                content.push(String::new());
            }

            // Base model first (if configured)
            if let Some(ref base_model) = model.base_model {
                content.push(format_base_model(base_model));
                content.push(String::new());
                content.push(String::new());
            }

            content.push(format_model(model));
            content.push(String::new());

            output.insert(filename, content.join("\n"));

            // If we have a base model, add it to the exports too
            if let Some(ref base_model) = model.base_model {
                all_class_names.push(base_model.class_name.clone());
                all_module_names.push(module_name.clone());
            }

            all_class_names.push(model.class_name.clone());
            all_module_names.push(module_name);
        }

        // Generate __init__.py with re-exports
        let init_content = generate_init_file(
            &all_module_names,
            &all_class_names,
            &self.config.module_prefix,
        );
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
///
/// If `module_prefix` is provided, uses absolute imports (e.g., `from app.models.user import User`).
/// Otherwise, uses relative imports (e.g., `from .user import User`).
fn generate_init_file(
    module_names: &[String],
    class_names: &[String],
    module_prefix: &Option<String>,
) -> String {
    let mut content = Vec::new();
    content.push("\"\"\"SQLModel definitions generated by Tern.\n".to_string());
    content.push("This file was automatically generated. Do not edit manually.".to_string());
    content.push("\"\"\"".to_string());
    content.push(String::new());

    // Import statements
    for (module, class) in module_names.iter().zip(class_names.iter()) {
        let import_path = match module_prefix {
            Some(prefix) => format!("{prefix}.{module}"),
            None => format!(".{module}"),
        };
        content.push(format!("from {import_path} import {class}"));
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

    #[test]
    fn test_generate_multi_file_with_module_prefix() {
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
            module_prefix: Some("app.models".to_string()),
            ..Default::default()
        };
        let codegen = PythonCodegen::new(config);
        let output = codegen.generate(vec![user_table, post_table]);

        // Check __init__.py uses absolute imports with prefix
        let init = &output["__init__.py"];
        assert!(
            init.contains("from app.models.user import User"),
            "__init__.py should use absolute import with module prefix, got: {}",
            init
        );
        assert!(
            init.contains("from app.models.post import Post"),
            "__init__.py should use absolute import with module prefix, got: {}",
            init
        );
        assert!(
            !init.contains("from .user import"),
            "Should not have relative import with module_prefix"
        );
    }

    #[test]
    fn test_generate_with_base_models() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("email", "text", false),
            make_column("name", "text", true),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let config = PythonCodegenConfig {
            generate_base_models: true,
            ..Default::default()
        };
        let codegen = PythonCodegen::new(config);
        let output = codegen.generate(vec![table]);

        let content = &output["models.py"];

        // Should contain base model class
        assert!(
            content.contains("class UserBase(BaseModel):"),
            "Should have base model class, got: {}",
            content
        );

        // Base model should have regular fields but not PK
        assert!(
            content.contains("class User(SQLModel, table=True):"),
            "Should have table model class"
        );

        // Should import BaseModel from pydantic
        assert!(
            content.contains("from pydantic import BaseModel"),
            "Should import BaseModel from pydantic, got: {}",
            content
        );

        // Base model should NOT have the id field (PK)
        // Check that the base model doesn't have primary_key=True
        let base_model_section = content.split("class User(SQLModel").next().unwrap_or("");
        assert!(
            !base_model_section.contains("primary_key=True"),
            "Base model should not have primary_key field"
        );
    }

    #[test]
    fn test_generate_base_models_multi_file() {
        let columns = vec![
            make_column("id", "int4", false),
            make_column("email", "text", false),
        ];
        let table = make_table_with_pk("users", columns, &["id"]);

        let config = PythonCodegenConfig {
            output_mode: OutputMode::MultiFile,
            generate_base_models: true,
            ..Default::default()
        };
        let codegen = PythonCodegen::new(config);
        let output = codegen.generate(vec![table]);

        let user_file = &output["user.py"];

        // Should contain both base model and table model
        assert!(
            user_file.contains("class UserBase(BaseModel):"),
            "File should have base model"
        );
        assert!(
            user_file.contains("class User(SQLModel, table=True):"),
            "File should have table model"
        );

        // __init__.py should export both classes
        let init = &output["__init__.py"];
        assert!(
            init.contains("UserBase"),
            "__init__.py should export UserBase"
        );
        assert!(init.contains("User"), "__init__.py should export User");
    }

    #[test]
    fn test_generate_with_relationships() {
        use tern_ddl::types::{ForeignKeyAction, QualifiedName};
        use tern_ddl::{Constraint, ConstraintKind, ForeignKeyConstraint};

        // Create users table
        let user_columns = vec![
            make_column("id", "int4", false),
            make_column("name", "text", false),
        ];
        let users_table = make_table_with_pk("users", user_columns, &["id"]);

        // Create posts table with FK to users
        let post_columns = vec![
            make_column("id", "int4", false),
            make_column("title", "text", false),
            make_column("author_id", "int4", false),
        ];
        let mut posts_table = make_table_with_pk("posts", post_columns, &["id"]);
        posts_table.constraints.push(Constraint {
            name: ConstraintName::try_new("posts_author_id_fkey".to_string()).unwrap(),
            kind: ConstraintKind::ForeignKey(ForeignKeyConstraint {
                columns: vec![ColumnName::try_new("author_id".to_string()).unwrap()],
                referenced_table: QualifiedName::new(
                    SchemaName::try_new("public".to_string()).unwrap(),
                    TableName::try_new("users".to_string()).unwrap(),
                ),
                referenced_columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                on_delete: ForeignKeyAction::Cascade,
                on_update: ForeignKeyAction::NoAction,
                is_deferrable: false,
                is_initially_deferred: false,
            }),
            comment: None,
        });

        let config = PythonCodegenConfig {
            generate_relationships: true,
            ..Default::default()
        };
        let codegen = PythonCodegen::new(config);
        let output = codegen.generate(vec![users_table, posts_table]);

        let content = &output["models.py"];

        // Post should have a relationship to User
        assert!(
            content.contains("# Relationships"),
            "Should have relationships section, got: {}",
            content
        );
        assert!(
            content.contains("author:"),
            "Post should have author relationship, got: {}",
            content
        );
        assert!(
            content.contains("Relationship("),
            "Should have Relationship declaration, got: {}",
            content
        );

        // Should import Relationship
        assert!(
            content.contains("Relationship"),
            "Should import Relationship"
        );
    }
}
