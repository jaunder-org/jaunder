//! Line-identity coverage classifier.
//!
//! Given the current per-line coverage, the committed accepted-uncovered
//! baseline, and the HEAD→working-tree line maps, bucket every delta:
//!
//! - **structural** (heal): a baseline gap whose line maps to `None` (deleted).
//! - **improvement** (heal): a baseline gap whose line maps to a now-covered line.
//! - *accepted (silent):* a baseline gap whose line still maps to an uncovered
//!   line — a known gap, kept, not reported.
//! - **regression** (FAIL): a current uncovered line that is not an accepted-gap
//!   image and is NOT newly added (it existed at baseline → it was covered, now
//!   isn't).
//! - **new_uncovered** (FAIL): a current uncovered line that is not an
//!   accepted-gap image and IS newly added (no HEAD preimage).
//!
//! Under an identity/empty map `added_lines()` is empty, so every non-gap
//! uncovered line is a `regression` — correct, because with no diff nothing is
//! "new".

use std::collections::{HashMap, HashSet};

use crate::coverage::baseline::Baseline;
use crate::coverage::diffmap::{empty_map, LineMap};
use crate::coverage::{CoverageVerdict, FileCoverage, FileLines};

pub fn classify(
    current: &[FileCoverage],
    baseline: &Baseline,
    maps: &HashMap<String, LineMap>,
) -> CoverageVerdict {
    let mut v = CoverageVerdict::default();
    let default_map = empty_map();

    for f in current {
        let map = maps.get(&f.path).unwrap_or(&default_map);
        let covered_now: HashSet<u32> = f
            .lines
            .iter()
            .filter(|l| l.covered)
            .map(|l| l.line)
            .collect();
        let uncovered_now: HashSet<u32> = f
            .lines
            .iter()
            .filter(|l| !l.covered)
            .map(|l| l.line)
            .collect();
        let added: HashSet<u32> = map.added_lines();

        // 1. Walk baseline gaps: each maps to deleted / now-covered / still-gap.
        let mut accepted_images: HashSet<u32> = HashSet::new();
        let (mut structural, mut improvements) = (Vec::new(), Vec::new());
        for g in baseline.gaps(&f.path) {
            match map.map(g.line) {
                None => structural.push(g.line),
                Some(c) if covered_now.contains(&c) => improvements.push(c),
                Some(c) => {
                    accepted_images.insert(c); // still an accepted, unchanged gap
                }
            }
        }
        push_nonempty(&mut v.structural, &f.path, structural);
        push_nonempty(&mut v.improvements, &f.path, improvements);

        // 2. Walk current uncovered lines not explained by an accepted gap image.
        let (mut regr, mut newu) = (Vec::new(), Vec::new());
        for &c in &uncovered_now {
            if accepted_images.contains(&c) {
                continue; // a known, unchanged baseline gap
            }
            if added.contains(&c) {
                newu.push(c); // brand-new line, uncovered
            } else {
                regr.push(c); // existed at baseline, was covered, now isn't
            }
        }
        push_nonempty(&mut v.regressions, &f.path, regr);
        push_nonempty(&mut v.new_uncovered, &f.path, newu);
    }
    v
}

