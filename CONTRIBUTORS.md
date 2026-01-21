# Contributing to Tern

Thank you for your interest in contributing to Tern.

## Before You Start

Before implementing a new feature or significant change, please open a GitHub issue to discuss it with a maintainer. This helps ensure your contribution aligns with the project's direction and avoids duplicate effort.

Bug fixes and documentation improvements generally don't require prior discussion, but feel free to open an issue if you're unsure.

## Development Setup

### Prerequisites

- Rust toolchain (stable, edition 2024)
- [cargo-make](https://github.com/sagiegurari/cargo-make): `cargo install cargo-make`
- [cargo-nextest](https://nexte.st/): `cargo install cargo-nextest`

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run the CLI
cargo run -- --help
```

### Development Commands

```bash
# Run the full CI pipeline (format check, clippy, build, tests)
cargo make ci-flow

# Run tests only
cargo make test

# Format code
cargo make format

# Run clippy linting
cargo make clippy

# Watch mode for development (rerun tests on file changes)
cargo make bacon
```

### Code Quality Requirements

All contributions must:

- Pass `cargo make format` (rustfmt)
- Pass `cargo make clippy` with no warnings
- Include tests for new functionality
- Pass all existing tests

## Pull Request Process

1. Fork the repository and create a branch for your changes
2. Make your changes, ensuring code quality requirements are met
3. Write clear commit messages that explain the "why" behind changes
4. Open a pull request with a description of what changed and why
5. Address any feedback from maintainers

## Project Structure

```
src/
├── cli/           # Command-line interface (clap)
└── db/
    ├── model/     # In-memory PostgreSQL schema representation
    ├── query/     # Database introspection (Catalog trait)
    ├── diff/      # Schema comparison and rename detection
    ├── migrate/   # Migration planning and SQL rendering
    ├── compile/   # WebAssembly component generation
    ├── state/     # Migration history tracking
    └── history/   # Migration history application
```

## Architecture Notes

Tern follows a **sans-I/O** design pattern. The `Catalog` trait in `src/db/query/catalog.rs` abstracts database queries, allowing business logic to be tested without a live database. When adding new functionality:

- Keep I/O operations behind trait abstractions
- Write unit tests using `FakeCatalog` where possible
- Integration tests that require a database should be clearly marked

## Getting Help

If you have questions about contributing, feel free to open a GitHub issue.
