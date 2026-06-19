// Scaffold module: the parser API below is consumed by later coverage tasks
// (diffmap/baseline/classify/crap/run). Suppress dead-code noise until wired.
#![allow(dead_code)]

pub mod baseline;
pub mod classify;
pub mod crap;
pub mod diffmap;
pub mod report;

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

/// A file plus a set of its line numbers — the unit reported in each verdict
/// bucket. Lines are kept sorted for stable output/diffs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileLines {
    pub file: String,
    pub lines: Vec<u32>,
}

/// The classifier's verdict: each delta bucketed by line identity.
/// `regressions` (previously-covered line now uncovered) and `new_uncovered`
/// (brand-new uncovered line) both FAIL the gate; `structural` (a baseline gap
/// whose line was deleted) and `improvements` (a baseline gap now covered) are
/// safe deltas the gate auto-heals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageVerdict {
    pub regressions: Vec<FileLines>,
    pub new_uncovered: Vec<FileLines>,
    pub structural: Vec<FileLines>,
    pub improvements: Vec<FileLines>,
}

impl CoverageVerdict {
    /// Clean iff there is nothing that fails the gate: no regressions and no
    /// new uncovered lines. (structural/improvements are heals, not failures.)
    pub fn is_clean(&self) -> bool {
        self.regressions.is_empty() && self.new_uncovered.is_empty()
    }
}
