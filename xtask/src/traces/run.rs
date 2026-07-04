//! `cargo xtask traces run` — build the e2e VM checks and analyze their traces.
//!
//! Host orchestrator (ADR-0028): nix-builds the `{sqlite,postgres}×{chromium,firefox}`
//! e2e checks (or their `-cold` package variants), collects each exported
//! `otel-traces.jsonl`, and hands the files to the in-process `traces::analyze` /
//! `render` seam. Port of `scripts/run-e2e-trace-analysis`; the pure helpers below
//! are unit-tested, the nix/filesystem I/O in `collect_trace_files` is manual.

use std::path::PathBuf;

use anyhow::{ensure, Result};

use crate::nix_build::build_out_path;
use crate::{E2eBackend, E2eBrowser};

/// Both backends are always built, regardless of `--browser` (matches the script).
const BACKENDS: [E2eBackend; 2] = [E2eBackend::Sqlite, E2eBackend::Postgres];

/// The flake attr for one e2e combo. `cold` → the `packages…-cold` variant, warm →
/// the `checks…` variant (e.g. `checks.x86_64-linux.e2e-sqlite-chromium`,
/// `packages.x86_64-linux.e2e-postgres-firefox-cold`).
pub fn e2e_attr(backend: E2eBackend, browser: E2eBrowser, cold: bool) -> String {
    let ns = if cold { "packages" } else { "checks" };
    let suffix = if cold { "-cold" } else { "" };
    format!(
        "{ns}.x86_64-linux.e2e-{}-{}{suffix}",
        backend.as_str(),
        browser.as_str()
    )
}

/// The trace artifact inside a built e2e check's store path:
/// `<out>/otel-traces-<backend>.jsonl/otel-traces.jsonl`.
pub fn trace_file_path(out: &str, backend: E2eBackend) -> PathBuf {
    PathBuf::from(out)
        .join(format!("otel-traces-{}.jsonl", backend.as_str()))
        .join("otel-traces.jsonl")
}

/// The browsers to build: just `--browser` if given, else both.
pub fn browsers(browser: Option<E2eBrowser>) -> Vec<E2eBrowser> {
    match browser {
        Some(b) => vec![b],
        None => vec![E2eBrowser::Chromium, E2eBrowser::Firefox],
    }
}

/// Build every combo (both backends × the selected browsers) and collect each
/// trace file, erroring (naming the path) when one is absent. nix/filesystem I/O
/// — not unit-tested; exercised by a manual run. Iteration is backends-outer,
/// browsers-inner, matching the script's file order.
pub fn collect_trace_files(cold: bool, browser: Option<E2eBrowser>) -> Result<Vec<PathBuf>> {
    let browsers = browsers(browser);
    let mut files = Vec::new();
    for backend in BACKENDS {
        for &browser in &browsers {
            let attr = e2e_attr(backend, browser, cold);
            let out = build_out_path(&attr)?;
            let file = trace_file_path(&out, backend);
            ensure!(file.exists(), "trace file not found: {}", file.display());
            files.push(file);
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2e_attr_warm_and_cold() {
        assert_eq!(
            e2e_attr(E2eBackend::Sqlite, E2eBrowser::Chromium, false),
            "checks.x86_64-linux.e2e-sqlite-chromium"
        );
        assert_eq!(
            e2e_attr(E2eBackend::Postgres, E2eBrowser::Firefox, true),
            "packages.x86_64-linux.e2e-postgres-firefox-cold"
        );
    }

    #[test]
    fn trace_file_path_shape() {
        assert_eq!(
            trace_file_path("/nix/store/x", E2eBackend::Sqlite),
            PathBuf::from("/nix/store/x/otel-traces-sqlite.jsonl/otel-traces.jsonl")
        );
    }

    #[test]
    fn browsers_one_or_both() {
        assert_eq!(
            browsers(Some(E2eBrowser::Firefox)),
            vec![E2eBrowser::Firefox]
        );
        assert_eq!(
            browsers(None),
            vec![E2eBrowser::Chromium, E2eBrowser::Firefox]
        );
    }
}