fn push_nonempty(into: &mut Vec<FileLines>, file: &str, mut lines: Vec<u32>) {
    if !lines.is_empty() {
        lines.sort();
        into.push(FileLines {
            file: file.to_string(),
            lines,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::baseline::{Baseline, Gap};
    use crate::coverage::diffmap::LineMap;
    use crate::coverage::{FileCoverage, LineCov};
    use std::collections::HashMap;

    /// Build a FileCoverage from (line, covered) pairs; text is irrelevant here.
    fn fc(path: &str, lines: &[(u32, bool)]) -> FileCoverage {
        FileCoverage {
            path: path.into(),
            lines: lines
                .iter()
                .map(|(l, c)| LineCov {
                    line: *l,
                    covered: *c,
                    text: String::new(),
                })
                .collect(),
        }
    }

    fn gap(line: u32) -> Gap {
        Gap {
            line,
            text: String::new(),
        }
    }

    fn baseline_with(path: &str, gaps: Vec<Gap>) -> Baseline {
        let mut b = Baseline::default();
        b.set_gaps(path, gaps);
        b
    }

    // --- bucket 1: structural (deleted baseline gap) ---------------------------
    #[test]
    fn structural_when_baseline_gap_line_is_deleted() {
        let b = baseline_with("a.rs", vec![gap(2)]);
        let mut lm = LineMap::default();
        lm.set_for_test(2, None); // old line 2 deleted
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        // current file no longer has the gap line; only line 1 remains, covered.
        let cur = vec![fc("a.rs", &[(1, true)])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.structural,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![2]
            }]
        );
        assert!(v.improvements.is_empty());
        assert!(v.regressions.is_empty());
        assert!(v.new_uncovered.is_empty());
        assert!(v.is_clean());
    }

    // --- bucket 2: improvement (baseline gap now covered) ----------------------
    #[test]
    fn improvement_when_baseline_gap_line_is_now_covered() {
        let b = baseline_with("a.rs", vec![gap(3)]);
        // identity map (no diff); line 3 is now covered.
        let maps: HashMap<String, LineMap> = HashMap::new();
        let cur = vec![fc("a.rs", &[(3, true)])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.improvements,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![3]
            }]
        );
        assert!(v.structural.is_empty());
        assert!(v.regressions.is_empty());
        assert!(v.new_uncovered.is_empty());
        assert!(v.is_clean());
    }

    // --- bucket 3: accepted (baseline gap still uncovered) — silent ------------
    #[test]
    fn accepted_gap_still_uncovered_is_silent_and_clean() {
        let b = baseline_with("a.rs", vec![gap(3)]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        // line 3 still uncovered — exactly the accepted gap.
        let cur = vec![fc("a.rs", &[(3, false)])];

        let v = classify(&cur, &b, &maps);

        assert!(v.structural.is_empty());
        assert!(v.improvements.is_empty());
        assert!(v.regressions.is_empty(), "accepted gap must not regress");
        assert!(v.new_uncovered.is_empty());
        assert!(v.is_clean());
    }

    // --- bucket 4: regression (covered→uncovered, identity map) ----------------
    #[test]
    fn regression_when_previously_covered_line_is_now_uncovered_identity_map() {
        // No baseline gaps for this file; no diff (identity map).
        let b = Baseline::default();
        let maps: HashMap<String, LineMap> = HashMap::new();
        // line 5 is uncovered now; not a gap, not newly added → regression.
        let cur = vec![fc("a.rs", &[(5, false)])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.regressions,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![5]
            }]
        );
        assert!(v.new_uncovered.is_empty());
        assert!(v.structural.is_empty());
        assert!(v.improvements.is_empty());
        assert!(!v.is_clean());
    }

    // --- bucket 5: new_uncovered (added line, uncovered) -----------------------
    #[test]
    fn new_uncovered_when_added_line_is_uncovered() {
        let b = Baseline::default();
        // A map that marks current line 7 as added (no HEAD preimage).
        let mut lm = LineMap::default();
        lm.set_added_for_test(7);
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        let cur = vec![fc("a.rs", &[(7, false)])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.new_uncovered,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![7]
            }]
        );
        assert!(v.regressions.is_empty());
        assert!(v.structural.is_empty());
        assert!(v.improvements.is_empty());
        assert!(!v.is_clean());
    }

    // --- mixed: an added covered line plus an added uncovered line -------------
    #[test]
    fn added_covered_line_is_not_flagged() {
        let b = Baseline::default();
        let mut lm = LineMap::default();
        lm.set_added_for_test(7);
        lm.set_added_for_test(8);
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        // line 7 added+covered (fine), line 8 added+uncovered (new_uncovered).
        let cur = vec![fc("a.rs", &[(7, true), (8, false)])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.new_uncovered,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![8]
            }]
        );
        assert!(v.regressions.is_empty());
    }
}
