//! The scan roots, in one place because two crates consume them: `devtool` scans
//! [`WORKSPACE`] inside the Nix producer, and `xtask` scans [`HOST`] and asserts
//! [`ALL`] covers every tracked `.rs` file.
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
    "web",
];

/// Roots no Nix check can see: `xtask/` is excluded from the flake `src` filter
/// and `tools/` is a separate virtual workspace.
pub const HOST: &[&str] = &["xtask", "tools"];

/// Every scan root. The union must cover every tracked `.rs` file.
pub const ALL: &[&str] = &[
    "client",
    "common",
    "csr",
    "host",
    "macros",
    "server",
    "storage",
    "test-support",
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
