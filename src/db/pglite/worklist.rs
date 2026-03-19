//! Worklist executor for multi-file DDL execution.
//!
//! This module provides the [`WorklistExecutor`] which executes multiple SQL
//! files while automatically resolving dependencies between them.
//!
//! # Algorithm
//!
//! The executor uses a retry-based worklist algorithm:
//!
//! 1. All SQL files are queued for execution
//! 2. Each file is attempted in turn
//! 3. If execution fails due to a missing dependency (e.g., undefined table),
//!    the file is moved to the back of the queue for retry
//! 4. If execution succeeds, the file is removed from the queue
//! 5. If a full rotation through the queue occurs without progress, a circular
//!    dependency is detected
//!
//! # Why This Approach?
//!
//! - **No SQL parsing required**: We don't need to understand SQL syntax to
//!   detect dependencies
//! - **Handles all dependency types**: Foreign keys, sequences, functions,
//!   types, triggers—all handled uniformly
//! - **PostgreSQL is the authority**: PostgreSQL itself tells us when
//!   dependencies are missing
//! - **Correctly detects circular dependencies**: If we complete a full
//!   rotation without progress, there's a genuine circular dependency
//!
//! # Example
//!
//! ```ignore
//! use tern::db::pglite::WorklistExecutor;
//! use tokio_postgres::Client;
//!
//! let executor = WorklistExecutor::new(100);
//!
//! let files = vec![
//!     PathBuf::from("schema/users.sql"),
//!     PathBuf::from("schema/orders.sql"),  // depends on users
//!     PathBuf::from("schema/products.sql"),
//! ];
//!
//! executor.execute_files(&client, &files).await?;
//! ```

use std::collections::VecDeque;
use std::path::PathBuf;

use async_trait::async_trait;
use tokio_postgres::Client;

use super::error::{WorklistError, error_codes};

/// Result of attempting to execute a single SQL file.
#[derive(Debug)]
pub enum ExecutionResult {
    /// File executed successfully.
    Success,

    /// File failed due to a missing dependency (should retry later).
    DependencyError {
        /// The PostgreSQL error code.
        code: String,
        /// The error message.
        message: String,
    },

    /// File can be skipped (object already exists).
    Skipped {
        /// The PostgreSQL error code.
        code: String,
    },

    /// File failed with a non-retryable error.
    Failed {
        /// The underlying error.
        error: tokio_postgres::Error,
    },
}

/// Abstraction over SQL execution for testability.
///
/// This trait allows the worklist algorithm to be tested without a real
/// PostgreSQL connection, following the sans-I/O pattern used elsewhere
/// in the codebase (e.g., `Catalog` / `FakeCatalog`).
#[async_trait]
pub trait SqlExecutor: Send + Sync {
    /// Executes a SQL string and returns the categorized result.
    async fn execute(&self, sql: &str) -> ExecutionResult;
}

#[async_trait]
impl SqlExecutor for Client {
    async fn execute(&self, sql: &str) -> ExecutionResult {
        match self.batch_execute(sql).await {
            Ok(()) => ExecutionResult::Success,

            Err(error) => {
                if let Some(db_error) = error.as_db_error() {
                    let code = db_error.code().code();

                    if error_codes::is_dependency_error(code) {
                        return ExecutionResult::DependencyError {
                            code: code.to_string(),
                            message: db_error.message().to_string(),
                        };
                    }

                    if error_codes::is_duplicate_error(code) {
                        return ExecutionResult::Skipped {
                            code: code.to_string(),
                        };
                    }
                }

                ExecutionResult::Failed { error }
            }
        }
    }
}

/// Executes SQL files with automatic dependency resolution.
///
/// The `WorklistExecutor` uses a retry-based algorithm to execute SQL files
/// in an order that respects dependencies, without requiring explicit
/// dependency declarations or SQL parsing.
///
/// # How It Works
///
/// 1. Files are added to a queue
/// 2. Each file is executed in turn
/// 3. If a file fails because it references an object that doesn't exist yet
///    (e.g., a table that another file creates), it's moved to the back of
///    the queue
/// 4. The process continues until all files are executed or a circular
///    dependency is detected
///
/// # Circular Dependency Detection
///
/// A circular dependency is detected when the executor makes a complete pass
/// through all remaining files without successfully executing any of them.
/// This indicates that each remaining file depends on objects that would be
/// created by other remaining files, forming a cycle.
///
/// # Error Handling
///
/// - **Dependency errors** (undefined_table, undefined_function, etc.):
///   Retry later
/// - **Duplicate errors** (duplicate_table, duplicate_schema, etc.):
///   Skip the file
/// - **Other errors** (syntax errors, constraint violations, etc.):
///   Abort immediately
#[derive(Debug, Clone)]
pub struct WorklistExecutor {
    /// Maximum number of items to process before giving up.
    ///
    /// This prevents infinite loops if something goes wrong. Set to
    /// `files.len() * (files.len() + 1)` which is the worst case for
    /// a linear dependency chain.
    max_iterations: usize,
}

