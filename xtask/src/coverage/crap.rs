//! CRAP threshold gate over the CRAP report `devtool coverage emit` produces.
//! Any function whose CRAP score exceeds a fixed threshold FAILS, unless an
//! in-source `crap:allow` override within the function's span waives it. Stateless
//! — there is no committed manifest or history comparison.

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct Report {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
pub struct Entry {
    #[serde(default)]
    file: String,
    #[serde(default)]
    function: String,
    #[serde(default)]
    line: i64,
    #[serde(default)]
    crap: f64,
}

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

/// Parse a CRAP report into its entries, so callers outside this module can run
/// [`evaluate_crap`] over them.
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

    fn ent(file: &str, function: &str, line: i64, crap: f64) -> Entry {
        Entry {
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
    fn parse_entries_reads_the_report() {
        let entries =
            parse_entries(r#"{"entries":[{"file":"a.rs","function":"f","line":1,"crap":2.0}]}"#)
                .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].function, "f");
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
