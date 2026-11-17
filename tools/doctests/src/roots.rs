//! The scan roots, in one place so the population the gate *checks* cannot drift
//! from the population it *scans*: `devtool` scans [`WORKSPACE`] inside the Nix
//! producer, and this module's tests assert that every maintained `.rs` file
//! has exactly one owner. Imported dependency sources have separate ownership;
//! Cargo's workspace doctest runs do not execute them.
//!
//! A gate quietly shrinking its own reach is the failure ADR-0085 principle 6
//! names.

/// Root-workspace directories the `cargo test --workspace --doc` run covers.
///
/// Explicit rather than "every `.rs` in the derivation source": the source also
/// carries `tools/`, which the workspace run does not reach — scanning it there
/// would manufacture `NotRun` violations for fences no reconciled run covers.
pub const WORKSPACE: &[&str] = &[
    "client",
    "common",
    "csr",
    "host",
    "macros",
    "server",
    "storage",
    "test-support",
    "ux-sandbox",
    "web",
];

/// Maintained source deliberately outside the fence population. `tools/` is a
/// separate virtual workspace with no doctests, so `checks.tools-doctests` runs
/// its `cargo test --doc` without reconciling fences.
pub const OUTSIDE_FENCE_POPULATION: &[&str] = &["tools"];

/// Imported dependency source, not maintained workspace doctest targets.
///
/// Keep this exact rather than excluding all of `vendor/`: an unclassified
/// crate must still fail the repository ownership census. Provenance and removal
/// are documented in `docs/sqlx-sqlite-patch.md`.
pub const VENDORED_DEPENDENCY_SOURCES: &[&str] = &["vendor/sqlx-sqlite-0.9.0/src"];

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Every owner of tracked Rust source; together they must account for each
    /// tracked `.rs` file exactly once.
    fn owners() -> impl Iterator<Item = &'static str> {
        WORKSPACE
            .iter()
            .chain(OUTSIDE_FENCE_POPULATION)
            .chain(VENDORED_DEPENDENCY_SOURCES)
            .copied()
    }

    fn owner_count(path: &str) -> usize {
        owners()
            .filter(|root| path.starts_with(&format!("{root}/")))
            .count()
    }

    /// `git` at the repository top level, with a hook's inherited repository
    /// overrides removed so it always answers for this checkout.
    fn git(args: &[&str]) -> String {
        let mut command = Command::new("git");
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(name);
        }
        let output = command.args(args).output().expect("run git");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout).expect("git prints UTF-8")
    }

    #[test]
    fn imported_source_ownership_is_exact() {
        assert_eq!(owner_count("vendor/sqlx-sqlite-0.9.0/src/any.rs"), 1);
        assert_eq!(
            owner_count("vendor/sqlx-sqlite-0.9.0/src/connection/describe.rs"),
            1
        );
        assert_eq!(owner_count("vendor/other-crate/src/lib.rs"), 0);
        assert_eq!(
            owner_count("vendor/sqlx-sqlite-0.9.0/examples/example.rs"),
            0
        );
        assert_eq!(owner_count("vendor/sqlx-sqlite-0.9.0/src-other/lib.rs"), 0);
        assert_eq!(owner_count("new-workspace-crate/src/lib.rs"), 0);
    }

    #[test]
    fn tools_is_outside_the_fence_population() {
        assert!(!WORKSPACE.contains(&"tools"));
        assert_eq!(owner_count("tools/devtool/src/main.rs"), 1);
    }

    #[test]
    fn no_owner_is_a_prefix_of_another() {
        // Otherwise a file could fall under two owners and be reconciled against
        // the wrong run, or not at all.
        let all: Vec<&str> = owners().collect();
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                assert!(
                    i == j || !(a == b || b.starts_with(&format!("{a}/"))),
                    "{a} covers {b}"
                );
            }
        }
    }

    #[test]
    fn every_tracked_rust_source_has_exactly_one_owner() {
        // A crate outside every root is shrink vector 4, which no fixture crate
        // can demonstrate; this census is the assertion that covers it.
        //
        // Asked from the toplevel, not the cwd: `cargo test` runs with the
        // crate's directory as its cwd, and `ls-files` lists only what is
        // beneath it — a cwd-relative query would see a partial tree.
        let top = git(&["rev-parse", "--show-toplevel"]);
        let tracked = git(&["-C", top.trim_end(), "ls-files", "--", "*.rs"]);
        let tracked: Vec<&str> = tracked.lines().collect();
        assert!(
            tracked.len() > 100,
            "only {} tracked .rs files — the query saw a partial tree",
            tracked.len()
        );
        for path in tracked {
            let n = owner_count(path);
            assert_eq!(n, 1, "{path} has {n} source owners, want exactly 1");
        }
    }
}
