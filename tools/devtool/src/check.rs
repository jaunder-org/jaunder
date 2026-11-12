//! Retained ERT request timing; shared static invocations belong to Just.

use std::{io::Write, process::Command, time::Instant};

use anyhow::{Context, Result, bail};

/// ERT has no autofix; the existing caller flag is inert.
pub fn run(name: &str, _fix: bool) -> Result<()> {
    if name != "ert" {
        bail!("unknown retained timing check '{name}' (expected ert)");
    }
    let mut command = Command::new("just");
    command.args([
        "--justfile",
        "recipes/static.just",
        "--working-directory",
        ".",
        "ert",
    ]);
    // Preserve the existing Instant/status owner. This fixed request includes
    // Just startup/evaluation; spawn failure produces no fabricated completion.
    let started = Instant::now();
    let status = command.status().context("spawning `ert` for check ert")?;
    let event = serde_json::json!({
        "event": "devtool-check-timing",
        "schema_version": 2,
        "timing_scope": "request",
        "check": "ert",
        "duration_ms": started.elapsed().as_millis(),
        "outcome": if status.success() { "success" } else { "failure" },
    });
    writeln!(std::io::stderr().lock(), "{event}").context("writing ert timing event")?;
    if !status.success() {
        bail!("check ert failed ({status})");
    }
    Ok(())
}
