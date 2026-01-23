use clap::{CommandFactory, Parser};
use tern::cli::Cli;

#[tokio::main]
async fn main() -> miette::Result<()> {
    let cli = Cli::parse();
    dispatch_command(cli).await
}

async fn dispatch_command(cli: Cli) -> miette::Result<()> {
    match cli.cmd {
        None => {
            Cli::command().print_help().ok();
            Ok(())
        }
        Some(cmd) => cmd.dispatch().await,
    }
}
