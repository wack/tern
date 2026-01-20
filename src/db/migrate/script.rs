//! Final rendered migration output.
//!
//! A `MigrationScript` contains the rendered SQL for a migration plan,
//! ready for execution against a database.

use super::render::RenderedOperation;

/// A complete migration script ready for execution.
#[derive(Debug, Clone)]
pub struct MigrationScript {
    /// Rendered operations in order.
    pub operations: Vec<RenderedOperation>,
}

impl MigrationScript {
    /// Create from rendered operations.
    pub fn new(operations: Vec<RenderedOperation>) -> Self {
        Self { operations }
    }

    /// Create an empty script.
    pub fn empty() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// Returns true if the script has no operations.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns the number of operations.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Generate the forward migration SQL.
    pub fn to_sql(&self) -> String {
        self.to_sql_with_options(SqlOptions::default())
    }

    /// Generate forward SQL with options.
    pub fn to_sql_with_options(&self, options: SqlOptions) -> String {
        let mut sql = String::new();

        if options.include_transaction {
            sql.push_str("BEGIN;\n\n");
        }

        for (i, op) in self.operations.iter().enumerate() {
            if options.include_comments {
                sql.push_str(&format!("-- {}\n", op.description));
            }

            for stmt in &op.forward {
                sql.push_str(stmt);
                sql.push_str(";\n");
            }

            if i < self.operations.len() - 1 {
                sql.push('\n');
            }
        }

        if options.include_transaction {
            sql.push_str("\nCOMMIT;\n");
        }

        sql
    }

    /// Generate the rollback SQL (operations in reverse order).
    pub fn to_rollback_sql(&self) -> Option<String> {
        self.to_rollback_sql_with_options(SqlOptions::default())
    }

    /// Generate rollback SQL with options.
    pub fn to_rollback_sql_with_options(&self, options: SqlOptions) -> Option<String> {
        let mut sql = String::new();
        let mut has_any = false;

        if options.include_transaction {
            sql.push_str("BEGIN;\n\n");
        }

        // Process in reverse order
        for (i, op) in self.operations.iter().rev().enumerate() {
            if let Some(ref rollback) = op.rollback {
                has_any = true;
                if options.include_comments {
                    sql.push_str(&format!("-- Rollback: {}\n", op.description));
                }
                for stmt in rollback {
                    sql.push_str(stmt);
                    sql.push_str(";\n");
                }
                if i < self.operations.len() - 1 {
                    sql.push('\n');
                }
            }
        }

        if options.include_transaction && has_any {
            sql.push_str("\nCOMMIT;\n");
        }

        if has_any { Some(sql) } else { None }
    }

    /// Get all forward statements as a flat list.
    pub fn all_statements(&self) -> Vec<&str> {
        self.operations
            .iter()
            .flat_map(|op| op.forward.iter().map(String::as_str))
            .collect()
    }

    /// Get operation descriptions.
    pub fn descriptions(&self) -> Vec<&str> {
        self.operations
            .iter()
            .map(|op| op.description.as_str())
            .collect()
    }
}

/// Options for SQL generation.
#[derive(Debug, Clone)]
pub struct SqlOptions {
    /// Wrap in BEGIN/COMMIT transaction.
    pub include_transaction: bool,
    /// Include comment descriptions before each operation.
    pub include_comments: bool,
}

impl Default for SqlOptions {
    fn default() -> Self {
        Self {
            include_transaction: true,
            include_comments: true,
        }
    }
}

impl SqlOptions {
    /// Create options without transaction wrapper.
    pub fn without_transaction() -> Self {
        Self {
            include_transaction: false,
            ..Default::default()
        }
    }

    /// Create options without comments.
    pub fn without_comments() -> Self {
        Self {
            include_comments: false,
            ..Default::default()
        }
    }