impl WorklistExecutor {
    /// Creates a new worklist executor.
    ///
    /// # Arguments
    ///
    /// * `max_retries` - Maximum number of retry cycles. A value of 100
    ///   is sufficient for most schemas (up to ~100 files with complex
    ///   dependencies).
    #[must_use]
    pub fn new(max_retries: usize) -> Self {
        Self {
            max_iterations: max_retries * max_retries, // n² worst case
        }
    }

    /// Creates a new worklist executor with default settings.
    #[must_use]
    pub fn default_executor() -> Self {
        Self::new(100)
    }

    /// Executes multiple SQL files with automatic dependency resolution.
    ///
    /// Files are executed using a retry-based algorithm that automatically
    /// determines the correct execution order based on dependency errors.
    ///
    /// # Arguments
    ///
    /// * `client` - PostgreSQL client connection
    /// * `files` - Paths to SQL files to execute
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A file cannot be read
    /// - A file contains SQL syntax errors
    /// - A circular dependency is detected
    /// - Any non-retryable database error occurs
    pub async fn execute_files(
        &self,
        client: &(impl SqlExecutor + ?Sized),
        files: &[PathBuf],
    ) -> Result<(), WorklistError> {
        if files.is_empty() {
            return Ok(());
        }

        // Initialize the queue with all files
        let mut queue: VecDeque<PathBuf> = files.iter().cloned().collect();

        // Track how many items we've visited since the last successful execution
        let mut visits_since_success = 0;

        // Track total iterations for safety
        let mut total_iterations = 0;

        tracing::debug!(
            file_count = files.len(),
            "Starting worklist execution for {} files",
            files.len()
        );

        while let Some(file) = queue.pop_front() {
            total_iterations += 1;

            // Safety check to prevent infinite loops
            if total_iterations > self.max_iterations {
                tracing::error!(
                    "Worklist executor exceeded maximum iterations ({})",
                    self.max_iterations
                );
                return Err(WorklistError::CircularDependency {
                    pending_files: queue.into_iter().collect(),
                });
            }

            // Read the file content
            let sql = std::fs::read_to_string(&file).map_err(|source| WorklistError::ReadFile {
                path: file.clone(),
                source,
            })?;

            // Attempt to execute
            let result = client.execute(&sql).await;

            match result {
                ExecutionResult::Success => {
                    tracing::debug!(file = %file.display(), "Successfully executed");
                    visits_since_success = 0;
                }

                ExecutionResult::Skipped { code } => {
                    tracing::debug!(
                        file = %file.display(),
                        error_code = %code,
                        "Skipped (object already exists)"
                    );
                    visits_since_success = 0;
                }

                ExecutionResult::DependencyError { code, message } => {
                    tracing::debug!(
                        file = %file.display(),
                        error_code = %code,
                        error_message = %message,
                        "Dependency error, will retry later"
                    );

                    visits_since_success += 1;

                    // Check for circular dependency
                    // If we've visited all remaining files plus this one without success,
                    // we have a circular dependency
                    if visits_since_success > queue.len() {
                        tracing::error!(
                            "Circular dependency detected among {} files",
                            queue.len() + 1
                        );

                        // Include the current file in the pending files list
                        let mut pending = vec![file];
                        pending.extend(queue);

                        return Err(WorklistError::CircularDependency {
                            pending_files: pending,
                        });
                    }

                    // Move to back of queue for retry
                    queue.push_back(file);
                }

                ExecutionResult::Failed { error } => {
                    tracing::error!(
                        file = %file.display(),
                        error = %error,
                        "Execution failed with non-retryable error"
                    );

                    return Err(WorklistError::ExecutionFailed {
                        path: file,
                        source: error,
                    });
                }
            }
        }

        tracing::debug!(
            iterations = total_iterations,
            "Worklist execution completed successfully"
        );

        Ok(())
    }

