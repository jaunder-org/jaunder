//! Text-identity re-anchor safety for the coverage heal.
//!
//! The line-identity classifier flags an uncovered line as a
//! `regression`/`new_uncovered` purely by line number. A line-shifting change
//! whose diff models an accepted gap as deleted-then-reappeared produces a
//! *phantom* failure even though coverage is unchanged. This module decides
//! whether such a line-dirty verdict is in fact a safe re-anchor: per file, the
//! multiset of *appeared* texts (regressions ∪ new_uncovered) must be contained
//! in the multiset of *structural* texts (accepted gaps the diff removed). A new
//! uncovered text with no removed-gap counterpart is a genuine lowering.

use std::collections::HashMap;

use crate::coverage::baseline::Baseline;
use crate::coverage::{CoverageVerdict, FileCoverage};

/// One genuinely-lowered uncovered line (an appeared text with no matching
/// removed-gap text), for the gate's failure report.
#[derive(Clone, Debug, PartialEq)]
pub struct LineText {
    pub file: String,
    pub line: u32,
    pub text: String,
}

/// The re-anchor safety verdict: `safe` overall, plus the genuine lowerings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReanchorSafety {
    pub safe: bool,
    pub lowering: Vec<LineText>,
}

/// Decide whether a (possibly line-dirty) verdict is a safe re-anchor.
///
/// `current` supplies the text of appeared (currently-uncovered) lines;
/// `baseline` supplies the text of structural (removed accepted-gap) lines.
/// Owned `String` keys keep the multiset bookkeeping free of cross-borrow
/// lifetime entanglement between `current` and `baseline`.
pub fn reanchor_is_safe(
    verdict: &CoverageVerdict,
    current: &[FileCoverage],
    baseline: &Baseline,
) -> ReanchorSafety {
    // (file, line) -> current source text.
    let mut cur_text: HashMap<(String, u32), String> = HashMap::new();
    for f in current {
        for l in &f.lines {
            cur_text.insert((f.path.clone(), l.line), l.text.clone());
        }
    }

    // Files that have any appeared (regression / new_uncovered) failure.
    let mut appeared_files: Vec<&str> = verdict
        .regressions
        .iter()
        .chain(&verdict.new_uncovered)
        .map(|fl| fl.file.as_str())
        .collect();
    appeared_files.sort_unstable();
    appeared_files.dedup();

    let mut lowering = Vec::new();
    for file in appeared_files {
        // Multiset of structural (removed accepted-gap) texts for this file.
        let structural_lines: std::collections::HashSet<u32> = verdict
            .structural
            .iter()
            .filter(|fl| fl.file == file)
            .flat_map(|fl| fl.lines.iter().copied())
            .collect();
        let mut counts: HashMap<String, i64> = HashMap::new();
        for g in baseline.gaps(file) {
            if structural_lines.contains(&g.line) {
                *counts.entry(g.text.clone()).or_default() += 1;
            }
        }

        // Each appeared line consumes one matching structural text; an
        // unmatched appeared text is a genuine lowering.
        let mut appeared: Vec<u32> = verdict
            .regressions
            .iter()
            .chain(&verdict.new_uncovered)
            .filter(|fl| fl.file == file)
            .flat_map(|fl| fl.lines.iter().copied())
            .collect();
        appeared.sort_unstable();
        // A single physical uncovered line consumes at most one structural slot;
        // dedup guards the multiset bookkeeping if the classifier ever lists a
        // line in both `regressions` and `new_uncovered` (today it never does).
        appeared.dedup();
        for line in appeared {
            let text = cur_text
                .get(&(file.to_string(), line))
                .cloned()
                .unwrap_or_default();
            let slot = counts.entry(text.clone()).or_default();
            if *slot > 0 {
                *slot -= 1;
            } else {
                lowering.push(LineText {
                    file: file.to_string(),
                    line,
                    text,
                });
            }
        }
    }

    ReanchorSafety {
        safe: lowering.is_empty(),
        lowering,
    }
}

/// Where a refused re-anchor writes its candidate baseline. Under the gitignored
/// `/.xtask/`, so it never dirties the tree or gets instrumented.
pub const CANDIDATE_PATH: &str = ".xtask/coverage-baseline.candidate.json";

