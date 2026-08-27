//! The `doctest-fences` gate: the half of the doctest population that lives
//! outside every Nix check (#763).
//!
//! `xtask/` is excluded from the flake `src` filter (`flake.nix`'s `cleanSourceWith`
//! drops `/xtask/`) and `tools/` is a separate virtual workspace, so the `doctests`
//! derivation's `cargo test --workspace --doc` reaches neither. This step runs each
//! one's doctests directly and reconciles them against the same scanner.
//!
//! `host_tests` covers the auxiliary workspaces' library, binary, and integration
//! targets without `--doc`; this step owns their two captured doctest executions
//! and all reconciliation. Like `host_tests` it runs in **every** mode — `--no-test`
//! skips only the Nix half. That asymmetry is deliberate: this half needs no Nix
//! build, so there is nothing to skip.
//!
//! The scan roots live in `doctests::roots`, shared with the producer, so the
//! population this step asserts over cannot drift from the one `devtool` scans.

use std::path::{Path, PathBuf};

use doctests::check::{ScannedFile, problems};
use doctests::roots;

use crate::result::{CommandResult, StepResult};

/// A repo-relative path as the runner prints it for a `--manifest-path <root>` run:
/// relative to the manifest's directory, so `xtask/src/steps/nix.rs` prints as
/// `src/steps/nix.rs`.
fn run_path(root: &str, path: &str) -> String {
    path.strip_prefix(root)
        .map_or(path, |rest| rest.trim_start_matches('/'))
        .to_string()
}

/// Paths derived from the Git top-level so an invocation beneath the repository
/// cannot change either the scanned population or Cargo's addressed manifests.
struct HostRoot {
    source: PathBuf,
    manifest: PathBuf,
    run_root: &'static str,
}

fn host_roots(start: &Path) -> anyhow::Result<(PathBuf, Vec<HostRoot>)> {
    let top = PathBuf::from(crate::git::toplevel(start)?);
    let roots = roots::HOST
        .iter()
        .map(|&run_root| HostRoot {
            source: top.join(run_root),
            manifest: top.join(run_root).join("Cargo.toml"),
            run_root,
        })
        .collect();
    Ok((top, roots))
}

fn doctest_args(manifest: &Path) -> Vec<String> {
    vec![
        "test".to_string(),
        "--manifest-path".to_string(),
        manifest.display().to_string(),
        "--doc".to_string(),
    ]
}

/// Run one root's doctests, capturing combined output and whether it succeeded.
///
/// The exit status is **not** discarded. For a root that has fences, a broken
/// build shows up anyway (no `test …` lines, so every fence reads as `NotRun`);
/// but for a root with *zero* fences — `tools/` today — a `cargo test --doc` that
/// fails outright would otherwise produce zero violations and a green step.
fn run_doctests(manifest: &Path) -> std::io::Result<(String, bool)> {
    let args = doctest_args(manifest);
    let out = std::process::Command::new("cargo").args(args).output()?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((combined, out.status.success()))
}

fn violation_detail(v: &doctests::check::Violation) -> String {
    let location = v
        .line
        .map_or_else(|| v.file.clone(), |line| format!("{}:{line}", v.file));
    format!("{location} [{}] {}", kind_str(v.kind), v.detail)
}

