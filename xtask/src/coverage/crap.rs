//! Host-side CRAP regression comparison over the CRAP report `devtool coverage
//! emit` produces. Each CRAP entry is keyed by
//! `(crate, file, function, ordinal)` — the ordinal is the entry's index among
//! those sharing the first three, ordered by line, disambiguating same-named
//! functions in a file without keying on the churn-prone absolute line (#7). A
//! key present in BOTH the new report and the old manifest is flagged when
//! `new.crap > old.crap + EPSILON`. Keys only in new or only in old are not
//! regressions. The epsilon ignores float noise.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Sub-epsilon CRAP deltas are float noise, not regressions.
const EPSILON: f64 = 0.01;

/// The committed CRAP baseline. An ordinary (non-dotted) tracked file.
pub const CRAP_MANIFEST_PATH: &str = "crap-manifest.json";

/// A function whose CRAP score got meaningfully worse between old and new.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrapRegression {
    pub file: String,
    pub function: String,
    pub old: f64,
    pub new: f64,
}

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    #[serde(rename = "crate", default)]
    crate_field: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    function: String,
    #[serde(default)]
    line: i64,
    #[serde(default)]
    crap: f64,
}

/// (crate, file, function, ordinal). The ordinal is the entry's index among
/// those sharing (crate, file, function), ordered by line — a shift-stable
/// disambiguator for same-named functions in one file (e.g. several `from`
/// impls), replacing the churn-prone absolute `line` in the compare key (#7).
///
/// The ordinal is stable under pure line-shifts but re-assigns if two
/// same-named functions in one file are reordered, or one is inserted between
/// them — so a CRAP regression on such a function could be misattributed or
/// missed. That is a rare, accepted edge; the ordinal remains a strict
/// improvement over the absolute-line key, which mis-keyed on *every* shift.
type Key = (String, String, String, usize);

/// Map every entry to its line-independent key → CRAP score.
fn keyed(entries: &[Entry]) -> HashMap<Key, f64> {
    let mut groups: HashMap<(String, String, String), Vec<(i64, f64)>> = HashMap::new();
    for e in entries {
        groups
            .entry((e.crate_field.clone(), e.file.clone(), e.function.clone()))
            .or_default()
            .push((e.line, e.crap));
    }
    let mut out = HashMap::new();
    for ((c, f, fun), mut v) in groups {
        v.sort_by_key(|(line, _)| *line);
        for (i, (_, crap)) in v.into_iter().enumerate() {
            out.insert((c.clone(), f.clone(), fun.clone(), i), crap);
        }
    }
    out
}

/// Compare a new CRAP report against the old manifest. Returns one
/// [`CrapRegression`] per key present in both whose CRAP score worsened by more
/// than [`EPSILON`]. Keying on the line-independent ordinal means a pure line
/// shift no longer hides a regression behind a key mismatch.
pub fn compare(new_report: &str, old_manifest: &str) -> Result<Vec<CrapRegression>> {
    let new: Report = serde_json::from_str(new_report)?;
    let old: Report = serde_json::from_str(old_manifest)?;
    let old_by_key = keyed(&old.entries);

    // Re-derive the new side's ordinals alongside the entry so a regression can
    // report the offending file/function.
    let mut groups: HashMap<(String, String, String), Vec<&Entry>> = HashMap::new();
    for e in &new.entries {
        groups
            .entry((e.crate_field.clone(), e.file.clone(), e.function.clone()))
            .or_default()
            .push(e);
    }
    let mut regressions = Vec::new();
    for ((c, f, fun), mut v) in groups {
        v.sort_by_key(|e| e.line);
        for (i, e) in v.into_iter().enumerate() {
            let k = (c.clone(), f.clone(), fun.clone(), i);
            if let Some(&old_crap) = old_by_key.get(&k) {
                if e.crap > old_crap + EPSILON {
                    regressions.push(CrapRegression {
                        file: e.file.clone(),
                        function: e.function.clone(),
                        old: old_crap,
                        new: e.crap,
                    });
                }
            }
        }
    }
    Ok(regressions)
}

