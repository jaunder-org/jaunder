use super::super::parse::{self, Span};
use super::model::{ByProjectRow, E2eTestRow, SlowSpanRow, TraceTotalRow};

/// Parse an `e2e.*` integer-count attribute (`0` when absent/non-numeric),
/// matching Node's `Number(getAttr(...) || "0")`.
fn count(raw: &serde_json::Value, key: &str) -> u64 {
    parse::get_attr(raw, key).parse().unwrap_or(0)
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

/// The `e2e.test` name for a span, or `-` when unset (Node `getAttr(...) || "-"`).
fn e2e_test_name(s: &Span) -> String {
    let t = parse::get_attr(&s.raw, "e2e.test");
    if t.is_empty() { "-".to_string() } else { t }
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

pub(super) fn slowest_spans(spans: &[Span]) -> Vec<SlowSpanRow> {
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

    slowest_spans
}

pub(super) fn slowest_e2e_tests(spans: &[Span]) -> Vec<E2eTestRow> {
    let mut slowest_e2e_tests: Vec<E2eTestRow> = spans
        .iter()
        .filter(|s| s.name == "e2e.test")
        .map(|s| E2eTestRow {
            duration_ms: s.duration_ms,
            project: project_label(&s.project),
            actions: count(&s.raw, "e2e.action_count"),
            requests: count(&s.raw, "e2e.request_count"),
            trace_id: s.trace_id.clone(),
            test: e2e_test_name(s),
        })
        .collect();
    sort_desc_by(&mut slowest_e2e_tests, |r| r.duration_ms);

    slowest_e2e_tests
}

pub(super) fn by_project_rows(spans: &[Span]) -> Vec<ByProjectRow> {
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

    by_project
}

pub(super) fn trace_total_rows(spans: &[Span]) -> Vec<TraceTotalRow> {
    // Section 12 — per-trace duration totals (all spans).
    #[derive(Default)]
    struct TraceAgg {
        total_ms: f64,
        spans: usize,
    }
    let mut trace_groups: Vec<(String, TraceAgg)> = Vec::new();
    for s in spans {
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
    trace_totals
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::{Filters, parse_spans};

    const FIXTURE: &str = include_str!("../testdata/otel-traces-sample.jsonl");

    fn fixture_spans() -> Vec<Span> {
        let mut spans = parse_spans(FIXTURE, &Filters::default(), "sample").unwrap();
        for span in &mut spans {
            if span.project == "chromium"
                && let Some(attributes) = span.raw["attributes"].as_array_mut()
            {
                attributes.retain(|attribute| {
                    attribute["key"].as_str() != Some("e2e.navigation_top_json")
                });
            }
        }
        spans
    }

    #[test]
    fn slowest_spans_sorted_desc_and_complete() {
        let spans = fixture_spans();
        let n = spans.len();
        assert!(n > 0, "fixture must have spans");
        let rows = slowest_spans(&spans);
        assert_eq!(rows.len(), n);
        // Every span present (not sliced), sorted by duration descending.
        assert_eq!(rows.len(), n);
        for pair in rows.windows(2) {
            assert!(
                pair[0].duration_ms >= pair[1].duration_ms,
                "not sorted desc"
            );
        }
    }

    #[test]
    fn slowest_e2e_tests_only_e2e_test_spans() {
        let rows = slowest_e2e_tests(&fixture_spans());
        // Two e2e.test spans in the fixture; the HTTP spans are excluded.
        assert_eq!(rows.len(), 2);
        // Slowest first: firefox (5000ms) then chromium (3000ms).
        let first = &rows[0];
        assert_eq!(first.project, "firefox");
        assert_eq!(first.duration_ms, 5000.0);
        assert_eq!(first.actions, 40);
        assert_eq!(first.requests, 12);
        assert_eq!(first.test, "timeline heavy");
        assert_eq!(rows[1].project, "chromium");
    }

    #[test]
    fn by_project_groups_and_averages() {
        let rows = by_project_rows(&fixture_spans());
        // One row per project, each with a single test; sorted by avg_ms desc.
        assert_eq!(rows.len(), 2);
        let ff = &rows[0];
        assert_eq!(ff.project, "firefox");
        assert_eq!(ff.tests, 1);
        assert_eq!(ff.avg_ms, 5000.0);
        assert_eq!(ff.max_ms, 5000.0);
        assert_eq!(ff.avg_actions, 40.0);
        assert_eq!(ff.avg_requests, 12.0);
        assert_eq!(rows[1].project, "chromium");
        assert_eq!(rows[1].avg_ms, 3000.0);
    }

    #[test]
    fn trace_totals_sum_per_trace() {
        let rows = trace_total_rows(&fixture_spans());
        assert_eq!(rows.len(), 2);
        // Trace 1: e2e.test 5000 + GET 200 = 5200 (2 spans); largest first.
        let t1 = &rows[0];
        assert_eq!(t1.total_ms, 5200.0);
        assert_eq!(t1.spans, 2);
        // Trace 2: e2e.test 3000 + POST 150 = 3150.
        assert_eq!(rows[1].total_ms, 3150.0);
        assert_eq!(rows[1].spans, 2);
    }
}