    /// Executes SQL from multiple strings (useful for testing).
    ///
    /// This is similar to `execute_files` but works with in-memory SQL strings
    /// instead of reading from files.
    pub async fn execute_sql_strings(
        &self,
        client: &(impl SqlExecutor + ?Sized),
        sql_strings: &[(&str, &str)], // (name, sql)
    ) -> Result<(), WorklistError> {
        if sql_strings.is_empty() {
            return Ok(());
        }

        // Convert to queue of (name, sql)
        let mut queue: VecDeque<(&str, &str)> = sql_strings.iter().copied().collect();

        let mut visits_since_success = 0;
        let mut total_iterations = 0;

        while let Some((name, sql)) = queue.pop_front() {
            total_iterations += 1;

            if total_iterations > self.max_iterations {
                let pending: Vec<PathBuf> = queue.iter().map(|(n, _)| PathBuf::from(*n)).collect();
                return Err(WorklistError::CircularDependency {
                    pending_files: pending,
                });
            }

            let result = client.execute(sql).await;

            match result {
                ExecutionResult::Success | ExecutionResult::Skipped { .. } => {
                    visits_since_success = 0;
                }

                ExecutionResult::DependencyError { .. } => {
                    visits_since_success += 1;

                    if visits_since_success > queue.len() {
                        let mut pending = vec![PathBuf::from(name)];
                        pending.extend(queue.iter().map(|(n, _)| PathBuf::from(*n)));
                        return Err(WorklistError::CircularDependency {
                            pending_files: pending,
                        });
                    }

                    queue.push_back((name, sql));
                }

                ExecutionResult::Failed { error } => {
                    return Err(WorklistError::ExecutionFailed {
                        path: PathBuf::from(name),
                        source: error,
                    });
                }
            }
        }

        Ok(())
    }
}

