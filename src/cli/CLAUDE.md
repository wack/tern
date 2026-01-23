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

3. **Enum Variant**: Reference the struct as a tuple variant in `CliCommand` (in `src/cli/mod.rs`):
   ```rust
   #[derive(Debug, Subcommand, Clone)]
   pub enum CliCommand {
       /// Command description
       Verify(verify::Verify),
       // ...
   }
   ```

4. **Dispatch**: Handle the tuple variant in the `dispatch` method:
   ```rust
   CliCommand::Verify(args) => {
       verify::run_verify(&args.database_url, args.format).await
   }
   ```

This pattern keeps command definitions close to their implementations and makes the `CliCommand` enum more concise.

## Nested Subcommands

For nested subcommands (e.g., `tern schema export`), the same pattern applies recursively:

1. **Directory Structure**: Create nested directories matching the command hierarchy:
   ```
   src/cli/schema/           # Parent command
   src/cli/schema/export/    # Nested subcommand
   src/cli/schema/diff/      # Nested subcommand
   src/cli/schema/migrate/   # Nested subcommand
   ```

2. **Arguments Struct**: Each nested subcommand has its own struct in its `mod.rs`:
   ```rust
   // src/cli/schema/export/mod.rs
   #[derive(Debug, Clone, clap::Args)]
   pub struct Export {
       #[arg(short, long)]
       pub output: Option<PathBuf>,
       // ...
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

4. **Dispatch**: The parent enum's dispatch method delegates to the nested commands:
   ```rust
   impl SchemaAction {
       pub async fn dispatch(self) -> miette::Result<()> {
           match self {
               SchemaAction::Export(args) => {
                   schema::run_schema_export(args.output, args.path.as_deref(), args.format).await
               }
               // ...
           }
       }
   }
   ```
