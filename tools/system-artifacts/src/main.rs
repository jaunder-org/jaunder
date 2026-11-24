//! Build the system-theme artifacts consumed independently by the CSR shell and server.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

/// Compile and stage the closed system-theme inventory independently of CSR output.
#[derive(Parser)]
#[command(name = "system-artifacts", version)]
struct Cli {
    /// Output directory for the staged inventory and packaged assets.
    #[arg(long)]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let inventory = host::system_theme::compile_system_artifact_inventory()
        .context("compiling closed system styling inventory")?;
    host::system_theme::stage_system_artifact_inventory(&inventory, &cli.out)
        .context("staging closed system styling inventory")
}
