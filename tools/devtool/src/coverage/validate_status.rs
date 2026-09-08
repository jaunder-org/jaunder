use std::path::Path;

use anyhow::{Context, Result};
use coverage::status::CoverageStatus;

pub fn run(status_path: &Path) -> Result<()> {
    let raw = std::fs::read_to_string(status_path)
        .with_context(|| format!("reading coverage status at {}", status_path.display()))?;
    CoverageStatus::from_completed_json(&raw).context("validating completed coverage status")?;
    println!("coverage status valid: tests-ok");
    Ok(())
}