impl Default for WorklistExecutor {
    fn default() -> Self {
        Self::default_executor()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn executor_construction() {
        let executor = WorklistExecutor::new(50);
        assert_eq!(executor.max_iterations, 2500); // 50 * 50

        let default = WorklistExecutor::default();
        assert_eq!(default.max_iterations, 10000); // 100 * 100
    }

    #[test]
    fn execution_result_debug() {
        let success = ExecutionResult::Success;
        assert!(format!("{:?}", success).contains("Success"));

        let dep_error = ExecutionResult::DependencyError {
            code: "42P01".to_string(),
            message: "undefined_table".to_string(),
        };
        assert!(format!("{:?}", dep_error).contains("42P01"));
    }

    // =========================================================================
    // FakeSqlExecutor — test double for the worklist algorithm
    // =========================================================================

    /// Tracks state for a single SQL entry in the fake executor.
    ///
    /// Each entry has a sequence of results to return on successive calls.
    /// This allows simulating dependency resolution: the first call might
    /// return `DependencyError`, and after the dependency is "created" by
    /// another entry succeeding, the next call returns `Success`.
    struct FakeEntry {
        results: Vec<ExecutionResult>,
        call_index: usize,
    }

    /// A fake SQL executor that returns pre-programmed results per SQL string.
    ///
    /// The executor tracks execution order so tests can verify that the
    /// worklist algorithm processes entries in the expected sequence.
    struct FakeSqlExecutor {
        entries: Mutex<HashMap<String, FakeEntry>>,
        execution_order: Mutex<Vec<String>>,
    }

    impl FakeSqlExecutor {
        fn new() -> Self {
            Self {
                entries: Mutex::new(HashMap::new()),
                execution_order: Mutex::new(Vec::new()),
            }
        }

        /// Programs a SQL string to return the given sequence of results.
        fn on_sql(self, sql: &str, results: Vec<ExecutionResult>) -> Self {
            self.entries.lock().unwrap().insert(
                sql.to_string(),
                FakeEntry {
                    results,
                    call_index: 0,
                },
            );
            self
        }

        /// Returns the SQL strings in the order they were executed.
        fn execution_order(&self) -> Vec<String> {
            self.execution_order.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl SqlExecutor for FakeSqlExecutor {
        async fn execute(&self, sql: &str) -> ExecutionResult {
            self.execution_order.lock().unwrap().push(sql.to_string());

            let mut entries = self.entries.lock().unwrap();
            let entry = entries
                .get_mut(sql)
                .unwrap_or_else(|| panic!("unexpected SQL: {sql}"));

            let idx = entry.call_index;
            entry.call_index += 1;

            if idx < entry.results.len() {
                // Return the pre-programmed result for this call
                match &entry.results[idx] {
                    ExecutionResult::Success => ExecutionResult::Success,
                    ExecutionResult::DependencyError { code, message } => {
                        ExecutionResult::DependencyError {
                            code: code.clone(),
                            message: message.clone(),
                        }
                    }
                    ExecutionResult::Skipped { code } => {
                        ExecutionResult::Skipped { code: code.clone() }
                    }
                    ExecutionResult::Failed { .. } => {
                        panic!(
                            "FakeSqlExecutor cannot clone tokio_postgres::Error; use DependencyError for retryable or Success for terminal results"
                        )
                    }
                }
            } else {
                // Default to success after exhausting programmed results
                ExecutionResult::Success
            }
        }
    }

    fn dep_error() -> ExecutionResult {
        ExecutionResult::DependencyError {
            code: "42P01".to_string(),
            message: "relation does not exist".to_string(),
        }
    }

    fn dup_skip() -> ExecutionResult {
        ExecutionResult::Skipped {
            code: "42P07".to_string(),
        }
    }

    // =========================================================================
    // Tests for execute_sql_strings
    // =========================================================================

    #[tokio::test]
    async fn all_files_succeed_on_first_try() {
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE TABLE users (id int)",
                vec![ExecutionResult::Success],
            )
            .on_sql(
                "CREATE TABLE orders (id int)",
                vec![ExecutionResult::Success],
            )
            .on_sql(
                "CREATE TABLE products (id int)",
                vec![ExecutionResult::Success],
            );

        let sql_strings = [
            ("users.sql", "CREATE TABLE users (id int)"),
            ("orders.sql", "CREATE TABLE orders (id int)"),
            ("products.sql", "CREATE TABLE products (id int)"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(order.len(), 3);
    }

    #[tokio::test]
    async fn empty_input_succeeds_immediately() {
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new();

        let result = executor.execute_sql_strings(&fake, &[]).await;
        assert!(result.is_ok());
        assert!(fake.execution_order().is_empty());
    }

    #[tokio::test]
    async fn single_file_succeeds() {
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE TABLE t (id int)", vec![ExecutionResult::Success]);

        let sql_strings = [("t.sql", "CREATE TABLE t (id int)")];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());
        assert_eq!(fake.execution_order().len(), 1);
    }

    #[tokio::test]
    async fn dependency_error_retries_and_succeeds() {
        // orders.sql depends on users.sql. When orders is tried first, it
        // fails with a dependency error. After users succeeds, orders retries
        // and succeeds.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE TABLE orders (user_id int REFERENCES users(id))",
                vec![dep_error(), ExecutionResult::Success],
            )
            .on_sql(
                "CREATE TABLE users (id int PRIMARY KEY)",
                vec![ExecutionResult::Success],
            );

        let sql_strings = [
            (
                "orders.sql",
                "CREATE TABLE orders (user_id int REFERENCES users(id))",
            ),
            ("users.sql", "CREATE TABLE users (id int PRIMARY KEY)"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        // orders tried first -> dep error, users succeeds, orders retried -> success
        assert_eq!(order.len(), 3);
        assert_eq!(
            order[0],
            "CREATE TABLE orders (user_id int REFERENCES users(id))"
        );
        assert_eq!(order[1], "CREATE TABLE users (id int PRIMARY KEY)");
        assert_eq!(
            order[2],
            "CREATE TABLE orders (user_id int REFERENCES users(id))"
        );
    }

    #[tokio::test]
    async fn linear_dependency_chain_resolves() {
        // C depends on B, B depends on A. Given in reverse order: C, B, A.
        // Expected: C fails, B fails, A succeeds, C fails, B succeeds, C succeeds.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE C",
                vec![dep_error(), dep_error(), ExecutionResult::Success],
            )
            .on_sql("CREATE B", vec![dep_error(), ExecutionResult::Success])
            .on_sql("CREATE A", vec![ExecutionResult::Success]);

        let sql_strings = [
            ("c.sql", "CREATE C"),
            ("b.sql", "CREATE B"),
            ("a.sql", "CREATE A"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(
            order,
            vec![
                "CREATE C", // fail (dep on B)
                "CREATE B", // fail (dep on A)
                "CREATE A", // success
                "CREATE C", // fail (dep on B still)
                "CREATE B", // success
                "CREATE C", // success
            ]
        );
    }

    #[tokio::test]
    async fn circular_dependency_detected_two_files() {
        // A depends on B, B depends on A — circular.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![dep_error(), dep_error(), dep_error()])
            .on_sql("CREATE B", vec![dep_error(), dep_error(), dep_error()]);

        let sql_strings = [("a.sql", "CREATE A"), ("b.sql", "CREATE B")];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        match &err {
            WorklistError::CircularDependency { pending_files } => {
                assert_eq!(pending_files.len(), 2);
            }
            other => panic!("expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn circular_dependency_detected_three_files() {
        // A -> B -> C -> A: all fail on every attempt.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![dep_error(), dep_error()])
            .on_sql("CREATE B", vec![dep_error(), dep_error()])
            .on_sql("CREATE C", vec![dep_error(), dep_error()]);

        let sql_strings = [
            ("a.sql", "CREATE A"),
            ("b.sql", "CREATE B"),
            ("c.sql", "CREATE C"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            WorklistError::CircularDependency { pending_files } => {
                assert_eq!(pending_files.len(), 3);
            }
            other => panic!("expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn single_file_dependency_error_is_circular() {
        // A single file that always fails with a dependency error
        // is effectively a self-referencing circular dependency.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new().on_sql("CREATE A", vec![dep_error()]);

        let sql_strings = [("a.sql", "CREATE A")];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            WorklistError::CircularDependency { pending_files } => {
                assert_eq!(pending_files.len(), 1);
                assert_eq!(pending_files[0], PathBuf::from("a.sql"));
            }
            other => panic!("expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn duplicate_objects_are_skipped() {
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE TABLE users (id int)",
                vec![ExecutionResult::Success],
            )
            .on_sql("CREATE TABLE users (id int) -- dup", vec![dup_skip()]);

        let sql_strings = [
            ("users.sql", "CREATE TABLE users (id int)"),
            ("users_copy.sql", "CREATE TABLE users (id int) -- dup"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn skip_resets_visit_counter() {
        // Verifies that a Skipped result counts as progress (resets
        // visits_since_success), preventing false circular dependency detection.
        // Sequence: A=dep_error, B=skip, A=success
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![dep_error(), ExecutionResult::Success])
            .on_sql("CREATE B", vec![dup_skip()]);

        let sql_strings = [("a.sql", "CREATE A"), ("b.sql", "CREATE B")];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(order, vec!["CREATE A", "CREATE B", "CREATE A"]);
    }

    #[tokio::test]
    async fn mixed_success_dependency_and_skip() {
        // A complex scenario: 4 files with mixed behaviors.
        // - schema.sql: succeeds immediately
        // - types.sql: duplicate (skipped)
        // - orders.sql: depends on users, fails then succeeds
        // - users.sql: succeeds immediately
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE SCHEMA app", vec![ExecutionResult::Success])
            .on_sql("CREATE TYPE status", vec![dup_skip()])
            .on_sql(
                "CREATE TABLE orders",
                vec![dep_error(), ExecutionResult::Success],
            )
            .on_sql("CREATE TABLE users", vec![ExecutionResult::Success]);

        let sql_strings = [
            ("schema.sql", "CREATE SCHEMA app"),
            ("types.sql", "CREATE TYPE status"),
            ("orders.sql", "CREATE TABLE orders"),
            ("users.sql", "CREATE TABLE users"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(
            order,
            vec![
                "CREATE SCHEMA app",   // success
                "CREATE TYPE status",  // skip
                "CREATE TABLE orders", // dep error -> back of queue
                "CREATE TABLE users",  // success
                "CREATE TABLE orders", // retry -> success
            ]
        );
    }

    #[tokio::test]
    async fn partial_circular_with_some_successful() {
        // Two files succeed, but the remaining two form a cycle.
        // A: success, B: success, C: always dep_error, D: always dep_error
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![ExecutionResult::Success])
            .on_sql("CREATE B", vec![ExecutionResult::Success])
            .on_sql("CREATE C", vec![dep_error(), dep_error(), dep_error()])
            .on_sql("CREATE D", vec![dep_error(), dep_error(), dep_error()]);

        let sql_strings = [
            ("a.sql", "CREATE A"),
            ("b.sql", "CREATE B"),
            ("c.sql", "CREATE C"),
            ("d.sql", "CREATE D"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            WorklistError::CircularDependency { pending_files } => {
                assert_eq!(pending_files.len(), 2);
                let names: Vec<_> = pending_files.iter().map(|p| p.to_str().unwrap()).collect();
                assert!(names.contains(&"c.sql"));
                assert!(names.contains(&"d.sql"));
            }
            other => panic!("expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn max_iterations_safety_limit() {
        // With max_retries=2, max_iterations=4. Three files that all keep
        // failing should hit the safety limit before natural circular
        // dependency detection if the queue is large enough.
        let executor = WorklistExecutor::new(2); // max_iterations = 4
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE A",
                vec![
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                ],
            )
            .on_sql(
                "CREATE B",
                vec![
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                ],
            )
            .on_sql(
                "CREATE C",
                vec![
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                    dep_error(),
                ],
            );

        let sql_strings = [
            ("a.sql", "CREATE A"),
            ("b.sql", "CREATE B"),
            ("c.sql", "CREATE C"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            WorklistError::CircularDependency { .. } => {}
            other => panic!("expected CircularDependency, got: {other}"),
        }
    }

    #[tokio::test]
    async fn dependency_resolves_after_retry() {
        // File A depends on B, C, and D. On its first attempt A fails because
        // dependencies haven't been created yet. After B, C, D all succeed,
        // A is retried and succeeds.
        //
        // Trace: A(dep_error) -> B(success) -> C(success) -> D(success) -> A(success)
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![dep_error(), ExecutionResult::Success])
            .on_sql("CREATE B", vec![ExecutionResult::Success])
            .on_sql("CREATE C", vec![ExecutionResult::Success])
            .on_sql("CREATE D", vec![ExecutionResult::Success]);

        let sql_strings = [
            ("a.sql", "CREATE A"),
            ("b.sql", "CREATE B"),
            ("c.sql", "CREATE C"),
            ("d.sql", "CREATE D"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(
            order,
            vec!["CREATE A", "CREATE B", "CREATE C", "CREATE D", "CREATE A"]
        );
    }

    #[tokio::test]
    async fn diamond_dependency_resolves() {
        // Diamond: D depends on B and C; B and C each depend on A.
        // Input order: D, C, B, A
        //
        // Trace:
        //   D(dep) -> C(dep) -> B(dep) -> A(ok)     [A resolves, queue: D,C,B]
        //   D(dep) -> C(ok)  -> B(ok)               [B,C resolve, queue: D]
        //   D(ok)                                    [D resolves]
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql(
                "CREATE D",
                vec![dep_error(), dep_error(), ExecutionResult::Success],
            )
            .on_sql("CREATE C", vec![dep_error(), ExecutionResult::Success])
            .on_sql("CREATE B", vec![dep_error(), ExecutionResult::Success])
            .on_sql("CREATE A", vec![ExecutionResult::Success]);

        let sql_strings = [
            ("d.sql", "CREATE D"),
            ("c.sql", "CREATE C"),
            ("b.sql", "CREATE B"),
            ("a.sql", "CREATE A"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        assert_eq!(
            order,
            vec![
                "CREATE D", // dep error (needs B/C)
                "CREATE C", // dep error (needs A)
                "CREATE B", // dep error (needs A)
                "CREATE A", // success
                "CREATE D", // dep error (needs C)
                "CREATE C", // success
                "CREATE B", // success
                "CREATE D", // success
            ]
        );
    }

    #[tokio::test]
    async fn already_correct_order_no_retries() {
        // Files given in correct dependency order: no retries needed.
        let executor = WorklistExecutor::new(10);
        let fake = FakeSqlExecutor::new()
            .on_sql("CREATE A", vec![ExecutionResult::Success])
            .on_sql("CREATE B", vec![ExecutionResult::Success])
            .on_sql("CREATE C", vec![ExecutionResult::Success]);

        let sql_strings = [
            ("a.sql", "CREATE A"),
            ("b.sql", "CREATE B"),
            ("c.sql", "CREATE C"),
        ];

        let result = executor.execute_sql_strings(&fake, &sql_strings).await;
        assert!(result.is_ok());

        let order = fake.execution_order();
        // Each file executed exactly once
        assert_eq!(order.len(), 3);
        assert_eq!(order, vec!["CREATE A", "CREATE B", "CREATE C"]);
    }
}
