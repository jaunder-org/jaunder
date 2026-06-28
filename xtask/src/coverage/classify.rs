//! Text-verified coverage classifier.
//!
//! Given the current per-line coverage, the committed accepted-uncovered
//! baseline, and the anchor→working-tree line maps, resolve each baseline gap to
//! its *current* location and bucket every delta:
//!
//! - **structural** (heal): a baseline gap whose source text is gone entirely.
//! - **improvement** (heal): a baseline gap whose text is now only on covered lines.
//! - *accepted (silent):* a baseline gap whose text is still on an uncovered line.
//! - **regression** (FAIL): a current uncovered line no gap claims, that is not
//!   newly added (it existed before → it was covered, now isn't).
//! - **new_uncovered** (FAIL): a current uncovered line no gap claims, that IS
//!   newly added (no preimage in the diff).
//!
//! Resolution is **text-verified**: a gap is mapped through the diff, but the
//! mapped line is only accepted when its *text* matches the gap. When it does
//! not — a move the diff can't see, e.g. an upstream rebase shift baked into the
//! anchor commit's tree — the gap is found by searching for its text among the
//! current lines (nearest to the diff hint, disambiguating duplicate texts).
//! This keeps the gate robust to line-shifts the anchor diff misses, while the
//! confirm-the-mapped-line-first rule keeps the net-zero-swap sound: a gap whose
//! own line is now covered resolves to an *improvement* before any same-text line
//! elsewhere can claim it, so a genuinely new uncovered line of the same text is
//! still flagged. Line numbers are a hint for disambiguation, not the key.

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
        let added: HashSet<u32> = map.added_lines();

        // Per-line text (for confirming a diff-mapped line) and text→lines
        // indexes (for finding a gap whose line the diff couldn't map).
        let mut covered_text: HashMap<u32, &str> = HashMap::new();
        let mut uncovered_text: HashMap<u32, &str> = HashMap::new();
        let mut uncovered_by_text: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut covered_by_text: HashMap<&str, Vec<u32>> = HashMap::new();
        for l in &f.lines {
            let t = l.text.as_str();
            if l.covered {
                covered_text.insert(l.line, t);
                covered_by_text.entry(t).or_default().push(l.line);
            } else {
                uncovered_text.insert(l.line, t);
                uncovered_by_text.entry(t).or_default().push(l.line);
            }
        }

        // 1. Resolve each baseline gap to a current line, text-verified.
        let mut claimed: HashSet<u32> = HashSet::new(); // uncovered lines a gap owns
        let (mut structural, mut improvements) = (Vec::new(), Vec::new());
        for g in baseline.gaps(&f.path) {
            let gt = g.text.as_str();
            let mapped = map.map(g.line);

            // 1a. The diff-mapped line, accepted ONLY if its text confirms the
            //     gap — so a gap whose own line is now covered resolves to an
            //     improvement here, before any same-text line elsewhere can steal
            //     it (the net-zero-swap soundness).
            if let Some(c) = mapped {
                if covered_text.get(&c).copied() == Some(gt) {
                    improvements.push(c);
                    continue;
                }
                if uncovered_text.get(&c).copied() == Some(gt) && !claimed.contains(&c) {
                    claimed.insert(c);
                    continue;
                }
            }

            // 1b. The diff couldn't map the gap to its text (a move it can't see).
            //     Find the gap's text, nearest to the hint, preferring an
            //     unclaimed uncovered line (still a gap), then a covered line
            //     (improved), else it is gone (structural).
            let hint = mapped.unwrap_or(g.line);
            if let Some(k) = nearest_unclaimed(uncovered_by_text.get(gt), &claimed, hint) {
                claimed.insert(k);
                continue;
            }
            if let Some(k) = nearest(covered_by_text.get(gt), hint) {
                improvements.push(k);
                continue;
            }
            structural.push(g.line);
        }
        push_nonempty(&mut v.structural, &f.path, structural);
        push_nonempty(&mut v.improvements, &f.path, improvements);

        // 2. Current uncovered lines no gap claimed are genuine lowerings.
        let (mut regr, mut newu) = (Vec::new(), Vec::new());
        for l in &f.lines {
            if l.covered || claimed.contains(&l.line) {
                continue;
            }
            if added.contains(&l.line) {
                newu.push(l.line); // brand-new line, uncovered
            } else {
                regr.push(l.line); // existed before, was covered, now isn't
            }
        }
        push_nonempty(&mut v.regressions, &f.path, regr);
        push_nonempty(&mut v.new_uncovered, &f.path, newu);
    }
    v
}

/// The unclaimed candidate line nearest to `hint` (for disambiguating duplicate
/// texts), or `None` if there are no candidates.
fn nearest_unclaimed(
    candidates: Option<&Vec<u32>>,
    claimed: &HashSet<u32>,
    hint: u32,
) -> Option<u32> {
    candidates?
        .iter()
        .copied()
        .filter(|k| !claimed.contains(k))
        .min_by_key(|k| k.abs_diff(hint))
}

