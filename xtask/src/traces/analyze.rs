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
use serde_json::Value;

use super::parse::{get_attr, read_spans, Filters, Span};

/// Parse an `e2e.*` integer-count attribute (`0` when absent/non-numeric),
/// matching Node's `Number(getAttr(...) || "0")`.
fn count(raw: &Value, key: &str) -> u64 {
    get_attr(raw, key).parse().unwrap_or(0)
}

/// The e2e project label a report groups on: the span's `e2e.project`, or `-`
/// when unset (Node's `getAttr(...) || "-"`).
fn project_label(project: &str) -> String {
    if project.is_empty() {
        "-".to_string()
    } else {
        project.to_string()
    }
}

/// Every report section, as typed rows. Grown additively across the port; unbuilt
/// sections stay empty via `Default`.
#[derive(Debug, Default)]
pub struct Analysis {
    pub span_count: usize,
    pub project_filter: Option<String>,
    /// All spans, sorted by `duration_ms` descending (not sliced). Section 1.
    pub slowest_spans: Vec<SlowSpanRow>,
    /// `e2e.test` spans, slowest first. Section 2.
    pub slowest_e2e_tests: Vec<E2eTestRow>,
    /// `e2e.test` durations grouped by project, slowest average first. All rows
    /// (not sliced). Section 11.
    pub by_project: Vec<ByProjectRow>,
    /// Per-trace span-duration totals, largest first. Section 12.
    pub trace_totals: Vec<TraceTotalRow>,
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

/// One row of "slowest e2e.test spans" (Node `printSlowestE2eTests` :216-249).
#[derive(Debug, Clone)]
pub struct E2eTestRow {
    pub duration_ms: f64,
    pub project: String,
    pub actions: u64,
    pub requests: u64,
    pub trace_id: String,
    pub test: String,
}

/// One row of "E2E test duration by project" (Node `printE2eByProject` :1017-1067).
#[derive(Debug, Clone)]
pub struct ByProjectRow {
    pub project: String,
    pub tests: usize,
    pub avg_ms: f64,
    pub max_ms: f64,
    pub avg_actions: f64,
    pub avg_requests: f64,
}

/// One row of "Trace totals" (Node `printTraceTotals` :1070-1096).
#[derive(Debug, Clone)]
pub struct TraceTotalRow {
    pub trace_id: String,
    pub total_ms: f64,
    pub spans: usize,
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

/// Look up (or first-insert) the accumulator for `key` in an insertion-ordered
/// `Vec` of `(key, acc)`. First-seen order mirrors the JS `Map` the Node script
/// groups with, so tie-order in the sorted output matches. Group counts are tiny
/// (projects, traces), so linear search is fine.
fn entry<'a, V>(groups: &'a mut Vec<(String, V)>, key: &str, init: impl Fn() -> V) -> &'a mut V {
    if let Some(idx) = groups.iter().position(|(k, _)| k == key) {
        &mut groups[idx].1
    } else {
        groups.push((key.to_string(), init()));
        &mut groups.last_mut().unwrap().1
    }
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

    // Section 2 — slowest e2e.test spans.
    let mut slowest_e2e_tests: Vec<E2eTestRow> = spans
        .iter()
        .filter(|s| s.name == "e2e.test")
        .map(|s| E2eTestRow {
            duration_ms: s.duration_ms,
            project: project_label(&s.project),
            actions: count(&s.raw, "e2e.action_count"),
            requests: count(&s.raw, "e2e.request_count"),
            trace_id: s.trace_id.clone(),
            test: {
                let t = get_attr(&s.raw, "e2e.test");
                if t.is_empty() {
                    "-".to_string()
                } else {
                    t
                }
            },
        })
        .collect();
    sort_desc_by(&mut slowest_e2e_tests, |r| r.duration_ms);

