//! `cargo xtask traces run` — build the e2e VM checks and analyze their traces.
//!
//! Host orchestrator (ADR-0028): nix-builds the `{sqlite,postgres}×{chromium,firefox}`
//! e2e checks (or their `-cold` package variants), extracts each combo's
//! `capture/otel-traces.jsonl` from its `capture-<backend>.tar.gz` bundle (#332 — the
//! trace now rides the capture dir, not a standalone artifact), and hands the files to
//! the in-process `traces::analyze` / `render` seam. Port of
//! `scripts/run-e2e-trace-analysis`; the pure helpers below are unit-tested, the
//! nix/filesystem I/O in `collect_trace_files` is manual.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use flate2::read::GzDecoder;
use tempfile::TempDir;

use crate::nix_build::build_out_path;
use crate::{E2eBackend, E2eBrowser};

/// Both backends are always built, regardless of `--browser` (matches the script).
const BACKENDS: [E2eBackend; 2] = [E2eBackend::Sqlite, E2eBackend::Postgres];

/// The trace member inside every `capture-<backend>.tar.gz` — the collector writes
/// `capture/otel-traces.jsonl` under the capture dir, and the tarball is rooted at the
/// capture dir's parent (`tar … -C /var/lib/jaunder capture`).
const TRACE_MEMBER: &str = "capture/otel-traces.jsonl";

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

/// The capture bundle inside a built e2e check's store path:
/// `<out>/capture-<backend>.tar.gz` (holds `capture/otel-traces.jsonl`, #332).
pub fn capture_tarball_path(out: &str, backend: E2eBackend) -> PathBuf {
    PathBuf::from(out).join(format!("capture-{}.tar.gz", backend.as_str()))
}

/// The browsers to build: just `--browser` if given, else both.
pub fn browsers(browser: Option<E2eBrowser>) -> Vec<E2eBrowser> {
    match browser {
        Some(b) => vec![b],
        None => vec![E2eBrowser::Chromium, E2eBrowser::Firefox],
    }
}

/// Build every combo (both backends × the selected browsers), extract each
/// `capture/otel-traces.jsonl` from its `capture-<backend>.tar.gz` bundle to a
/// per-combo file in one `TempDir`, and collect the extracted files — erroring
/// (naming the tarball) when one is absent. The returned `TempDir` guards the
/// extracted files and MUST outlive analysis (the caller binds it); the per-combo
/// file name (`<backend>-<browser>-otel-traces.jsonl`) disambiguates the shared inner
/// member across combos. nix/filesystem I/O — not unit-tested; exercised by a manual
/// run. Iteration is backends-outer, browsers-inner, matching the script's file order.
pub fn collect_trace_files(
    cold: bool,
    browser: Option<E2eBrowser>,
) -> Result<(TempDir, Vec<PathBuf>)> {
    let tmp = TempDir::new()?;
    let browsers = browsers(browser);
    let mut files = Vec::new();
    for backend in BACKENDS {
        for &browser in &browsers {
            let attr = e2e_attr(backend, browser, cold);
            let out = build_out_path(&attr)?;
            let tarball = capture_tarball_path(&out, backend);
            ensure!(
                tarball.exists(),
                "capture tarball not found: {}",
                tarball.display()
            );
            let dest = tmp.path().join(format!(
                "{}-{}-otel-traces.jsonl",
                backend.as_str(),
                browser.as_str()
            ));
            extract_trace(&tarball, &dest)?;
            files.push(dest);
        }
    }
    Ok((tmp, files))
}

/// Extract the single `capture/otel-traces.jsonl` member of a `capture-*.tar.gz`
/// bundle to `dest`, via the `tar` + `flate2` crates (matching `storage::backup`'s
/// archive I/O rather than shelling out `tar`). Errors, naming the tarball, if the
/// member is absent.
fn extract_trace(tarball: &Path, dest: &Path) -> Result<()> {
    let file = File::open(tarball).with_context(|| format!("opening {}", tarball.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.path()?.as_ref() == Path::new(TRACE_MEMBER) {
            entry.unpack(dest)?;
            return Ok(());
        }
    }
    bail!(
        "otel trace member `{TRACE_MEMBER}` missing from {}",
        tarball.display()
    );
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
    fn capture_tarball_path_shape() {
        assert_eq!(
            capture_tarball_path("/nix/store/x", E2eBackend::Sqlite),
            PathBuf::from("/nix/store/x/capture-sqlite.tar.gz")
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