/// The candidate line nearest to `hint`, or `None`.
fn nearest(candidates: Option<&Vec<u32>>, hint: u32) -> Option<u32> {
    candidates?.iter().copied().min_by_key(|k| k.abs_diff(hint))
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

    /// Build a FileCoverage from (line, covered, text) triples.
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

    fn gap(line: u32, text: &str) -> Gap {
        Gap {
            line,
            text: text.into(),
        }
    }

    fn baseline_with(path: &str, gaps: Vec<Gap>) -> Baseline {
        let mut b = Baseline::default();
        b.set_gaps(path, gaps);
        b
    }

    // --- bucket 1: structural (gap text gone) ----------------------------------
    #[test]
    fn structural_when_baseline_gap_text_is_gone() {
        let b = baseline_with("a.rs", vec![gap(2, "let gone = 1;")]);
        let mut lm = LineMap::default();
        lm.set_for_test(2, None); // old line 2 deleted
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        let cur = vec![fc("a.rs", &[(1, true, "let other = 1;")])];

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

    // --- bucket 2: improvement (gap text now covered) --------------------------
    #[test]
    fn improvement_when_baseline_gap_line_is_now_covered() {
        let b = baseline_with("a.rs", vec![gap(3, "x")]);
        let maps: HashMap<String, LineMap> = HashMap::new(); // identity map
        let cur = vec![fc("a.rs", &[(3, true, "x")])];

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

    // --- bucket 3: accepted (gap still uncovered) — silent ---------------------
    #[test]
    fn accepted_gap_still_uncovered_is_silent_and_clean() {
        let b = baseline_with("a.rs", vec![gap(3, "x")]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        let cur = vec![fc("a.rs", &[(3, false, "x")])];

        let v = classify(&cur, &b, &maps);

        assert!(v.structural.is_empty());
        assert!(v.improvements.is_empty());
        assert!(v.regressions.is_empty(), "accepted gap must not regress");
        assert!(v.new_uncovered.is_empty());
        assert!(v.is_clean());
    }

    // --- bucket 4: regression (uncovered, not a gap, not added) ----------------
    #[test]
    fn regression_when_previously_covered_line_is_now_uncovered_identity_map() {
        let b = Baseline::default();
        let maps: HashMap<String, LineMap> = HashMap::new();
        let cur = vec![fc("a.rs", &[(5, false, "y")])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.regressions,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![5]
            }]
        );
        assert!(v.new_uncovered.is_empty());
        assert!(!v.is_clean());
    }

    // --- bucket 5: new_uncovered (added line, uncovered) -----------------------
    #[test]
    fn new_uncovered_when_added_line_is_uncovered() {
        let b = Baseline::default();
        let mut lm = LineMap::default();
        lm.set_added_for_test(7);
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        let cur = vec![fc("a.rs", &[(7, false, "z")])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.new_uncovered,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![7]
            }]
        );
        assert!(v.regressions.is_empty());
        assert!(!v.is_clean());
    }

    #[test]
    fn added_covered_line_is_not_flagged() {
        let b = Baseline::default();
        let mut lm = LineMap::default();
        lm.set_added_for_test(7);
        lm.set_added_for_test(8);
        let mut maps = HashMap::new();
        maps.insert("a.rs".to_string(), lm);
        let cur = vec![fc("a.rs", &[(7, true, "a"), (8, false, "b")])];

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

    // --- text-verified: a gap the diff can't map (rebase shift) self-resolves --
    #[test]
    fn gap_found_by_text_when_diff_cannot_map_it() {
        // Identity (empty) map — the diff sees no shift, as after a clean rebase
        // where origin/main moved the line. The gap's text "foo()" is now at line
        // 18 (uncovered); line 10 carries different, covered code. Must self-resolve
        // to the moved gap with NO phantom drift.
        let b = baseline_with("a.rs", vec![gap(10, "foo()")]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        let cur = vec![fc("a.rs", &[(10, true, "bar()"), (18, false, "foo()")])];

        let v = classify(&cur, &b, &maps);

        assert!(
            v.is_clean(),
            "a gap moved by an unseen shift must self-resolve, not phantom-fail: {v:?}"
        );
    }

    // --- soundness: net-zero swap of an identical-text line still fails ---------
    #[test]
    fn net_zero_swap_of_identical_text_still_fails() {
        // The gap's own line (2) is now COVERED; a DIFFERENT line (7) with the
        // same text regressed. The gap must resolve to its own line as an
        // improvement, leaving line 7 unclaimed → a regression. (If text-search
        // greedily claimed line 7, this real regression would be masked.)
        let b = baseline_with("a.rs", vec![gap(2, "}")]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        let cur = vec![fc("a.rs", &[(2, true, "}"), (7, false, "}")])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.regressions,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![7]
            }]
        );
        assert!(
            !v.is_clean(),
            "the swapped-in uncovered line must still fail"
        );
    }

    // --- duplicates: several same-text gaps all shift, 1:1 by proximity --------
    #[test]
    fn duplicate_text_gaps_resolve_one_to_one() {
        let b = baseline_with("a.rs", vec![gap(2, "}"), gap(4, "}")]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        // Both "}" gaps moved; now uncovered at 8 and 9.
        let cur = vec![fc("a.rs", &[(8, false, "}"), (9, false, "}")])];

        let v = classify(&cur, &b, &maps);

        assert!(
            v.is_clean(),
            "two same-text gaps must claim the two same-text lines 1:1: {v:?}"
        );
    }

    // --- a genuine new uncovered line is caught even with no diff (rebase) ------
    #[test]
    fn real_lowering_caught_with_empty_map() {
        let b = baseline_with("a.rs", vec![gap(5, "keep")]);
        let maps: HashMap<String, LineMap> = HashMap::new();
        // The accepted gap "keep" is still uncovered; a new uncovered "newbug"
        // appears with no diff to mark it added.
        let cur = vec![fc("a.rs", &[(5, false, "keep"), (9, false, "newbug")])];

        let v = classify(&cur, &b, &maps);

        assert_eq!(
            v.regressions,
            vec![FileLines {
                file: "a.rs".into(),
                lines: vec![9]
            }]
        );
        assert!(!v.is_clean());
    }
}
