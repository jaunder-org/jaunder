use serde::Serialize;

#[derive(Clone, Debug, PartialEq)]
pub struct LineCov {
    pub line: u32,
    pub covered: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileCoverage {
    pub path: String,
    pub lines: Vec<LineCov>,
}

/// The `.coverage` block of the host result envelope (`.xtask/last-result.json`):
/// the stateless gate's counts. This is NOT the Nix `status.json` (produced by
/// `devtool coverage emit` and read by CI/flake.nix) — it is the host's own
/// summary of the post-processing verdict.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CoverageReport {
    /// Uncovered, unexempt, un-ignored executable lines (each FAILS the gate).
    pub failures: usize,
    /// Covered lines inside an exempt (`unreachable!`) span (the A1-guard tripwire).
    pub guard_violations: usize,
    /// Functions whose CRAP exceeds the threshold with no `crap:allow` override.
    pub crap_fails: usize,
}
