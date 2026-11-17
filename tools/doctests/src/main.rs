//! The doctest producer that runs inside its Nix derivation.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "doctests", about = "Jaunder doctest producer", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the workspace doctests and emit the reconciliation status.
    Emit {
        /// Directory to write emitted artifacts into (defaults to CWD).
        #[arg(long, default_value = ".")]
        out: String,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Emit { out } => doctests::emit::run(&out),
    }
}