/// Canonical, line- and order-independent form of a CRAP report: each entry
/// minus its `line`, with key-sorted JSON (serde_json `Value` is a `BTreeMap`),
/// and the entry set itself sorted. Two reports that differ only in line
/// attribution (a pure shift) normalize equal, so a refresh does not rewrite
/// `crap-manifest.json` unless some non-`line` field changed — the `crap` score
/// or its `coverage`/`cyclomatic` inputs, or the set of functions (#7). The
/// `line` field is retained in the written manifest as a non-authoritative
/// jump-to hint that refreshes wholesale on the next such change.
pub fn normalize_without_line(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    let mut rows: Vec<String> = v
        .get("entries")
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .map(|e| {
                    let mut e = e.clone();
                    if let Some(o) = e.as_object_mut() {
                        o.remove("line");
                    }
                    e.to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort();
    Ok(rows.join("\n"))
}

/// Canonical (key-sorted, via `serde_json::Value`'s `BTreeMap`) but
/// pretty-printed with a trailing newline — the on-disk form of the committed
/// manifest, so coverage diffs stay readable.
pub fn pretty_manifest(s: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&v)?))
}

/// Where a refused refresh writes its candidate manifest. Under the gitignored
/// `/.xtask/`, so it never dirties the tree or gets instrumented (mirrors the
/// baseline candidate in `reanchor`).
pub const CRAP_CANDIDATE_PATH: &str = ".xtask/crap-manifest.candidate.json";

/// The action a `coverage refresh-crap` run should take, decided purely from the
/// fresh report vs. the committed manifest. No I/O — the caller writes the bytes
/// and sets the exit status.
#[derive(Debug, PartialEq)]
pub enum CrapRefreshPlan {
    /// No regressions. `manifest` is the pretty bytes to write to the committed
    /// manifest, or `None` when there is no CRAP-relevant drift (already current).
    Refresh { manifest: Option<String> },
    /// A regression would raise the bar: write `candidate` to the side path and
    /// refuse (non-zero). Promotion stays a manual `cp`.
    Refuse {
        candidate: String,
        regressions: Vec<CrapRegression>,
    },
}

/// Decide the refresh action. With no regressions, refresh in place only when a
/// CRAP-relevant field actually changed (a pure line-shift / no change is a
/// no-op, mirroring the Fix-mode heal's churn-avoidance). With regressions, the
/// fresh report becomes a candidate and the run refuses.
pub fn plan_crap_refresh(fresh_report: &str, old_manifest: &str) -> Result<CrapRefreshPlan> {
    let regressions = if old_manifest.trim().is_empty() {
        Vec::new()
    } else {
        compare(fresh_report, old_manifest)?
    };
    if regressions.is_empty() {
        let new_canon = normalize_without_line(fresh_report)?;
        let old_canon = normalize_without_line(old_manifest).unwrap_or_default();
        let manifest = if new_canon != old_canon {
            Some(pretty_manifest(fresh_report)?)
        } else {
            None
        };
        Ok(CrapRefreshPlan::Refresh { manifest })
    } else {
        Ok(CrapRefreshPlan::Refuse {
            candidate: pretty_manifest(fresh_report)?,
            regressions,
        })
    }
}

