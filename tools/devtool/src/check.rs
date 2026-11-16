//! Retained ERT request timing; shared static invocations belong to `dev`.

use std::{io::Write, process::Command, time::Instant};

use anyhow::{Context, Result, bail};

/// ERT has no autofix; the existing caller flag is inert.
pub fn run(name: &str, _fix: bool) -> Result<()> {
    if name != "ert" {
        bail!("unknown retained timing check '{name}' (expected ert)");
    }
    // `--sandbox` runs ERT in place, so the request works from the checkout
    // root `dev static all` starts it in and from a Nix build directory alike.
    let mut command = Command::new("dev");
    command.args(["static", "ert", "--sandbox"]);
    // Preserve the existing Instant/status owner. This fixed request includes
    // `dev` startup; spawn failure produces no fabricated completion.
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
