//! Migrate command with up/down subcommands.
//!
//! This module provides the `tern migrate` command which offers precise control
//! over database migrations.

pub mod down;
pub mod up;

pub use down::Down;
pub use up::Up;

use clap::Subcommand;

/// Migrate subcommands for precise migration control.
#[derive(Debug, Subcommand, Clone)]
pub enum MigrateAction {
    /// Run pending migrations against a database
    ///
    /// Connects to a database and applies all migrations that haven't been
    /// applied yet. Each migration runs in its own transaction.
    Up(Up),

    /// Revert the most recently applied migration
    ///
    /// Connects to a database and reverts the last applied migration.
    /// Only one migration is reverted at a time for safety.
    Down(Down),
}

impl MigrateAction {
    /// Dispatch migrate subcommands.
    pub async fn dispatch(self) -> miette::Result<()> {
        match self {
            MigrateAction::Up(args) => args.dispatch().await,
            MigrateAction::Down(args) => args.dispatch().await,
        }
    }
}
