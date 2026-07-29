//! The e2e lane of the `#[server]` flow-coverage gate (#681).
//!
//! Only a real run produces traces, so only here can the gate notice that the
//! committed snapshot no longer matches what the suite actually exercises.
//! `regenerate` rewrites the snapshot from a capture; `verify` fails on any
//! difference from the committed one.
//!
//! Per the spec's D8 this runs on the per-combo `cargo xtask e2e sqlite chromium`
//! path **only**, never from the aggregate `checks.e2e` join, where both sqlite
//! combos emit a `capture-sqlite.tar.gz` and collide unpredictably.
//!
//! The complementary **static lane** — snapshot + allowlist + syn inventory →
//! `verdict`, with no capture, so it runs in `check` / `validate --no-e2e` — lands
//! in Task 9 together with the committed artifacts, since it cannot be green (nor
//! even reachable, under `-D dead-code`) until they exist. Neither half is
//! sufficient alone: traces exist only in the e2e lane, and fast feedback only in
//! the static one.

use std::path::Path;

use anyhow::Result;

use crate::result::StepResult;
use crate::server_fn_coverage::io::{
    coverage_from_capture, inventory, write_snapshot, SNAPSHOT_PATH, WEB_SRC,
};
use crate::server_fn_coverage::{render, Snapshot, REGENERATE_CMD};

const REGENERATE_STEP: &str = "server-fn-coverage-regenerate";
const VERIFY_STEP: &str = "server-fn-coverage-verify";

/// The e2e lane's core, over explicit paths so it is testable without the repo.
///
/// Comparison is on the rendered bytes rather than the parsed value, so the
/// committed file is checked to be exactly what regeneration would produce —
/// a hand-edit that happens to parse equal is still drift.
fn regenerate_or_verify(
    web_src: &Path,
    capture: &Path,
    snapshot_path: &Path,
    regenerate: bool,
) -> Result<StepResult> {
    let name = if regenerate {
        REGENERATE_STEP
    } else {
        VERIFY_STEP
    };
    let inventory = inventory(web_src)?;
    let coverage = coverage_from_capture(capture, &inventory)?;
    let snapshot = Snapshot::from(coverage);
    let covered = snapshot.covered.len();

    if regenerate {
        let orphans: usize = snapshot.orphans.values().sum();
        write_snapshot(snapshot_path, &snapshot)?;
        return Ok(StepResult::ok(name).detail(format!(
            "{covered} covered, {orphans} orphan hit(s) → {}",
            snapshot_path.display()
        )));
    }

    // A missing snapshot reads as empty, so it mismatches and fails — the strict
    // reading, not a lenient one.
    let committed = std::fs::read_to_string(snapshot_path).unwrap_or_default();
    if committed == render(&snapshot) {
        return Ok(StepResult::ok(name).detail(format!("{covered} covered; snapshot current")));
    }
    Ok(StepResult::fail(name).detail(format!(
        "{} is out of date with this run's traces — regenerate it with `{REGENERATE_CMD}` and \
         commit the result",
        snapshot_path.display()
    )))
}

/// Derive coverage from an e2e capture over the repo's real roots, and either
/// rewrite the committed snapshot (`regenerate`) or fail on any difference from
/// it (`verify`).
pub fn from_capture(capture: &Path, regenerate: bool) -> Result<StepResult> {
    regenerate_or_verify(
        Path::new(WEB_SRC),
        capture,
        Path::new(SNAPSHOT_PATH),
        regenerate,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `web/src`-shaped tree with one `#[server]` fn per ident given.
    fn web_src_with(dir: &Path, idents: &[&str]) {
        let src: String = idents
            .iter()
            .map(|i| format!("#[server(endpoint = \"/{i}\")]\npub async fn {i}() {{}}\n"))
            .collect();
        std::fs::write(dir.join("lib.rs"), src).expect("write source");
    }

    #[test]
    fn verify_from_a_missing_capture_is_an_error() {
        // Not `Ok(fail)`: a broken capture must reach the exit-2 path, so it can
        // never be confused with "the suite covers everything".
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let err = regenerate_or_verify(
            tmp.path(),
            Path::new("/nonexistent-capture.tar.gz"),
            &tmp.path().join("snap.json"),
            false,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("capture"), "{err:#}");
    }

    #[test]
    fn an_unscannable_web_src_is_an_error_not_an_empty_inventory() {
        // A moved/renamed `web/src` must not quietly derive "no fns, all covered".
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = regenerate_or_verify(
            &tmp.path().join("nonexistent"),
            Path::new("/nonexistent-capture.tar.gz"),
            &tmp.path().join("snap.json"),
            false,
        )
        .unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("nonexistent"), "{chain}");
        assert!(chain.contains("#[server]"), "{chain}");
    }
}
