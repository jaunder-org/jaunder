//! Host-side CRAP regression comparison — mirrors the jq gate
//! `scripts/check-coverage` runs today. Each CRAP entry is keyed by
//! `(crate, file, function, line)` (line disambiguates same-named functions in
//! a file); a key present in BOTH the new report and the old manifest is flagged
//! when `new.crap > old.crap + EPSILON`. Keys only in new or only in old are not
//! regressions. The epsilon ignores float noise.

use std::collections::HashMap;

use anyhow::Result;
use serde::Deserialize;

/// Sub-epsilon CRAP deltas are float noise, not regressions.
const EPSILON: f64 = 0.01;

/// A function whose CRAP score got meaningfully worse between old and new.
#[derive(Clone, Debug, PartialEq)]
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

type Key = (String, String, String, i64);

fn key(e: &Entry) -> Key {
    (
        e.crate_field.clone(),
        e.file.clone(),
        e.function.clone(),
        e.line,
    )
}

/// Compare a new CRAP report against the old manifest. Returns one
/// [`CrapRegression`] per key present in both whose CRAP score worsened by more
/// than [`EPSILON`].
pub fn compare(new_report: &str, old_manifest: &str) -> Result<Vec<CrapRegression>> {
    let new: Report = serde_json::from_str(new_report)?;
    let old: Report = serde_json::from_str(old_manifest)?;

    let old_by_key: HashMap<Key, f64> = old.entries.iter().map(|e| (key(e), e.crap)).collect();

    let mut regressions = Vec::new();
    for e in &new.entries {
        if let Some(&old_crap) = old_by_key.get(&key(e)) {
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
    Ok(regressions)
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
}