/// Scan and reconcile both host roots, pushing one `doctest-fences` step.
pub fn run(result: &mut CommandResult) {
    let mut violations = Vec::new();
    let mut hard_errors = Vec::new();

    // `git ls-files`, not a filesystem walk. A walk descends into `xtask/target/`
    // and `tools/target/`, where build scripts emit generated `.rs` — so the
    // population would depend on whether the machine had built recently, and a
    // dependency bump could put a third party's doc fence (or a file `syn` cannot
    // parse) into a gate the developer has no way to satisfy.
    let (top, roots) = match host_roots(Path::new(".")) {
        Ok(paths) => paths,
        Err(e) => {
            result.push(
                StepResult::fail("doctest-fences")
                    .detail(format!("cannot enumerate tracked sources: {e}")),
            );
            return;
        }
    };
    let tracked = match crate::git::tracked_files(&top, "*.rs") {
        Ok(tracked) => tracked,
        Err(e) => {
            result.push(
                StepResult::fail("doctest-fences")
                    .detail(format!("cannot enumerate tracked sources: {e}")),
            );
            return;
        }
    };

    for root in roots {
        let prefix = format!(
            "{}/",
            root.source
                .strip_prefix(&top)
                .expect("host root derived from Git top-level")
                .display()
        );
        let paths: Vec<&String> = tracked.iter().filter(|p| p.starts_with(&prefix)).collect();
        if paths.is_empty() {
            // A root that yields nothing is a root that moved: silently scanning
            // an empty population is the one way this gate must never be green.
            hard_errors.push(format!(
                "scan root {} matched no tracked .rs files",
                root.run_root
            ));
            continue;
        }
        let mut scanned = Vec::with_capacity(paths.len());
        for path in paths {
            let source_path = root.source.join(
                path.strip_prefix(&prefix)
                    .expect("tracked path matched host root prefix"),
            );
            match std::fs::read_to_string(source_path) {
                Ok(source) => scanned.push(ScannedFile {
                    run_path: run_path(root.run_root, path),
                    path: path.clone(),
                    source,
                }),
                // Same reasoning as an unparseable file: an unread file is
                // invisible to this gate, so it fails rather than being dropped.
                Err(e) => hard_errors.push(format!(
                    "{path}: cannot read: {e} — an unread file is invisible to this gate."
                )),
            }
        }
        match run_doctests(&root.manifest) {
            Ok((output, ok)) => {
                if !ok && doctests::libtest::run_entries(&output).is_empty() {
                    hard_errors.push(format!(
                        "{} doctests exited non-zero and reported no tests — the run \
                         failed before any fence was evaluated.",
                        root.run_root
                    ));
                }
                violations.extend(problems(&scanned, &output));
            }
            Err(e) => hard_errors.push(format!("cannot run {} doctests: {e}", root.run_root)),
        }
    }

    if hard_errors.is_empty() && violations.is_empty() {
        result.push(StepResult::ok("doctest-fences"));
        return;
    }
    let mut lines = hard_errors;
    lines.extend(violations.iter().map(violation_detail));
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
    use crate::test_support::{commit, temp_repo};

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
    fn host_doctest_commands_are_exactly_one_per_auxiliary_workspace() {
        let manifests = [
            Path::new("/repo/xtask/Cargo.toml"),
            Path::new("/repo/tools/Cargo.toml"),
        ];
        let commands: Vec<Vec<String>> = manifests
            .iter()
            .map(|manifest| doctest_args(manifest))
            .collect();

        assert_eq!(commands.len(), 2);
        assert!(
            commands
                .iter()
                .all(|args| args.iter().filter(|arg| *arg == "--doc").count() == 1)
        );
        assert_eq!(
            commands
                .iter()
                .map(|args| args[2].as_str())
                .collect::<Vec<_>>(),
            ["/repo/xtask/Cargo.toml", "/repo/tools/Cargo.toml"]
        );
    }

    #[test]
    fn host_roots_are_absolute_when_invoked_beneath_the_repository_root() {
        let repo = temp_repo("doctest-fences", "outside-root");
        commit(&repo, "xtask/Cargo.toml", "[workspace]\n");
        commit(&repo, "xtask/src/lib.rs", "");
        commit(&repo, "tools/Cargo.toml", "[workspace]\n");
        commit(&repo, "tools/devtool/src/lib.rs", "");

        let (top, roots) = host_roots(&repo.join("xtask/src")).expect("resolve fixture top");

        assert_eq!(
            std::fs::canonicalize(&top).expect("canonical top"),
            std::fs::canonicalize(&repo).expect("canonical fixture root")
        );
        assert_eq!(roots.len(), 2);
        assert_eq!(
            std::fs::read_to_string(roots[0].source.join("src/lib.rs")).expect("absolute source"),
            ""
        );
        assert_eq!(roots[0].source, top.join("xtask"));
        assert_eq!(roots[0].manifest, top.join("xtask/Cargo.toml"));
        assert_eq!(roots[1].source, top.join("tools"));
        assert_eq!(roots[1].manifest, top.join("tools/Cargo.toml"));
        assert!(roots.iter().all(|root| root.manifest.is_absolute()));

        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn violation_detail_omits_missing_source_line() {
        let numeric = doctests::check::Violation {
            file: "tools/doctests/src/check.rs".to_string(),
            line: Some(56),
            kind: doctests::check::Kind::NotRun,
            detail: "not evaluated".to_string(),
        };
        let missing = doctests::check::Violation {
            line: None,
            ..numeric.clone()
        };

        assert_eq!(
            violation_detail(&numeric),
            "tools/doctests/src/check.rs:56 [not-run] not evaluated"
        );
        assert_eq!(
            violation_detail(&missing),
            "tools/doctests/src/check.rs [not-run] not evaluated"
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
