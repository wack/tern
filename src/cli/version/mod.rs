//! Version command.
//!
//! This command prints the CLI version and exits.

use clap::Args;

use crate::output;

/// Print the CLI version and exit.
#[derive(Debug, Clone, Args)]
pub struct Version;

impl Version {
    /// Dispatch the version command.
    pub async fn dispatch(self) -> miette::Result<()> {
        output!("tern {}", env!("CARGO_PKG_VERSION"));
        Ok(())
    }
}
