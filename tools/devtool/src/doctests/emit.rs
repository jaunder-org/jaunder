//! Runs the workspace doctests and reconciles them against the scanned fence
//! population, emitting the sentinel the `doctests-gate` derivation reads.
//!
//! Mirrors `coverage::emit`: always writes `out/status.json` and returns `Ok`, so
//! the Nix producer derivation can always realize `$out`. Gating is the consumer.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use doctests::check::{problems, Kind, ScannedFile, Violation};
use doctests::status::DoctestStatus;

/// The roots this producer scans — the shared workspace list, not a local copy, so
/// the population `xtask` asserts over cannot drift from the one scanned here.
pub const SCAN_ROOTS: &[&str] = doctests::roots::WORKSPACE;

/// The doctest invocation.
///
/// `--workspace`, never `-p`. Package-scoping is exactly what made the issue's own
/// measurement wrong: `-p common -p macros --doc` silently drops the three
/// `#[cfg(feature = "sanitize")]` fences in `common/src/render.rs`, because nothing
/// in that package set enables the feature. Under `--workspace`, feature
/// unification enables it via `storage/Cargo.toml`'s
/// `common = { features = ["sqlx", "sanitize"] }` and all three run.
pub fn doctest_command() -> Command {
    let mut c = Command::new("cargo");
    c.args(["test", "--workspace", "--doc"]);
    c
}

/// Every `.rs` file under `root`, recursively, as repo-relative paths.
///
/// An unreadable directory is an error, never a short list: a root the gate cannot
/// enumerate could hide the very fences it exists to police.
fn rs_files(root: &Path, out: &mut Vec<String>) -> std::io::Result<()> {
    if !root.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("scan root {} does not exist", root.display()),
        ));
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            rs_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path.display().to_string());
        }
    }
    Ok(())
}

/// Read every scanned root into `ScannedFile`s, turning any file that cannot be
/// READ into a violation rather than dropping it — an unread file is as invisible
/// to the gate as an unparsed one.
fn collect(roots: &[&str]) -> Result<(Vec<ScannedFile>, Vec<Violation>)> {
    let mut paths = Vec::new();
    for root in roots {
        rs_files(Path::new(root), &mut paths).with_context(|| format!("scanning root {root}"))?;
    }
    paths.sort();

    let mut files = Vec::with_capacity(paths.len());
    let mut unreadable = Vec::new();
    for path in paths {
        match fs::read_to_string(&path) {
            // A workspace run prints repo-relative paths, so both spellings match.
            Ok(source) => files.push(ScannedFile {
                run_path: path.clone(),
                path,
                source,
            }),
            Err(e) => unreadable.push(Violation {
                file: path,
                line: 0,
                kind: Kind::Unreadable,
                detail: format!(
                    "cannot read: {e} — an unread file is invisible to this gate, so it fails \
                     rather than shrinking the population."
                ),
            }),
        }
    }
    Ok((files, unreadable))
}

/// Run the workspace doctests, reconcile, and write `out/status.json` plus the run
/// log. Returns `Err` only if the emit could not run at all.
pub fn run(out: &str) -> Result<()> {
    let out = Path::new(out);
    let diag = out.join("diagnostics");
    fs::create_dir_all(&diag).with_context(|| format!("creating {}", diag.display()))?;

    let status = match emit(&diag) {
        Ok(violations) => DoctestStatus::from_violations(violations),
        Err(e) => DoctestStatus::infra(format!("{e:#}")),
    };
    fs::write(out.join("status.json"), status.to_json())?;
    Ok(())
}

/// The fallible half: run, scan, reconcile. Split out so any failure becomes an
/// `infra` status rather than an unrealized `$out`.
fn emit(diag: &Path) -> Result<Vec<Violation>> {
    let output = doctest_command()
        .output()
        .context("spawning cargo test --workspace --doc")?;
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    fs::write(diag.join("doctests.log"), &log)?;

    let (files, mut violations) = collect(SCAN_ROOTS)?;
    violations.extend(problems(&files, &log));
    Ok(violations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_command_is_workspace_scoped_not_package_scoped() {
        // Asserted on the command `run` actually builds, not on a free helper a
        // divergent `run` could ignore.
        let cmd = doctest_command();
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(cmd.get_program().to_string_lossy(), "cargo");
        assert_eq!(args, vec!["test", "--workspace", "--doc"]);
    }

    #[test]
    fn the_scan_roots_are_the_shared_workspace_list() {
        assert_eq!(SCAN_ROOTS, doctests::roots::WORKSPACE);
        assert!(!SCAN_ROOTS.iter().any(|r| *r == "tools" || *r == "xtask"));
    }

    #[test]
    fn a_missing_scan_root_is_an_error_not_an_empty_list() {
        // A root that moved must fail loudly: silently scanning nothing is the
        // one way this gate must never report green.
        let err = collect(&["no-such-root"]).expect_err("must fail");
        assert!(format!("{err:#}").contains("no-such-root"), "{err:#}");
    }
}