/// The action a `coverage reanchor` run should take, decided purely from the safety
/// verdict. The candidate content is identical in both arms
/// (`Baseline::from_files(current)`); only the destination and the exit status differ.
pub enum ReanchorPlan {
    /// Safe re-anchor — persist `baseline` to the committed `coverage-baseline.json`.
    Reanchor { baseline: Baseline },
    /// Genuine lowering — persist `candidate` to [`CANDIDATE_PATH`] and refuse,
    /// surfacing the offending lines.
    Refuse {
        candidate: Baseline,
        lowering: Vec<LineText>,
    },
}

/// Decide the re-anchor action. Pure (no I/O) so it is unit-testable; the caller
/// performs the write and sets the exit status.
pub fn plan_reanchor(safety: ReanchorSafety, candidate: Baseline) -> ReanchorPlan {
    if safety.safe {
        ReanchorPlan::Reanchor {
            baseline: candidate,
        }
    } else {
        ReanchorPlan::Refuse {
            candidate,
            lowering: safety.lowering,
        }
    }
}

/// Operator-facing message for a refused re-anchor: the offending `file:line: text`
/// plus how to inspect and (only if genuinely approved) promote the candidate. There
/// is deliberately no flag that promotes automatically — approval is a visible diff.
pub fn refusal_report(lowering: &[LineText]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "refused: {} genuinely-uncovered line(s) would lower coverage:",
        lowering.len()
    );
    for l in lowering.iter().take(MAX) {
        let _ = write!(s, "\n    {}:{}: {}", l.file, l.line, l.text.trim());
    }
    if lowering.len() > MAX {
        let _ = write!(s, "\n    … and {} more", lowering.len() - MAX);
    }
    let _ = write!(
        s,
        "\n  wrote candidate to {CANDIDATE_PATH} (NOT the committed baseline).\
         \n  inspect:  git diff --no-index coverage-baseline.json {CANDIDATE_PATH}\
         \n  if genuinely approved (coverage-baseline policy), promote:\
         \n    cp {CANDIDATE_PATH} coverage-baseline.json && git add coverage-baseline.json\
         \n  otherwise add a test — never promote a real loss."
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::baseline::{Baseline, Gap};
    use crate::coverage::{FileCoverage, FileLines, LineCov};

    fn fc(path: &str, lines: &[(u32, bool, &str)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines
                .iter()
                .map(|(l, c, t)| LineCov {
                    line: *l,
                    covered: *c,
                    text: (*t).into(),
                })
                .collect(),
        }
    }

    fn baseline_with(path: &str, gaps: Vec<Gap>) -> Baseline {
        let mut b = Baseline::default();
        b.set_gaps(path, gaps);
        b
    }

    fn fl(file: &str, lines: &[u32]) -> FileLines {
        FileLines {
            file: file.into(),
            lines: lines.to_vec(),
        }
    }

    #[test]
    fn pure_move_is_safe() {
        // Accepted gap "let x = 1;" was at line 2 (now removed by the diff →
        // structural) and reappears uncovered at line 9 (→ new_uncovered),
        // identical text. appeared ⊆ structural → safe.
        let baseline = baseline_with(
            "a.rs",
            vec![Gap {
                line: 2,
                text: "let x = 1;".into(),
            }],
        );
        let current = vec![fc("a.rs", &[(9, false, "let x = 1;")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2])],
            new_uncovered: vec![fl("a.rs", &[9])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(s.safe, "identical-text move must be a safe re-anchor");
        assert!(s.lowering.is_empty());
    }

    #[test]
    fn net_zero_swap_is_not_safe() {
        // An accepted "}" gap was COVERED (improvement, not structural) and a
        // different "}" regressed. structural has no "}" → appeared "}" is a
        // genuine lowering.
        let baseline = baseline_with(
            "a.rs",
            vec![Gap {
                line: 2,
                text: "}".into(),
            }],
        );
        let current = vec![fc("a.rs", &[(2, true, "}"), (7, false, "}")])];
        let verdict = CoverageVerdict {
            improvements: vec![fl("a.rs", &[2])],
            regressions: vec![fl("a.rs", &[7])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(
            !s.safe,
            "covering one `}}` then regressing another must fail"
        );
        assert_eq!(
            s.lowering,
            vec![LineText {
                file: "a.rs".into(),
                line: 7,
                text: "}".into()
            }]
        );
    }

    #[test]
    fn genuine_new_uncovered_is_not_safe() {
        // Brand-new uncovered text, nothing removed → not contained.
        let baseline = Baseline::default();
        let current = vec![fc("a.rs", &[(5, false, "todo!()")])];
        let verdict = CoverageVerdict {
            new_uncovered: vec![fl("a.rs", &[5])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(!s.safe);
        assert_eq!(s.lowering.len(), 1);
    }

    #[test]
    fn line_clean_verdict_is_safe() {
        // No appeared failures → trivially safe.
        let s = reanchor_is_safe(&CoverageVerdict::default(), &[], &Baseline::default());
        assert!(s.safe);
        assert!(s.lowering.is_empty());
    }

    #[test]
    fn duplicate_text_counts_are_multiset() {
        // Two accepted "}" removed; two "}" reappear → contained (safe).
        let baseline = baseline_with(
            "a.rs",
            vec![
                Gap {
                    line: 2,
                    text: "}".into(),
                },
                Gap {
                    line: 4,
                    text: "}".into(),
                },
            ],
        );
        let current = vec![fc("a.rs", &[(8, false, "}"), (9, false, "}")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2, 4])],
            new_uncovered: vec![fl("a.rs", &[8, 9])],
            ..Default::default()
        };
        assert!(reanchor_is_safe(&verdict, &current, &baseline).safe);
    }

    #[test]
    fn identical_text_masks_a_new_gap_documented_residual() {
        // Residual ambiguity (ADR-0030): an accepted gap "}" is removed
        // (structural) while an UNRELATED brand-new uncovered "}" appears.
        // Text-identity cannot tell them apart, so the new gap consumes the
        // structural slot and is accepted. This pins the documented, bounded
        // behaviour — a future change to it should be deliberate.
        let baseline = baseline_with(
            "a.rs",
            vec![Gap {
                line: 2,
                text: "}".into(),
            }],
        );
        let current = vec![fc("a.rs", &[(40, false, "}")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2])],
            new_uncovered: vec![fl("a.rs", &[40])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(
            s.safe,
            "documented residual: identical text masks a new gap"
        );
    }

    #[test]
    fn more_appeared_than_removed_is_not_safe() {
        // One "}" removed but two "}" appear → one is a genuine lowering.
        let baseline = baseline_with(
            "a.rs",
            vec![Gap {
                line: 2,
                text: "}".into(),
            }],
        );
        let current = vec![fc("a.rs", &[(8, false, "}"), (9, false, "}")])];
        let verdict = CoverageVerdict {
            structural: vec![fl("a.rs", &[2])],
            new_uncovered: vec![fl("a.rs", &[8, 9])],
            ..Default::default()
        };
        let s = reanchor_is_safe(&verdict, &current, &baseline);
        assert!(!s.safe);
        assert_eq!(s.lowering.len(), 1);
    }

    #[test]
    fn plan_reanchors_when_safe() {
        let safety = ReanchorSafety {
            safe: true,
            lowering: vec![],
        };
        let candidate = Baseline::from_files(&[]);
        match plan_reanchor(safety, candidate) {
            ReanchorPlan::Reanchor { .. } => {}
            ReanchorPlan::Refuse { .. } => panic!("safe must re-anchor"),
        }
    }

    #[test]
    fn plan_refuses_and_carries_lowering_when_unsafe() {
        let safety = ReanchorSafety {
            safe: false,
            lowering: vec![LineText {
                file: "a.rs".into(),
                line: 5,
                text: "}".into(),
            }],
        };
        let candidate = Baseline::from_files(&[]);
        match plan_reanchor(safety, candidate) {
            ReanchorPlan::Refuse { lowering, .. } => {
                assert_eq!(lowering.len(), 1);
                assert_eq!(lowering[0].line, 5);
            }
            ReanchorPlan::Reanchor { .. } => panic!("a lowering must refuse"),
        }
    }

    #[test]
    fn refusal_report_lists_lines_and_promotion_recipe() {
        let report = refusal_report(&[LineText {
            file: "src/x.rs".into(),
            line: 12,
            text: "    Ok(())".into(),
        }]);
        assert!(report.contains("src/x.rs:12: Ok(())"));
        assert!(report.contains(CANDIDATE_PATH));
        assert!(report.contains("cp "));
        assert!(report.contains("git diff --no-index"));
    }
}
