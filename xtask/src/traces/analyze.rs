//! Trace analysis: `Vec<Span>` → a typed [`Analysis`] of every report section.
//!
//! The reusable in-crate seam (ADR-0028 host analyzer). `analyze_spans` does the
//! whole computation with no I/O, so #33's `traces run` can call it in-process on
//! trace files it collected, and unit tests can drive it from a fixture without
//! spawning a process. Rows are **fully sorted** here; `--top` slicing is
//! [`super::render`]'s job. Port of the twelve `print*` functions in
//! `scripts/analyze-otel-traces`.

use std::path::PathBuf;

use anyhow::Result;

use super::parse::{read_spans, Filters, Span};

/// Every report section, as typed rows. Grown additively across the port; unbuilt
/// sections stay empty via `Default`.
#[derive(Debug, Default)]
pub struct Analysis {
    pub span_count: usize,
    pub project_filter: Option<String>,
    /// All spans, sorted by `duration_ms` descending (not sliced). Section 1.
    pub slowest_spans: Vec<SlowSpanRow>,
}

/// One row of the "slowest spans" table (Node `printSlowest` :189-214).
#[derive(Debug, Clone)]
pub struct SlowSpanRow {
    pub duration_ms: f64,
    pub trace_id: String,
    pub name: String,
    pub method: String,
    pub uri: String,
    pub busy_ns: String,
    pub idle_ns: String,
    pub source: String,
}

/// Sort a `f64`-keyed vector descending, treating the key as a total order (NaN
/// sinks to the end). Used by every ranked section.
fn sort_desc_by<T>(rows: &mut [T], key: impl Fn(&T) -> f64) {
    rows.sort_by(|a, b| {
        key(b)
            .partial_cmp(&key(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Compute the whole [`Analysis`] from already-parsed spans. No I/O.
pub fn analyze_spans(spans: Vec<Span>, project_filter: Option<String>) -> Analysis {
    let mut slowest_spans: Vec<SlowSpanRow> = spans
        .iter()
        .map(|s| SlowSpanRow {
            duration_ms: s.duration_ms,
            trace_id: s.trace_id.clone(),
            name: s.name.clone(),
            method: s.method.clone(),
            uri: s.uri.clone(),
            busy_ns: s.busy_ns.clone(),
            idle_ns: s.idle_ns.clone(),
            source: s.source.clone(),
        })
        .collect();
    sort_desc_by(&mut slowest_spans, |r| r.duration_ms);

    Analysis {
        span_count: spans.len(),
        project_filter,
        slowest_spans,
    }
}

/// Read + parse every input, then analyze. `filters.project` is carried into
/// `Analysis.project_filter` for the render header.
pub fn analyze(inputs: &[PathBuf], filters: Filters) -> Result<Analysis> {
    let mut spans = Vec::new();
    for input in inputs {
        spans.extend(read_spans(input, &filters)?);
    }
    Ok(analyze_spans(spans, filters.project))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::parse_spans;

    const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");

    fn fixture_spans() -> Vec<Span> {
        parse_spans(FIXTURE, &Filters::default(), "sample").unwrap()
    }

    #[test]
    fn slowest_spans_sorted_desc_and_complete() {
        let spans = fixture_spans();
        let n = spans.len();
        assert!(n > 0, "fixture must have spans");
        let a = analyze_spans(spans, None);
        assert_eq!(a.span_count, n);
        // Every span present (not sliced), sorted by duration descending.
        assert_eq!(a.slowest_spans.len(), n);
        for pair in a.slowest_spans.windows(2) {
            assert!(
                pair[0].duration_ms >= pair[1].duration_ms,
                "not sorted desc"
            );
        }
    }

    #[test]
    fn analyze_reads_files() {
        let dir = std::env::temp_dir().join(format!("traces-analyze-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("otel-traces.jsonl");
        std::fs::write(&file, FIXTURE).unwrap();

        let via_file = analyze(&[file], Filters::default()).unwrap();
        let via_spans = analyze_spans(fixture_spans(), None);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(via_file.span_count, via_spans.span_count);
        assert_eq!(via_file.slowest_spans.len(), via_spans.slowest_spans.len());
        assert_eq!(
            via_file.slowest_spans.first().map(|r| r.duration_ms),
            via_spans.slowest_spans.first().map(|r| r.duration_ms),
        );
    }
}