    // Section 11 — e2e.test duration grouped by project.
    #[derive(Default)]
    struct ProjAgg {
        tests: usize,
        total_ms: f64,
        max_ms: f64,
        actions: u64,
        requests: u64,
    }
    let mut proj_groups: Vec<(String, ProjAgg)> = Vec::new();
    for s in spans.iter().filter(|s| s.name == "e2e.test") {
        let a = entry(
            &mut proj_groups,
            &project_label(&s.project),
            ProjAgg::default,
        );
        a.tests += 1;
        a.total_ms += s.duration_ms;
        a.max_ms = a.max_ms.max(s.duration_ms);
        a.actions += count(&s.raw, "e2e.action_count");
        a.requests += count(&s.raw, "e2e.request_count");
    }
    let mut by_project: Vec<ByProjectRow> = proj_groups
        .into_iter()
        .map(|(project, a)| ByProjectRow {
            project,
            tests: a.tests,
            avg_ms: a.total_ms / a.tests as f64,
            max_ms: a.max_ms,
            avg_actions: a.actions as f64 / a.tests as f64,
            avg_requests: a.requests as f64 / a.tests as f64,
        })
        .collect();
    sort_desc_by(&mut by_project, |r| r.avg_ms);

    // Section 12 — per-trace duration totals (all spans).
    #[derive(Default)]
    struct TraceAgg {
        total_ms: f64,
        spans: usize,
    }
    let mut trace_groups: Vec<(String, TraceAgg)> = Vec::new();
    for s in &spans {
        let a = entry(&mut trace_groups, &s.trace_id, TraceAgg::default);
        a.total_ms += s.duration_ms;
        a.spans += 1;
    }
    let mut trace_totals: Vec<TraceTotalRow> = trace_groups
        .into_iter()
        .map(|(trace_id, a)| TraceTotalRow {
            trace_id,
            total_ms: a.total_ms,
            spans: a.spans,
        })
        .collect();
    sort_desc_by(&mut trace_totals, |r| r.total_ms);

    Analysis {
        span_count: spans.len(),
        project_filter,
        slowest_spans,
        slowest_e2e_tests,
        by_project,
        trace_totals,
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
    fn slowest_e2e_tests_only_e2e_test_spans() {
        let a = analyze_spans(fixture_spans(), None);
        // Two e2e.test spans in the fixture; the HTTP spans are excluded.
        assert_eq!(a.slowest_e2e_tests.len(), 2);
        // Slowest first: firefox (5000ms) then chromium (3000ms).
        let first = &a.slowest_e2e_tests[0];
        assert_eq!(first.project, "firefox");
        assert_eq!(first.duration_ms, 5000.0);
        assert_eq!(first.actions, 40);
        assert_eq!(first.requests, 12);
        assert_eq!(first.test, "timeline heavy");
        assert_eq!(a.slowest_e2e_tests[1].project, "chromium");
    }

    #[test]
    fn by_project_groups_and_averages() {
        let a = analyze_spans(fixture_spans(), None);
        // One row per project, each with a single test; sorted by avg_ms desc.
        assert_eq!(a.by_project.len(), 2);
        let ff = &a.by_project[0];
        assert_eq!(ff.project, "firefox");
        assert_eq!(ff.tests, 1);
        assert_eq!(ff.avg_ms, 5000.0);
        assert_eq!(ff.max_ms, 5000.0);
        assert_eq!(ff.avg_actions, 40.0);
        assert_eq!(ff.avg_requests, 12.0);
        assert_eq!(a.by_project[1].project, "chromium");
        assert_eq!(a.by_project[1].avg_ms, 3000.0);
    }

    #[test]
    fn trace_totals_sum_per_trace() {
        let a = analyze_spans(fixture_spans(), None);
        assert_eq!(a.trace_totals.len(), 2);
        // Trace 1: e2e.test 5000 + GET 200 = 5200 (2 spans); largest first.
        let t1 = &a.trace_totals[0];
        assert_eq!(t1.total_ms, 5200.0);
        assert_eq!(t1.spans, 2);
        // Trace 2: e2e.test 3000 + POST 150 = 3150.
        assert_eq!(a.trace_totals[1].total_ms, 3150.0);
        assert_eq!(a.trace_totals[1].spans, 2);
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
