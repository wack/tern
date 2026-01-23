# CLI Module Style Guide

## Command Organization

Each subcommand of `tern` (e.g., `tern verify`) is organized as follows:

1. **Directory**: Create a directory matching the command name under `src/cli/` (e.g., `src/cli/verify/`)

2. **Arguments Struct**: Define the command's arguments in a struct within `mod.rs`, decorated with `clap::Args`:
   ```rust
   #[derive(Debug, Clone, clap::Args)]
   pub struct Verify {
       #[arg(long)]
       pub some_flag: bool,
       // ...
   }
   ```

3. **Dispatch Method**: Each arguments struct must implement a `dispatch()` method that contains the full command implementation:
   ```rust
   impl Verify {
       /// Dispatch the verify command.
       pub async fn dispatch(self) -> miette::Result<()> {
           // Full implementation here using self.* fields
           let backend = load_backend(self.path.as_deref());
           ensure_backend_initialized(&backend).await?;

           // ... rest of implementation

           Ok(())
       }
   }
   ```

4. **Helper Functions**: Keep reusable helper functions (like `confirm_destructive_changes()`, `find_migration()`, `parse_migration_id()`) as separate freestanding functions within the same module:
   ```rust
   /// Helper function for user confirmation.
   fn confirm_destructive_changes() -> miette::Result<bool> {
       // ...
   }
   ```

5. **Enum Variant**: Reference the struct as a tuple variant in `CliCommand` (in `src/cli/mod.rs`):
   ```rust
   #[derive(Debug, Subcommand, Clone)]
   pub enum CliCommand {
       /// Command description
       Verify(verify::Verify),
       // ...
   }
   ```

6. **Enum Dispatch**: The `CliCommand::dispatch` method simply delegates to the wrapped type's `dispatch()` method:
   ```rust
   impl CliCommand {
       pub async fn dispatch(self) -> miette::Result<()> {
           match self {
               CliCommand::Verify(args) => args.dispatch().await,
               // ...
           }
       }
   }
   ```

This pattern:
- Keeps command definitions close to their implementations
- Makes the `CliCommand` enum dispatch trivial (just delegation)
- Ensures no business logic lives in `src/cli/mod.rs`
- Makes each command self-contained and testable

## Nested Subcommands

For nested subcommands (e.g., `tern schema export`), the same pattern applies recursively:

1. **Directory Structure**: Create nested directories matching the command hierarchy:
   ```
   src/cli/schema/           # Parent command
   src/cli/schema/export/    # Nested subcommand
   src/cli/schema/diff/      # Nested subcommand
   src/cli/schema/migrate/   # Nested subcommand
   ```

2. **Arguments Struct with Dispatch**: Each nested subcommand has its own struct with a `dispatch()` method containing the full implementation:
   ```rust
   // src/cli/schema/export/mod.rs
   #[derive(Debug, Clone, clap::Args)]
   pub struct Export {
       #[arg(short, long)]
       pub output: Option<PathBuf>,
       // ...
   }

   impl Export {
       /// Dispatch the schema export command.
       pub async fn dispatch(self) -> miette::Result<()> {
           let backend = load_backend(self.path.as_deref());
           ensure_backend_initialized(&backend).await?;

           // Full implementation using self.output, self.path, etc.

           Ok(())
       }
   }
   ```

3. **Parent Enum**: The parent command defines a subcommand enum using tuple variants:
   ```rust
   #[derive(Debug, Subcommand, Clone)]
   pub enum SchemaAction {
       /// Export the current schema as SQL DDL
       Export(schema::export::Export),
       /// Show diff between current state and edited schema.sql
       Diff(schema::diff::Diff),
       // ...
   }
   ```

4. **Parent Dispatch**: The parent enum's dispatch method delegates to each nested command's `dispatch()`:
   ```rust
   impl SchemaAction {
       pub async fn dispatch(self) -> miette::Result<()> {
           match self {
               SchemaAction::Export(args) => args.dispatch().await,
               SchemaAction::Diff(args) => args.dispatch().await,
               // ...
           }
       }
   }
   ```

## Deprecated Commands

For deprecated commands, include the deprecation warning at the start of the `dispatch()` method:

```rust
impl Compile {
    /// Dispatch the compile command.
    pub async fn dispatch(self) -> miette::Result<()> {
        anstream::eprintln!(
            "WARNING: 'compile' is deprecated. Use 'tern import' + 'tern build' instead."
        );

        // Full implementation follows...
        let backend = load_backend(self.path.as_deref());
        // ...
    }
}
```

Also mark the command as hidden in the enum:
```rust
#[derive(Debug, Subcommand, Clone)]
pub enum CliCommand {
    /// [DEPRECATED] Compile a migration to source code
    #[command(hide = true)]
    Compile(compile::Compile),
    // ...
}
```

## Testing Commands

When writing tests for commands, use the struct-based approach rather than calling helper functions:

```rust
#[tokio::test]
async fn test_show_migration() {
    let temp_dir = TempDir::new().unwrap();
    // ... setup ...

    let show = Show {
        migration_id: baseline_id,
        format: OutputFormat::Text,
        path: Some(temp_dir.path().to_path_buf()),
    };
    show.dispatch().await.unwrap();
}
```

This approach:
- Tests the actual command interface users will use
- Ensures argument parsing and dispatch work together
- Makes tests more representative of real usage
