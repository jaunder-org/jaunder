//! Build the system-theme artifacts consumed independently by the CSR shell and server.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Parser;

/// Compile and stage the closed system-theme inventory independently of CSR output.
#[derive(Parser)]
#[command(name = "system-artifacts", version)]
struct Cli {
    /// Output directory for the staged inventory and packaged assets.
    #[arg(long)]
    out: PathBuf,
    /// Internal qualification-only system styling fixture (`a`, `b-app`, or `b-theme`).
    #[cfg(feature = "qualification")]
    #[arg(long)]
    fixture: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    #[cfg(feature = "qualification")]
    let fixture = cli.fixture.as_deref();
    #[cfg(not(feature = "qualification"))]
    let fixture = None;
    run(&cli.out, fixture)
}

/// Compile and stage the closed system-theme inventory without producing a CSR bundle.
///
/// # Errors
///
/// Returns an error when source themes fail validation or staging fails.
fn run(out: &Path, fixture: Option<&str>) -> anyhow::Result<()> {
    #[cfg(feature = "qualification")]
    let inventory = if let Some(fixture) = fixture {
        let fixture = host::system_theme::qualification::Fixture::parse(fixture)
            .map_err(anyhow::Error::msg)?;
        host::system_theme::qualification::compile(fixture)
    } else {
        host::system_theme::compile_system_artifact_inventory()
    }
    .context("compiling closed system styling inventory")?;

    #[cfg(not(feature = "qualification"))]
    let inventory = {
        anyhow::ensure!(
            fixture.is_none(),
            "system-artifact fixtures require the qualification build"
        );
        host::system_theme::compile_system_artifact_inventory()
            .context("compiling closed system styling inventory")?
    };

    host::system_theme::stage_system_artifact_inventory(&inventory, out)
        .context("staging closed system styling inventory")
}
