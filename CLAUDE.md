# CLAUDE.md

> **IMPORTANT**: This is a production-ready repository, and therefore must adhere to a high bar of professional quality.

## Project Overview

**Tern** is a database migration tool written in Rust. The project is named after the tern bird, known for making the longest migrations of any bird species—a fitting metaphor for a tool that manages database schema migrations.

## Build & Development Commands

### Prerequisites

- Rust toolchain (stable)
- cargo-make: `cargo install cargo-make`
- cargo-nextest: `cargo install cargo-nextest`

### Common Commands

```bash
# Run the full CI pipeline (format check, clippy, build, tests, coverage)
cargo make ci-flow

# Development workflow with formatting
cargo make dev-test-flow

# Run tests only
cargo make test

# Format code
cargo make format

# Check formatting without applying changes
cargo make check-format

# Run clippy linting
cargo make clippy

# Watch mode for tests (rerun on file changes)
cargo make bacon

# Generate CLI reference documentation
cargo make gen-cli-reference
```

### Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run the CLI
cargo run -- --help
```

## Project Structure

```
src/
├── lib.rs              # Library root (exports cli module)
├── bin/
│   └── main.rs         # Binary entry point
└── cli/
    ├── mod.rs          # CLI command definitions using clap
    └── colors.rs       # Color output handling (Auto/Always/Never)
```

## Architecture

- **CLI Framework**: Uses `clap` with derive macros for argument parsing
- **Async Runtime**: `tokio` with full features
- **Error Handling**: `miette` for user-facing diagnostics, `thiserror` for error type definitions
- **Logging**: `tracing` with `tracing-subscriber` (supports text and JSON formats)
- **Serialization**: `serde` for data serialization

## Code Style & Conventions

- Rust Edition 2024
- Code must pass `rustfmt` formatting checks
- Code must pass `clippy` linting with no warnings
- Use `thiserror` for defining error types
- Use `miette` for rich error diagnostics
- Prefer async/await patterns with tokio

## Testing

- Tests are run using `cargo-nextest` (faster than default cargo test)
- Run tests with: `cargo make test`
- Use `pretty_assertions` for enhanced assertion output in tests
- Use `static_assertions` for compile-time checks

## CI/CD

GitHub Actions workflow runs on all pushes (except `trunk` branch):
1. Format check
2. Clippy linting
3. Build
4. Tests
5. Coverage reporting

All CI checks must pass before merging.