/// Operator-facing message for a refused refresh: the offending `file::fn old → new`
/// plus how to inspect and (only if genuinely approved) promote the candidate.
/// There is deliberately no flag that promotes automatically — approval is a
/// visible diff (mirrors `reanchor::refusal_report`).
pub fn refusal_report(regressions: &[CrapRegression]) -> String {
    use std::fmt::Write as _;
    const MAX: usize = 25;
    let mut s = format!(
        "refused: {} CRAP regression(s) would raise the complexity-risk bar:",
        regressions.len()
    );
    for r in regressions.iter().take(MAX) {
        let _ = write!(
            s,
            "\n    {}::{}  {:.2} → {:.2}",
            r.file, r.function, r.old, r.new
        );
    }
    if regressions.len() > MAX {
        let _ = write!(s, "\n    … and {} more", regressions.len() - MAX);
    }
    let _ = write!(
        s,
        "\n  wrote candidate to {CRAP_CANDIDATE_PATH} (NOT the committed manifest).\
         \n  inspect:  git diff --no-index {CRAP_MANIFEST_PATH} {CRAP_CANDIDATE_PATH}\
         \n  if genuinely approved (stale drift, not a real regression), promote:\
         \n    cp {CRAP_CANDIDATE_PATH} {CRAP_MANIFEST_PATH} && git add {CRAP_MANIFEST_PATH}\
         \n  otherwise reduce complexity or improve coverage — never promote a real regression."
    );
    s
}

// ---------------------------------------------------------------------------
// CRAP threshold gate (#231 Task 6, shadow-add).
//
// Independent of the `compare`/manifest machinery above: instead of comparing a
// fresh report against a committed baseline, this fails any function whose CRAP
// score exceeds a fixed threshold, minus an in-source `crap:allow` override. It
// is wired into the SHADOW output only; the authoritative gate still runs
// `compare`. The old path is removed in a later task.
// ---------------------------------------------------------------------------

/// The CRAP score above which a function fails the gate. The boundary is
/// **exclusive**: a score of exactly [`CRAP_THRESHOLD`] (or less) passes;
/// anything strictly greater fails.
const CRAP_THRESHOLD: f64 = 30.0;

/// The in-source override marker. A real `//` comment — trailing or standalone —
/// carrying a NON-EMPTY reason: `// crap:allow: <reason>`. An empty/missing
/// reason is not a valid override (the reason is required so the waiver is
/// self-documenting and reviewable).
const CRAP_ALLOW_MARKER: &str = "// crap:allow:";

/// Lines scanned ABOVE cargo-crap's reported line when locating a function's
/// span. That line is only *approximately* the signature: for
/// `test-support/src/main.rs::main` it is the `// cov:ignore-start` comment (the
/// `async fn` signature is one line below, the `#[tokio::main]` attribute one
/// above); for `server/src/main.rs::run` it lands inside the doc comment, six
/// lines above the signature. Scanning a window above absorbs the attribute /
/// doc-comment / signature line that may carry the marker.
const CRAP_SPAN_ABOVE: usize = 12;

/// Fallback body length (in lines) when no opening brace is found at/below the
/// reported line — keeps the span bounded rather than running to end-of-file.
const CRAP_SPAN_FALLBACK: usize = 40;

/// A function whose CRAP score exceeds [`CRAP_THRESHOLD`] with no in-source
/// `crap:allow` override.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CrapFail {
    pub file: String,
    pub function: String,
    pub line: i64,
    pub crap: f64,
}

/// Resolves an entry's (repo-relative) source path to its text so
/// [`evaluate_crap`] can look for an in-source `crap:allow` marker. Wrapping a
/// closure keeps the reader injectable — production reads the file from disk,
/// tests hand back a literal source string. A path it cannot resolve yields
/// `None`, which [`evaluate_crap`] treats as *no override* (fail-closed).
/// Maps a repo-relative source path to its text (or `None` if unresolvable).
type SourceResolver<'a> = Box<dyn Fn(&str) -> Option<String> + 'a>;

pub struct AllowSet<'a> {
    source_of: SourceResolver<'a>,
}

impl<'a> AllowSet<'a> {
    /// Build an [`AllowSet`] from a source-resolver closure.
    pub fn new(source_of: impl Fn(&str) -> Option<String> + 'a) -> Self {
        Self {
            source_of: Box::new(source_of),
        }
    }

