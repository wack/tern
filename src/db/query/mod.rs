//! PostgreSQL catalog query module.
//!
//! This module provides functionality for loading schema metadata from
//! PostgreSQL's system catalogs into the domain model types.
//!
//! # Architecture
//!
//! This module follows the sans-I/O design pattern:
//!
//! - [`Catalog`]: A trait abstracting database queries
//! - [`PostgresCatalog`]: The real implementation using tokio-postgres
//! - [`FakeCatalog`]: A test double for unit testing
//! - [`load_namespace`]: Pure business logic that works with any `Catalog`
//!
//! # Example
//!
//! ```ignore
//! use tern::db::{connect, query::{load_namespace, PostgresCatalog}};
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     // Connect to the database
//!     let client = connect("postgres://localhost/mydb").await?;
//!
//!     // Create the catalog adapter
//!     let catalog = PostgresCatalog::new(&client);
//!
//!     // Load the public schema
//!     let namespace = load_namespace(&catalog, "public").await?;
//!
//!     println!("Schema: {}", namespace.name.as_ref());
//!     println!("Tables: {}", namespace.tables.len());
//!     println!("Views: {}", namespace.views.len());
//!     println!("Sequences: {}", namespace.sequences.len());
//!     println!("Enums: {}", namespace.enums.len());
//!
//!     for table in &namespace.tables {
//!         println!("  Table: {} ({} columns)", table.name.as_ref(), table.columns.len());
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! # Testing
//!
//! Use `FakeCatalog` for unit testing without a database:
//!
//! ```
//! use tern::db::query::{FakeCatalog, load_namespace};
//! use tern::db::query::catalog::{NamespaceRow, TableRow, ColumnRow};
//!
//! #[tokio::main]
//! async fn main() {
//!     let catalog = FakeCatalog::new()
//!         .with_namespace(NamespaceRow {
//!             oid: 100,
//!             name: "public".to_string(),
//!             comment: None,
//!         })
//!         .with_table(100, TableRow {
//!             oid: 200,
//!             name: "users".to_string(),
//!             relkind: 'r',
//!             comment: None,
//!         });
//!
//!     let ns = load_namespace(&catalog, "public").await.unwrap();
//!     assert_eq!(ns.tables.len(), 1);
//! }
//! ```
//!
//! # Supported Objects
//!
//! The loader supports the following PostgreSQL objects:
//!
//! - **Tables**: Regular and partitioned tables
//!   - Columns with types, defaults, identity, generated columns
//!   - Primary key, unique, foreign key, check, and exclusion constraints
//!   - Indexes (btree, hash, gin, gist, brin, spgist)
//!
//! - **Views**: Regular and materialized views with their definitions
//!
//! - **Sequences**: With all sequence parameters (start, increment, min, max, cache, cycle)
//!
//! - **Enum Types**: With their ordered list of values
//!
//! - **Comments**: Object comments from `COMMENT ON` statements
//!
//! # Limitations
//!
//! The following are not currently supported:
//!
//! - Partitions and partition metadata
//! - Foreign tables and foreign data wrappers
//! - Functions and procedures
//! - Triggers
//! - Row-level security policies
//! - Domain types
//! - Composite types (except as column types)
//! - Extensions
//! - Tablespaces

pub mod catalog;
mod error;
mod fake;
mod loader;
mod postgres;
mod sql;

pub use catalog::Catalog;
pub use error::QueryError;
pub use fake::FakeCatalog;
pub use loader::load_namespace;
pub use postgres::PostgresCatalog;
