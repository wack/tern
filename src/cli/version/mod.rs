//! Version command.
//!
//! This command prints the CLI version and exits.

use anstream::println;
use clap::Args;

/// Print the CLI version and exit.
#[derive(Debug, Clone, Args)]
pub struct Version;

impl Version {
    /// Dispatch the version command.
    pub async fn dispatch(self) -> miette::Result<()> {
        println!("tern {}", env!("CARGO_PKG_VERSION"));
        Ok(())
    }
}
