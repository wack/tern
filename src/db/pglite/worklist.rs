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
        client: &Client,
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
            let result = self.execute_sql(client, &sql).await;

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

    /// Executes a single SQL statement and categorizes the result.
    async fn execute_sql(&self, client: &Client, sql: &str) -> ExecutionResult {
        match client.batch_execute(sql).await {
            Ok(()) => ExecutionResult::Success,

            Err(error) => {
                // Try to extract the PostgreSQL error code
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

    /// Executes SQL from multiple strings (useful for testing).
    ///
    /// This is similar to `execute_files` but works with in-memory SQL strings
    /// instead of reading from files.
    pub async fn execute_sql_strings(
        &self,
        client: &Client,
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

            let result = self.execute_sql(client, sql).await;

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
}