    /// Create minimal options (no transaction, no comments).
    pub fn minimal() -> Self {
        Self {
            include_transaction: false,
            include_comments: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_operation(desc: &str, forward: &str, rollback: Option<&str>) -> RenderedOperation {
        RenderedOperation {
            forward: vec![forward.to_string()],
            rollback: rollback.map(|r| vec![r.to_string()]),
            description: desc.to_string(),
        }
    }

    #[test]
    fn empty_script() {
        let script = MigrationScript::empty();
        assert!(script.is_empty());
        assert_eq!(script.len(), 0);
        assert_eq!(script.to_sql(), "BEGIN;\n\n\nCOMMIT;\n");
    }

    #[test]
    fn to_sql_with_transaction() {
        let script = MigrationScript::new(vec![test_operation(
            "Create table users",
            "CREATE TABLE users (id int)",
            Some("DROP TABLE users"),
        )]);

        let sql = script.to_sql();
        assert!(sql.starts_with("BEGIN;\n"));
        assert!(sql.contains("-- Create table users"));
        assert!(sql.contains("CREATE TABLE users (id int);"));
        assert!(sql.ends_with("COMMIT;\n"));
    }

    #[test]
    fn to_sql_without_transaction() {
        let script = MigrationScript::new(vec![test_operation(
            "Create table users",
            "CREATE TABLE users (id int)",
            None,
        )]);

        let sql = script.to_sql_with_options(SqlOptions::without_transaction());
        assert!(!sql.contains("BEGIN"));
        assert!(!sql.contains("COMMIT"));
        assert!(sql.contains("CREATE TABLE users"));
    }

    #[test]
    fn to_sql_without_comments() {
        let script = MigrationScript::new(vec![test_operation(
            "Create table users",
            "CREATE TABLE users (id int)",
            None,
        )]);

        let sql = script.to_sql_with_options(SqlOptions::without_comments());
        assert!(!sql.contains("-- Create table"));
        assert!(sql.contains("CREATE TABLE users"));
    }

    #[test]
    fn to_rollback_sql() {
        let script = MigrationScript::new(vec![
            test_operation(
                "Create users",
                "CREATE TABLE users (id int)",
                Some("DROP TABLE users"),
            ),
            test_operation(
                "Create posts",
                "CREATE TABLE posts (id int)",
                Some("DROP TABLE posts"),
            ),
        ]);

        let rollback = script.to_rollback_sql().unwrap();

        // Operations should be in reverse order
        let users_pos = rollback.find("DROP TABLE users").unwrap();
        let posts_pos = rollback.find("DROP TABLE posts").unwrap();
        assert!(
            posts_pos < users_pos,
            "posts should be dropped before users"
        );
    }

    #[test]
    fn to_rollback_sql_none_when_no_rollback() {
        let script = MigrationScript::new(vec![test_operation(
            "Drop table users",
            "DROP TABLE users",
            None,
        )]);

        assert!(script.to_rollback_sql().is_none());
    }

    #[test]
    fn all_statements() {
        let script = MigrationScript::new(vec![
            test_operation("Op 1", "STMT 1", None),
            test_operation("Op 2", "STMT 2", None),
        ]);

        let stmts = script.all_statements();
        assert_eq!(stmts, vec!["STMT 1", "STMT 2"]);
    }

    #[test]
    fn descriptions() {
        let script = MigrationScript::new(vec![
            test_operation("First op", "STMT 1", None),
            test_operation("Second op", "STMT 2", None),
        ]);

        let descs = script.descriptions();
        assert_eq!(descs, vec!["First op", "Second op"]);
    }

    #[test]
    fn multiple_statements_per_operation() {
        let op = RenderedOperation {
            forward: vec!["STMT 1".to_string(), "STMT 2".to_string()],
            rollback: None,
            description: "Multi-statement op".to_string(),
        };

        let script = MigrationScript::new(vec![op]);
        let sql = script.to_sql_with_options(SqlOptions::minimal());

        assert!(sql.contains("STMT 1;\nSTMT 2;"));
    }
}
