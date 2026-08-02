//! The `doctest-fences` gate: the half of the doctest population that lives
//! outside every Nix check (#763).
//!
//! `xtask/` is excluded from the flake `src` filter (`flake.nix`'s `cleanSourceWith`
//! drops `/xtask/`) and `tools/` is a separate virtual workspace, so the `doctests`
//! derivation's `cargo test --workspace --doc` reaches neither. This step runs each
//! one's doctests directly and reconciles them against the same scanner.
//!
//! `host_tests`'s plain `cargo test --manifest-path …` also *executes* these
//! doctests, but discards the output; this step is the only thing that
//! *reconciles* them, so no reconciliation is duplicated. Like `host_tests` it runs
//! in **every** mode — `--no-test` skips only the Nix half. That asymmetry is
//! deliberate: this half needs no Nix build, so there is nothing to skip.
//!
//! The scan roots live in `doctests::roots`, shared with the producer, so the
//! population this step asserts over cannot drift from the one `devtool` scans.

use std::path::Path;

use doctests::check::{problems, ScannedFile};
use doctests::roots;

use crate::files;
use crate::result::{CommandResult, StepResult};

/// A repo-relative path as the runner prints it for a `--manifest-path <root>` run:
/// relative to the manifest's directory, so `xtask/src/steps/nix.rs` prints as
/// `src/steps/nix.rs`.
fn run_path(root: &str, path: &str) -> String {
    path.strip_prefix(root)
        .map_or(path, |rest| rest.trim_start_matches('/'))
        .to_string()
}

/// Run one root's doctests, capturing combined output.
fn run_doctests(root: &str) -> std::io::Result<String> {
    let out = std::process::Command::new("cargo")
        .args([
            "test",
            "--manifest-path",
            &format!("{root}/Cargo.toml"),
            "--doc",
        ])
        .output()?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(combined)
}

/// Scan and reconcile both host roots, pushing one `doctest-fences` step.
pub fn run(result: &mut CommandResult) {
    let mut violations = Vec::new();
    let mut hard_errors = Vec::new();

    for root in roots::HOST {
        let files = match files::with_extension(Path::new(root), "rs") {
            Ok(files) => files,
            Err(e) => {
                // A root that cannot be enumerated is a root the gate cannot
                // police; it fails rather than shrinking the population.
                hard_errors.push(format!("cannot scan {root}: {e}"));
                continue;
            }
        };
        let mut scanned = Vec::with_capacity(files.len());
        for p in &files {
            let path = p.display().to_string();
            match std::fs::read_to_string(p) {
                Ok(source) => scanned.push(ScannedFile {
                    run_path: run_path(root, &path),
                    path,
                    source,
                }),
                // Same reasoning as an unparseable file: an unread file is
                // invisible to this gate, so it fails rather than being dropped.
                Err(e) => hard_errors.push(format!(
                    "{path}: cannot read: {e} — an unread file is invisible to this gate."
                )),
            }
        }
        match run_doctests(root) {
            Ok(output) => violations.extend(problems(&scanned, &output)),
            Err(e) => hard_errors.push(format!("cannot run {root} doctests: {e}")),
        }
    }

    if hard_errors.is_empty() && violations.is_empty() {
        result.push(StepResult::ok("doctest-fences"));
        return;
    }
    let mut lines = hard_errors;
    lines.extend(
        violations
            .iter()
            .map(|v| format!("{}:{} [{}] {}", v.file, v.line, kind_str(v.kind), v.detail)),
    );
    result.push(StepResult::fail("doctest-fences").detail(lines.join("\n")));
}

/// The kebab-case wire spelling, so this step's message reads the same as the Nix
/// gate's `jq` output rather than `Debug`'s CamelCase.
fn kind_str(kind: doctests::check::Kind) -> String {
    serde_json::to_string(&kind)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rs_file_in_the_repo_falls_under_exactly_one_scan_root() {
        // A file under no root is invisible to the gate; a file under two would be
        // reconciled against the wrong run. Either is a population bug, and this is
        // the assertion that covers shrink vector 4 — a crate outside every root —
        // which no fixture crate can demonstrate.
        //
        // Asked from the toplevel, not the cwd: `cargo test --manifest-path
        // xtask/Cargo.toml` runs with `xtask/` as its cwd, and `ls-files` lists
        // only what is beneath it — so a cwd-relative query would have seen a
        // partial tree and reported every path as unrooted.
        let root = crate::git::toplevel(Path::new(".")).expect("git rev-parse");
        let tracked = crate::git::tracked_files(Path::new(&root), "*.rs").expect("git ls-files");
        assert!(
            tracked.len() > 100,
            "only {} tracked .rs files — the query saw a partial tree",
            tracked.len()
        );
        for path in tracked {
            let n = roots::ALL
                .iter()
                .filter(|r| path.starts_with(&format!("{r}/")))
                .count();
            assert_eq!(n, 1, "{path} falls under {n} scan roots, want exactly 1");
        }
    }

    #[test]
    fn run_paths_are_relative_to_the_invoked_manifest() {
        // `cargo test --manifest-path xtask/Cargo.toml --doc` prints `src/…`.
        assert_eq!(
            run_path("xtask", "xtask/src/steps/nix.rs"),
            "src/steps/nix.rs"
        );
        assert_eq!(
            run_path("tools", "tools/devtool/src/main.rs"),
            "devtool/src/main.rs"
        );
    }

    #[test]
    fn a_path_outside_the_root_is_left_alone() {
        // Defensive: a mis-paired (root, path) must not be silently truncated into
        // something that looks like a valid run path.
        assert_eq!(
            run_path("xtask", "tools/devtool/src/main.rs"),
            "tools/devtool/src/main.rs"
        );
    }

    #[test]
    fn kind_str_is_the_kebab_case_wire_spelling() {
        assert_eq!(kind_str(doctests::check::Kind::NotRun), "not-run");
        assert_eq!(
            kind_str(doctests::check::Kind::BannedAttribute),
            "banned-attribute"
        );
    }
}