    fn source(&self, file: &str) -> Option<String> {
        (self.source_of)(file)
    }
}

/// Parse a CRAP report (or the committed manifest — same shape) into its entries,
/// so callers outside this module can run [`evaluate_crap`] over them.
pub fn parse_entries(report: &str) -> Result<Vec<Entry>> {
    Ok(serde_json::from_str::<Report>(report)?.entries)
}

/// Fail every function whose CRAP score exceeds [`CRAP_THRESHOLD`] unless an
/// in-source `crap:allow` marker within the function's span overrides it. `allow`
/// resolves each over-threshold entry's source file to its text; an unreadable
/// file yields no override, so the function still fails (fail-closed).
pub fn evaluate_crap(entries: &[Entry], allow: &AllowSet<'_>) -> Vec<CrapFail> {
    let mut fails = Vec::new();
    for e in entries {
        if e.crap <= CRAP_THRESHOLD {
            continue;
        }
        let overridden = allow
            .source(&e.file)
            .is_some_and(|src| allow_overrides(&src, e.line));
        if overridden {
            continue;
        }
        fails.push(CrapFail {
            file: e.file.clone(),
            function: e.function.clone(),
            line: e.line,
            crap: e.crap,
        });
    }
    fails
}

/// Does `src` carry a valid `crap:allow` override within the span of the function
/// cargo-crap reported at (1-based) `line`? The span runs from a few lines above
/// that line (to cover an attribute / doc comment / signature) through the
/// brace-matched end of the function body — so it is robust to `line` pointing at
/// any of those, not just the exact signature.
fn allow_overrides(src: &str, line: i64) -> bool {
    let lines: Vec<&str> = src.lines().collect();
    if lines.is_empty() {
        return false;
    }
    let anchor = (line.max(1) as usize - 1).min(lines.len() - 1); // 0-based
    let start = anchor.saturating_sub(CRAP_SPAN_ABOVE);
    let end = function_body_end(&lines, anchor);
    lines[start..=end].iter().any(|l| is_allow_marker(l))
}

/// A line is a valid override marker iff it contains `// crap:allow:` followed by
/// a non-empty (trimmed) reason.
fn is_allow_marker(line: &str) -> bool {
    match line.find(CRAP_ALLOW_MARKER) {
        Some(pos) => !line[pos + CRAP_ALLOW_MARKER.len()..].trim().is_empty(),
        None => false,
    }
}

