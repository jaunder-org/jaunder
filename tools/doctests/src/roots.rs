//! The scan roots, in one place because two crates consume them: `devtool` scans
//! [`WORKSPACE`] inside the Nix producer, and `xtask` scans [`HOST`] and asserts
//! [`ALL`] covers every maintained `.rs` file. Imported dependency sources have
//! separate ownership; Cargo's workspace doctest runs do not execute them.
//!
//! Duplicating the lists would let the population the gate *checks* drift from
//! the population it *scans* — a gate quietly shrinking its own reach, which is
//! the failure ADR-0085 principle 6 names.

/// Root-workspace directories the `cargo test --workspace --doc` run covers.
///
/// Explicit rather than "every `.rs` in the derivation source": the source also
/// carries `tools/` (only `xtask/` is filtered out, `nix/packages.nix`), which the
/// workspace run does not reach — scanning it there would manufacture `NotRun`
/// violations for fences that are gated host-side instead.
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

/// Roots no Nix check can see: `xtask/` is excluded from the flake `src` filter
/// and `tools/` is a separate virtual workspace.
pub const HOST: &[&str] = &["xtask", "tools"];

/// Imported dependency source, not maintained workspace doctest targets.
///
/// Keep this exact rather than excluding all of `vendor/`: an unclassified
/// crate must still fail the repository ownership census. Provenance and removal
/// are documented in `docs/sqlx-sqlite-patch.md`.
pub const VENDORED_DEPENDENCY_SOURCES: &[&str] = &["vendor/sqlx-sqlite-0.9.0/src"];

/// Every maintained scan root. Together with explicit imported-source ownership,
/// the union must account for every tracked `.rs` file exactly once.
pub const ALL: &[&str] = &[
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
    "xtask",
    "tools",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_is_exactly_workspace_plus_host() {
        let mut want: Vec<&str> = WORKSPACE.iter().chain(HOST).copied().collect();
        want.sort_unstable();
        let mut got: Vec<&str> = ALL.to_vec();
        got.sort_unstable();
        assert_eq!(got, want);
    }

    #[test]
    fn imported_source_ownership_is_exact_and_disjoint_from_scan_roots() {
        let owners = |path: &str| {
            ALL.iter()
                .chain(VENDORED_DEPENDENCY_SOURCES)
                .filter(|root| path.starts_with(&format!("{root}/")))
                .count()
        };
        assert_eq!(owners("vendor/sqlx-sqlite-0.9.0/src/any.rs"), 1);
        assert_eq!(
            owners("vendor/sqlx-sqlite-0.9.0/src/connection/describe.rs"),
            1
        );
        assert_eq!(owners("vendor/other-crate/src/lib.rs"), 0);
        assert_eq!(owners("vendor/sqlx-sqlite-0.9.0/examples/example.rs"), 0);
        assert_eq!(owners("vendor/sqlx-sqlite-0.9.0/src-other/lib.rs"), 0);
        assert_eq!(owners("new-workspace-crate/src/lib.rs"), 0);
        for imported in VENDORED_DEPENDENCY_SOURCES {
            for root in ALL {
                assert!(
                    !imported.starts_with(&format!("{root}/"))
                        && !root.starts_with(&format!("{imported}/"))
                        && root != imported,
                    "{imported} overlaps scan root {root}"
                );
            }
        }
    }

    #[test]
    fn no_root_is_a_prefix_of_another() {
        // Otherwise a file could fall under two roots and be reconciled against
        // the wrong run.
        for a in ALL {
            for b in ALL {
                assert!(a == b || !b.starts_with(&format!("{a}/")), "{a} covers {b}");
            }
        }
    }
}
