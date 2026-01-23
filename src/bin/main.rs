use clap::{CommandFactory, Parser};
use tern::cli::Cli;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    dispatch_command(cli).await
}

/// Prints abbreviated help showing only core commands.
///
/// When the user runs `tern` without arguments, we show a shorter help
/// that highlights the most commonly used commands. The full command list
/// is available via `tern --help`.
fn print_short_help() -> miette::Result<()> {
    let cmd = Cli::command();
    let version = cmd.get_version().unwrap_or("unknown");

    println!("tern {version}");
    println!("A database migration tool written in Rust");
    println!();
    println!("Usage: tern [OPTIONS] <COMMAND>");
    println!();
    println!("Commands:");
    println!("  version  Print the CLI version and exit");
    println!("  init     Initialize a new Tern project with state backend");
    println!("  status   Show state backend status");
    println!("  help     Print this message or the help of the given subcommand(s)");
    println!();
    println!("Options:");
    println!("  -h, --help     Print help (use 'tern --help' for all commands)");
    println!("  -V, --version  Print version");

    Ok(())
}

async fn dispatch_command(cli: Cli) -> miette::Result<()> {
    match &cli.cmd {
        None => print_short_help(),
        Some(cmd) => cmd.clone().dispatch().await,
    }
}
