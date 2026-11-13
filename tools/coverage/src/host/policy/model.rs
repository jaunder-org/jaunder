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

/// Policy counts used by the existing local benchmark to compare verdicts.
/// These are not a command-result or producer-status envelope.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CoverageReport {
    pub failures: usize,
    pub guard_violations: usize,
    pub crap_fails: usize,
}
