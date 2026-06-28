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
struct Entry {
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
}
