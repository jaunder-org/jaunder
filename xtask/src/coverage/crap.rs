//! CRAP threshold gate over the CRAP report `devtool coverage emit` produces.
//! Any function whose CRAP score exceeds a fixed threshold FAILS, unless an
//! in-source `crap:allow` override within the function's span waives it. Stateless
//! — there is no committed manifest or history comparison.

use anyhow::Result;
use proc_macro2::Span;
use serde::{Deserialize, Serialize};
use syn::spanned::Spanned;

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
            .is_some_and(|src| allow_overrides(&src, e.line, &e.file));
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

/// 1-based `(start_line, end_line)` — attributes/doc through the closing brace — for
/// every function in `src` (free fns, impl methods, trait default methods), including
/// nested fns (the visitor recurses). Mirrors `exempt.rs`'s syn visitor. An
/// over-threshold function is guaranteed to have a body, so it always yields a span.
fn fn_spans(src: &str) -> syn::Result<Vec<(usize, usize)>> {
    let file = syn::parse_file(src)?;
    let mut out = Vec::new();
    let mut v = FnSpanVisitor { out: &mut out };
    syn::visit::visit_file(&mut v, &file);
    Ok(out)
}

struct FnSpanVisitor<'a> {
    out: &'a mut Vec<(usize, usize)>,
}

/// `(start_line, end_line)`: the min of the fn's attribute + signature span starts
/// (so an outer doc/attribute header is included) through the block's closing brace.
fn bounds(attrs: &[syn::Attribute], sig: Span, block: Span) -> (usize, usize) {
    let start = attrs
        .iter()
        .map(|a| a.span().start().line)
        .chain(std::iter::once(sig.start().line))
        .min()
        // The chained `sig.start().line` guarantees a non-empty iterator, so `min`
        // is always `Some`; the fallback is unreachable but keeps this panic-free.
        .unwrap_or_else(|| sig.start().line);
    (start, block.end().line)
}

impl<'ast> syn::visit::Visit<'ast> for FnSpanVisitor<'_> {
    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        self.out
            .push(bounds(&f.attrs, f.sig.span(), f.block.span()));
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.out
            .push(bounds(&f.attrs, f.sig.span(), f.block.span()));
        syn::visit::visit_impl_item_fn(self, f);
    }
    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        if let Some(block) = &f.default {
            self.out.push(bounds(&f.attrs, f.sig.span(), block.span()));
        }
        syn::visit::visit_trait_item_fn(self, f);
    }
}

/// The innermost (smallest) span containing 1-based `line`, or `None`.
fn resolve_span(spans: &[(usize, usize)], line: usize) -> Option<(usize, usize)> {
    spans
        .iter()
        .filter(|&&(s, e)| s <= line && line <= e)
        .min_by_key(|&&(s, e)| e - s)
        .copied()
}

/// Does `src` carry a valid `crap:allow` override for the function that contains
/// cargo-crap's reported (1-based) `line`? The function is the innermost parsed span
/// containing that line; the marker is searched only among lines that belong to *that*
/// function (excluding the interior of any nested fn, so a nested marker cannot leak
/// out). Fail-closed (no override) with a warning on a parse failure or a line no
/// function span contains — neither happens for the project's own compiling sources.
fn allow_overrides(src: &str, line: i64, file: &str) -> bool {
    let line = line.max(1) as usize;
    let spans = match fn_spans(src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("xtask: coverage: crap:allow span skipped — {file} did not parse: {e}");
            return false;
        }
    };
    let Some((start, end)) = resolve_span(&spans, line) else {
        eprintln!("xtask: coverage: crap:allow — no function span contains {file}:{line}");
        return false;
    };
    let lines: Vec<&str> = src.lines().collect();
    // Scan only lines whose own innermost span is this function — excludes nested-fn
    // interiors, so a nested fn's marker cannot waive its enclosing fn (and vice versa).
    (start..=end.min(lines.len()))
        .filter(|&ln| resolve_span(&spans, ln) == Some((start, end)))
        .any(|ln| is_allow_marker(lines[ln - 1]))
}

/// A line is a valid override marker iff it contains `// crap:allow:` followed by
/// a non-empty (trimmed) reason.
fn is_allow_marker(line: &str) -> bool {
    match line.find(CRAP_ALLOW_MARKER) {
        Some(pos) => !line[pos + CRAP_ALLOW_MARKER.len()..].trim().is_empty(),
        None => false,
    }
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

    /// Run `evaluate_crap` over one over-threshold entry with `src` injected, and
    /// return whether it was WAIVED (absent from the fail list).
    fn waived(src: &'static str, line: i64) -> bool {
        let entries = vec![ent("a.rs", "f", line, 99.0)];
        let allow = AllowSet::new(move |_| Some(src.to_string()));
        evaluate_crap(&entries, &allow).is_empty()
    }

    #[test]
    fn no_bleed_from_preceding_function() {
        // crap:allow belongs to g (above); it must NOT waive f. Within the old 12-line window.
        let src = "\
fn g() {
    // crap:allow: for g only
}
fn f() {
    body();
}
";
        assert!(!waived(src, 4)); // f starts at line 4; g's marker must not reach it
    }

    #[test]
    fn no_bleed_across_nested_functions() {
        // marker in inner must not waive outer, and vice versa (innermost-containing rule).
        let src = "\
fn outer() {
    fn inner() {
        // crap:allow: inner only
        x();
    }
    y();
}
";
        assert!(!waived(src, 1)); // outer (line 1) not waived by inner's marker
        assert!(waived(src, 2)); // inner (line 2) IS waived by its own marker

        // Reverse direction (AC3b): a marker in OUTER's body must not waive INNER.
        let src2 = "\
fn outer() {
    fn inner() {
        x();
    }
    // crap:allow: outer only
    y();
}
";
        assert!(!waived(src2, 2)); // inner (line 2) not waived by outer's marker
        assert!(waived(src2, 1)); // outer (line 1) waived by its own marker
    }

    #[test]
    fn brace_in_string_or_comment_does_not_misbound() {
        // A `}` in a string/comment must not end the span early; a marker AFTER the real
        // body must not waive; syn spans make this automatic.
        let src = "\
fn f() {
    let s = \"}\";
    body();
}
// crap:allow: this is OUTSIDE f
fn g() { z(); }
";
        assert!(!waived(src, 1)); // f not waived by the marker below its real close
    }

    #[test]
    fn reported_line_inside_doc_header_resolves() {
        // AC5(i): cargo-crap points into the doc-comment header (contained via #[doc]).
        let src = "\
/// doc line one
/// doc line two
/// doc line three
fn run() {
    // crap:allow: reason
    a();
}
";
        assert!(waived(src, 2)); // reported inside the doc header → contained → waived
    }

    #[test]
    fn no_containing_span_is_fail_closed() {
        // AC5(iii): a reported line outside every function span → no override (fails).
        let src = "\
const X: u32 = 1;
fn f() { a(); }
";
        assert!(!waived(src, 1)); // line 1 is in no fn span → fail-closed
    }

    #[test]
    fn resolve_span_picks_innermost() {
        // outer [1..7], inner [2..5]; line 3 → inner.
        let spans = vec![(1usize, 7usize), (2, 5)];
        assert_eq!(resolve_span(&spans, 3), Some((2, 5)));
        assert_eq!(resolve_span(&spans, 6), Some((1, 7)));
        assert_eq!(resolve_span(&spans, 9), None);
    }
}
