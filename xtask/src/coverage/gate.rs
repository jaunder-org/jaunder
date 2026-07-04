//! Stateless coverage gate: a verdict computed purely from the current report
//! plus each file's structural exemptions ([`super::exempt`]) — no baseline, no
//! anchor, no history. This is the authoritative coverage gate (its verdict
//! feeds `gate_fails` in [`super`]): an executable line FAILS iff it is
//! uncovered AND not `#[component]`-exempt AND not suppressed by a `cov:ignore`
//! marker.
//!
//! The A1-guard tripwire is the load-bearing safety check: a *covered* line
//! inside an exempt (`#[component]`) span means the "components are never
//! rendered natively" invariant — the premise that makes blanket component
//! exemption a wash — is violated. Such a line is reported as a
//! `guard_violation` so the gate fails until it is understood.

use std::collections::BTreeSet;

use crate::coverage::FileCoverage;

/// The stateless gate's verdict: uncovered-and-unexempt `failures` plus
/// covered-but-exempt `guard_violations` (the A1 tripwire).
#[derive(Debug, Default)]
pub struct Verdict {
    pub failures: Vec<Fail>,
    pub guard_violations: Vec<Fail>,
}

/// One flagged line: where it is and its source text (for the recovery hint).
#[derive(Debug, Clone)]
pub struct Fail {
    pub file: String,
    pub line: u32,
    pub text: String,
}

/// Stateless verdict: an executable line FAILS iff uncovered AND not exempt.
///
/// Guard (A1 tripwire): a COVERED line inside an exempt (`#[component]`) span
/// means the "components are never natively rendered" invariant is violated → it
/// is reported in `guard_violations`.
///
/// `exempt_of(path)` returns the exempt line set for that file (empty on any
/// error — fail-closed: unknown → measured).
pub fn evaluate(files: &[FileCoverage], exempt_of: impl Fn(&str) -> BTreeSet<u32>) -> Verdict {
    let mut v = Verdict::default();
    for f in files {
        let ex = exempt_of(&f.path);
        for l in &f.lines {
            let exempt = ex.contains(&l.line);
            if !l.covered && !exempt {
                v.failures.push(Fail {
                    file: f.path.clone(),
                    line: l.line,
                    text: l.text.clone(),
                });
            } else if l.covered && exempt {
                v.guard_violations.push(Fail {
                    file: f.path.clone(),
                    line: l.line,
                    text: l.text.clone(),
                });
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::LineCov;
    use std::collections::BTreeMap;

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

    /// Build an `exempt_of` closure from a per-file exempt-line map.
    fn exempt_from(map: BTreeMap<&'static str, BTreeSet<u32>>) -> impl Fn(&str) -> BTreeSet<u32> {
        move |path| map.get(path).cloned().unwrap_or_default()
    }

    #[test]
    fn uncovered_unexempt_fails() {
        let files = vec![fc("a.rs", &[(3, false, "let x = 1;")])];
        let v = evaluate(&files, exempt_from(BTreeMap::new()));
        assert_eq!(v.failures.len(), 1, "{v:?}");
        assert_eq!(v.failures[0].file, "a.rs");
        assert_eq!(v.failures[0].line, 3);
        assert!(v.guard_violations.is_empty());
    }

    #[test]
    fn uncovered_in_component_passes() {
        let files = vec![fc("a.rs", &[(3, false, "view! { <div/> }")])];
        let exempt = exempt_from(BTreeMap::from([("a.rs", BTreeSet::from([3]))]));
        let v = evaluate(&files, exempt);
        assert!(
            v.failures.is_empty(),
            "exempt uncovered line must not fail: {v:?}"
        );
        assert!(v.guard_violations.is_empty());
    }

    #[test]
    fn covered_in_component_trips_guard() {
        let files = vec![fc("a.rs", &[(3, true, "view! { <div/> }")])];
        let exempt = exempt_from(BTreeMap::from([("a.rs", BTreeSet::from([3]))]));
        let v = evaluate(&files, exempt);
        assert!(v.failures.is_empty());
        assert_eq!(
            v.guard_violations.len(),
            1,
            "covered exempt line must trip guard: {v:?}"
        );
        assert_eq!(v.guard_violations[0].line, 3);
    }

    #[test]
    fn covered_unexempt_passes() {
        let files = vec![fc("a.rs", &[(3, true, "let x = 1;")])];
        let v = evaluate(&files, exempt_from(BTreeMap::new()));
        assert!(v.failures.is_empty());
        assert!(v.guard_violations.is_empty());
    }
}