/// The 0-based index of the line closing the function body that opens at or below
/// `anchor`: find the first `{` at/after `anchor` and brace-match to its close.
/// Falls back to a bounded window when there is no brace (defensive — a reported
/// function should always have a body).
fn function_body_end(lines: &[&str], anchor: usize) -> usize {
    let n = lines.len();
    let Some(open) = (anchor..n).find(|&i| lines[i].contains('{')) else {
        return (anchor + CRAP_SPAN_FALLBACK).min(n - 1);
    };
    let mut depth = 0i32;
    for (i, l) in lines.iter().enumerate().skip(open) {
        for ch in l.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
    }
    n - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    const NEW_WORSE: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":3.0}]}"#;
    const NEW_SAME: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.005}]}"#;

    #[test]
    fn flags_worse_crap_beyond_epsilon() {
        let r = compare(NEW_WORSE, OLD).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].function, "f");
    }

    #[test]
    fn ignores_sub_epsilon_noise() {
        assert!(compare(NEW_SAME, OLD).unwrap().is_empty());
    }

    const OLD_LINE1: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
    // Same function, shifted to line 99, CRAP worsened.
    const NEW_SHIFTED_WORSE: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":5.0}]}"#;
    // Same function, shifted, CRAP unchanged.
    const NEW_SHIFTED_SAME: &str =
        r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":2.0}]}"#;

    #[test]
    fn detects_regression_across_a_line_shift() {
        let r = compare(NEW_SHIFTED_WORSE, OLD_LINE1).unwrap();
        assert_eq!(
            r.len(),
            1,
            "line shift must not hide a real CRAP regression"
        );
        assert_eq!(r[0].function, "f");
    }

    #[test]
    fn line_shift_alone_is_not_a_regression() {
        assert!(compare(NEW_SHIFTED_SAME, OLD_LINE1).unwrap().is_empty());
    }

    #[test]
    fn same_name_functions_in_one_file_are_disambiguated_by_ordinal() {
        // Two `from` impls in one file; the second worsened, the first held.
        let old = r#"{"entries":[
            {"crate":"c","file":"a.rs","function":"from","line":10,"crap":2.0},
            {"crate":"c","file":"a.rs","function":"from","line":20,"crap":2.0}]}"#;
        let new = r#"{"entries":[
            {"crate":"c","file":"a.rs","function":"from","line":10,"crap":2.0},
            {"crate":"c","file":"a.rs","function":"from","line":20,"crap":9.0}]}"#;
        let r = compare(new, old).unwrap();
        assert_eq!(r.len(), 1, "only the second `from` regressed");
        assert_eq!((r[0].old, r[0].new), (2.0, 9.0));
    }

    #[test]
    fn crap_normalize_ignores_line_and_formatting() {
        // Same scores, different line attribution + key order + whitespace →
        // equal canonical form, so the heal does not rewrite the manifest (#7).
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let b = r#"{ "entries": [ {"crap":2.0,"function":"f","file":"a.rs","crate":"c","line":888} ] }"#;
        assert_eq!(
            normalize_without_line(a).unwrap(),
            normalize_without_line(b).unwrap(),
            "line + key order + whitespace must not affect the canonical form"
        );
    }

    #[test]
    fn crap_normalize_detects_a_score_change() {
        let a = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let c = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":9.0}]}"#;
        assert_ne!(
            normalize_without_line(a).unwrap(),
            normalize_without_line(c).unwrap(),
            "a real CRAP change must change the canonical form"
        );
    }

    #[test]
    fn crap_pretty_json_is_multiline() {
        let compact =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        assert!(pretty_manifest(compact).unwrap().contains('\n'));
    }

    #[test]
    fn refresh_writes_when_crap_relevant_field_changed() {
        // No regression key match (different function name) but a real CRAP-relevant
        // change → Refresh carrying the pretty manifest to write.
        let old = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let fresh =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"g","line":1,"crap":2.0}]}"#;
        match plan_crap_refresh(fresh, old).unwrap() {
            CrapRefreshPlan::Refresh { manifest: Some(m) } => assert!(m.contains("\"g\"")),
            other => panic!("expected Refresh(Some), got {other:?}"),
        }
    }

    #[test]
    fn refresh_is_noop_on_pure_line_shift() {
        // Same scores, only line attribution differs → already current (no write).
        let old = r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":1,"crap":2.0}]}"#;
        let fresh =
            r#"{"entries":[{"crate":"c","file":"a.rs","function":"f","line":99,"crap":2.0}]}"#;
        assert_eq!(
            plan_crap_refresh(fresh, old).unwrap(),
            CrapRefreshPlan::Refresh { manifest: None }
        );
    }

    #[test]
    fn refresh_refuses_and_carries_candidate_on_regression() {
        match plan_crap_refresh(NEW_WORSE, OLD).unwrap() {
            CrapRefreshPlan::Refuse {
                candidate,
                regressions,
            } => {
                assert_eq!(regressions.len(), 1);
                assert_eq!(regressions[0].function, "f");
                assert!(
                    candidate.contains("\"crap\""),
                    "candidate is the pretty fresh report"
                );
            }
            other => panic!("expected Refuse, got {other:?}"),
        }
    }

    #[test]
    fn first_run_empty_manifest_writes_initial() {
        // Empty committed manifest (first run) → no regressions, write the fresh one.
        match plan_crap_refresh(OLD, "").unwrap() {
            CrapRefreshPlan::Refresh { manifest: Some(_) } => {}
            other => panic!("expected Refresh(Some), got {other:?}"),
        }
    }

    #[test]
    fn refusal_report_lists_functions_and_promotion_recipe() {
        let report = refusal_report(&[CrapRegression {
            file: "b.rs".into(),
            function: "f".into(),
            old: 9.0,
            new: 11.0,
        }]);
        assert!(report.contains("b.rs::f  9.00 → 11.00"), "{report}");
        assert!(report.contains(CRAP_CANDIDATE_PATH));
        assert!(report.contains("git diff --no-index"));
        assert!(report.contains("cp "));
        assert!(report.contains(CRAP_MANIFEST_PATH));
    }

    // --- CRAP threshold gate (#231 Task 6) ---------------------------------

    fn ent(file: &str, function: &str, line: i64, crap: f64) -> Entry {
        Entry {
            crate_field: "c".into(),
            file: file.into(),
            function: function.into(),
            line,
            crap,
        }
    }

    /// An `AllowSet` that never resolves a source file → no overrides possible.
    fn no_source() -> AllowSet<'static> {
        AllowSet::new(|_| None)
    }

    #[test]
    fn crap_over_threshold_fails() {
        let fails = evaluate_crap(&[ent("a.rs", "big", 10, 31.0)], &no_source());
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].function, "big");
        assert_eq!(fails[0].crap, 31.0);
    }

    #[test]
    fn crap_at_threshold_passes() {
        // Exclusive boundary: exactly 30 and just under pass; strictly over fails.
        let entries = [
            ent("a.rs", "exact", 1, 30.0),
            ent("a.rs", "under", 1, 29.999),
            ent("a.rs", "over", 1, 30.001),
        ];
        let fails = evaluate_crap(&entries, &no_source());
        assert_eq!(fails.len(), 1, "only the strictly-over function fails");
        assert_eq!(fails[0].function, "over");
    }

    #[test]
    fn crap_allow_overrides_single_fn() {
        let src = "\
async fn main() {
    // crap:allow: test harness entrypoint; real fix tracked in #232
    body();
}
";
        let allow = AllowSet::new(move |f: &str| (f == "m.rs").then(|| src.to_string()));
        let entries = [
            ent("m.rs", "main", 1, 156.0), // overridden in-source
            ent("o.rs", "other", 1, 99.0), // no source resolvable → still fails
        ];
        let fails = evaluate_crap(&entries, &allow);
        assert_eq!(fails.len(), 1, "only the un-waived function fails");
        assert_eq!(fails[0].function, "other");
    }

    #[test]
    fn crap_allow_requires_reason() {
        // A `crap:allow` marker with an empty reason is NOT a valid override.
        let src = "\
async fn main() {
    // crap:allow:
    body();
}
";
        let allow = AllowSet::new(move |_: &str| Some(src.to_string()));
        let fails = evaluate_crap(&[ent("m.rs", "main", 1, 156.0)], &allow);
        assert_eq!(fails.len(), 1, "an empty reason must not override");
    }

    #[test]
    fn crap_allow_matched_within_span() {
        // cargo-crap's reported line sits ABOVE the signature (as it does for the
        // real `main`), and the marker is several lines BELOW it, deep in the
        // body — the span must still cover it.
        let src = "\
#[tokio::main]
// cov:ignore-start
async fn main() -> Result<()> {
    let cli = parse();
    let x = work(cli);
    // crap:allow: harness entrypoint; real fix tracked in #232
    finish(x);
    Ok(())
}
";
        // Reported line 2 (the comment above the signature); marker on line 6.
        let allow = AllowSet::new(move |_: &str| Some(src.to_string()));
        let fails = evaluate_crap(&[ent("m.rs", "main", 2, 156.0)], &allow);
        assert!(
            fails.is_empty(),
            "a marker within the function span overrides even when a few lines from the reported line"
        );
    }
}
