<img width="1584" height="396" alt="Tern: Migrations that outlive your application code" src="https://github.com/user-attachments/assets/a57cf62d-27c2-4db2-bb90-57f1c8c9cf05" />



# Tern

[![CI](https://github.com/wack/tern/actions/workflows/on-push.yml/badge.svg)](https://github.com/wack/tern/actions/workflows/on-push.yml)
[![Rust](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**A database migration tool that compiles schema changes into standalone, self-documenting executables.**

Named after the Arctic tern—a bird that makes the longest migration of any species—Tern helps you navigate the journey from one database schema to another with confidence.

## Table of Contents

- [Key Features](#key-features)
- [Quick Start](#quick-start)
- [Installation](#installation)
- [Commands](#commands)
- [Concepts](#concepts)
  - [State Backend](#state-backend)
  - [Breaking Change Detection](#breaking-change-detection)
  - [Migration Executables](#migration-executables)
- [Architecture](#architecture)
- [Contributing](#contributing)
- [Supported Schema Objects](#supported-schema-objects)
- [Roadmap](#roadmap)
- [License](#license)

## Key Features

- **Schema introspection** — Automatically captures your PostgreSQL schema structure
- **Intelligent diffing** — Detects additions, removals, modifications, and potential renames
- **Breaking change detection** — Identifies risky changes and suggests mitigation strategies
- **Standalone executables** — Compiles migrations to self-contained binaries via WebAssembly
- **State tracking** — Maintains migration history locally without polluting your database
- **Verification** — Detects schema drift between your state backend and live database

## Quick Start

```bash
# Initialize a new project (captures current schema as baseline)
tern init --from postgres://localhost/mydb

# Make changes to your database schema, then compile a migration
tern compile --database-url postgres://localhost/mydb --description "Add users table"

# View migration history
tern history

# Verify state backend matches the database
tern verify --database-url postgres://localhost/mydb
```

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/wack/tern.git
cd tern

# Build release binary
cargo build --release

# The binary is at ./target/release/tern
```

### Prerequisites

- Rust toolchain (stable, edition 2024)
- PostgreSQL database

## Commands

| Command | Description |
|---------|-------------|
| `init` | Initialize a new Tern project with state backend |
| `status` | Show state backend status and schema summary |
| `compile` | Generate migration source code from schema diff |
| `history` | List migration history |
| `show` | Show details of a specific migration |
| `record` | Record a migration as applied without executing |
| `inspect` | Examine migration files (JSON or Rust source) |
| `verify` | Check if state backend matches live database |
| `verify-chain` | Validate migration chain integrity |
| `print-migrations` | Print SQL to recreate current schema from scratch |

### Examples

```bash
# Initialize from existing database
tern init --from postgres://user:pass@localhost/mydb

# Initialize with empty schema
tern init

# Compile migration with SQL preview
tern compile \
  --database-url postgres://localhost/mydb \
  --description "Add email column to users" \
  --show-sql

# Compile and record to state backend
tern compile \
  --database-url postgres://localhost/mydb \
  --description "Add indexes" \
  --record

# Preview changes without writing files
tern compile \
  --database-url postgres://localhost/mydb \
  --description "Preview" \
  --dry-run

# Show migration details as SQL
tern show abc123 --format sql

# Output in JSON for scripting
tern status --format json
tern history --format json
```

## Concepts

### State Backend

Tern tracks migration history in a local `.tern/` directory:

```
.tern/
├── state.json           # Current schema state
└── migrations/
    ├── index.json       # Ordered list of migration IDs
    ├── 00001.json       # Baseline migration
    ├── 00002.json       # Second migration
    └── ...
```

Each migration is content-addressed using BLAKE3 hashes, ensuring tamper-resistant history and deterministic IDs.

### Breaking Change Detection

Tern analyzes schema changes and classifies them by the mitigation strategy required:

| Strategy | When Used | Examples |
|----------|-----------|----------|
| **Safe** | No special handling needed | Add nullable column, create table |
| **Dual Write** | Requires parallel structures | Rename column, change column type |
| **Backfill** | Requires populating data | Add NOT NULL to existing column |
| **Ratchet** | Requires NOT VALID + VALIDATE pattern | Add unique/check/foreign key constraint |
| **Destructive** | Intentionally removes data | Drop table, drop column |

When compiling a migration with breaking changes, Tern warns you and documents the required mitigation.

### Migration Executables

Tern compiles migrations to standalone executables using WebAssembly:

```bash
# The compiled migration is self-documenting
./migration-add-users --describe

# Preview SQL without executing
./migration-add-users --database-url postgres://localhost/mydb --dry-run

# Run the migration
./migration-add-users --database-url postgres://localhost/mydb

# Skip confirmation prompts for breaking changes
./migration-add-users --database-url postgres://localhost/mydb --yes
```

## Architecture

Tern follows a **sans-I/O** design pattern, separating database access from business logic:

```
src/
├── cli/           # Command-line interface (clap)
└── db/
    ├── model/     # In-memory PostgreSQL schema representation
    ├── query/     # Database introspection (Catalog trait)
    ├── diff/      # Schema comparison and rename detection
    ├── migrate/   # Migration planning and SQL rendering
    ├── compile/   # WebAssembly component generation
    └── state/     # Migration history tracking
```

The `Catalog` trait abstracts database queries, enabling comprehensive unit testing without a live database.

## Contributing

We welcome GitHub issues and pull requests.

Before implementing a new feature or significant change, please open an issue to discuss it with a maintainer. This helps ensure your contribution aligns with the project's direction and avoids duplicate effort.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## Supported Schema Objects

Tern captures and diffs the following PostgreSQL objects:

- Tables with columns, defaults, identity columns, and generated columns
- Primary keys, foreign keys, unique constraints, check constraints, exclusion constraints
- Indexes with expressions, sort order, nulls handling, and partial predicates
- Views (definition tracking)
- Sequences
- Enum types

## Roadmap

Future enhancements under consideration:

- Table partitioning support
- Functions and procedures
- Triggers
- Row-level security policies
- Domain and composite types
- PostgreSQL state backend (store migrations in the database)
- Multi-database support beyond PostgreSQL

## License

See [LICENSE](LICENSE) for details.
