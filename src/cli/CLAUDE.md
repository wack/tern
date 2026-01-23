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
