use std::collections::HashMap;

use super::super::{
    parse::{Span, get_attr, interval_union_ms, span_interval_ms},
    report::{AttemptKey, ReportedDurations},
};
use super::model::SpanCoverageRow;

/// The e2e project label a report groups on: the span's `e2e.project`, or `-`
/// when unset (Node's `getAttr(...) || "-"`).
fn project_label(project: &str) -> String {
    if project.is_empty() {
        "-".to_string()
    } else {
        project.to_string()
    }
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

/// Per-test span coverage: how much of each attempt's wall-clock lands inside a
/// named phase of the lifecycle tree (#794, AC-4).
///
/// Covered time is the interval **union of the envelope's children**, not the
/// envelope's own duration. The envelope spans the whole lifecycle by
/// construction, so measuring it against Playwright's duration would report ~100 %
/// coverage no matter how much time sat between the phases. The union of the named
/// phases is the honest numerator.
///
/// A test with no matching report entry is skipped rather than rendered with a
/// zero denominator — see `report::ReportedDurations`.
pub fn span_coverage(spans: &[Span], reported: &ReportedDurations) -> Vec<SpanCoverageRow> {
    let mut children: HashMap<&str, Vec<&Span>> = HashMap::new();
    for span in spans {
        if span.parent_span_id.is_empty() {
            continue;
        }
        children
            .entry(span.parent_span_id.as_str())
            .or_default()
            .push(span);
    }

    let mut rows: Vec<SpanCoverageRow> = spans
        .iter()
        .filter(|span| span.name == super::LIFECYCLE_SPAN_NAME)
        .filter_map(|envelope| {
            let key = AttemptKey {
                test: get_attr(&envelope.raw, "e2e.test"),
                project: get_attr(&envelope.raw, "e2e.project"),
                retry: get_attr(&envelope.raw, "e2e.retry")
                    .parse::<u64>()
                    .unwrap_or(0),
            };
            // Scoped by the envelope's own trace file: sqlite and postgres produce
            // identical (test, project, retry) keys with different durations.
            let reported_ms = reported.get(&envelope.source, &key)?;
            let intervals: Vec<(f64, f64)> = children
                .get(envelope.span_id.as_str())
                .map(|kids| {
                    kids.iter()
                        .filter_map(|kid| span_interval_ms(&kid.raw))
                        .collect()
                })
                .unwrap_or_default();
            let covered_ms = interval_union_ms(intervals);
            // Clamped: clock skew between the Node-side span stamps and
            // Playwright's own timing can put covered marginally above reported,
            // and a negative "uncovered" would read as nonsense.
            let uncovered_ms = (reported_ms - covered_ms).max(0.0);
            Some(SpanCoverageRow {
                project: project_label(&key.project),
                test: key.test,
                reported_ms,
                covered_ms,
                uncovered_ms,
                uncovered_pct: if reported_ms > 0.0 {
                    uncovered_ms / reported_ms * 100.0
                } else {
                    0.0
                },
            })
        })
        .collect();
    sort_desc_by(&mut rows, |row| row.uncovered_ms);
    rows
}

/// Explain an empty coverage section, so "no report supplied" is never mistaken
/// for "everything is attributed".
pub(super) fn coverage_note(
    spans: &[Span],
    reported: &ReportedDurations,
    coverage: &[SpanCoverageRow],
) -> Option<String> {
    if !coverage.is_empty() {
        return None;
    }
    if reported.is_empty() {
        return Some(
            "no --playwright-report supplied; per-test coverage needs Playwright's \
             own durations as the denominator"
                .to_owned(),
        );
    }
    if !spans.iter().any(|s| s.name == super::LIFECYCLE_SPAN_NAME) {
        return Some(format!(
            "no `{}` spans in the capture (pre-#794 traces have none)",
            super::LIFECYCLE_SPAN_NAME
        ));
    }
    Some(format!(
        "no lifecycle span matched a report entry ({} attempt(s) in the report)",
        reported.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::coverage_note;
    use crate::traces::analyze::{LIFECYCLE_SPAN_NAME, span_coverage};
    use crate::traces::parse::{Filters, Span, parse_spans};
    use crate::traces::report::ReportedDurations;

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

    fn attr(key: &str, value: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "key": key, "value": value })
    }

    /// One span at `[start_ms, end_ms)`, in the shape `parse_spans` reads.
    fn timed_span(
        name: &str,
        span_id: &str,
        parent: &str,
        start_ms: u64,
        end_ms: u64,
        extra: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let mut attributes = vec![attr(
            "e2e.project",
            serde_json::json!({ "stringValue": "chromium" }),
        )];
        attributes.extend(extra);
        serde_json::json!({
            "name": name,
            "spanId": span_id,
            "parentSpanId": parent,
            "startTimeUnixNano": (start_ms * 1_000_000).to_string(),
            "endTimeUnixNano": (end_ms * 1_000_000).to_string(),
            "attributes": attributes,
        })
    }

    /// A lifecycle envelope with two children that OVERLAP, plus the identity
    /// attributes the report join needs.
    fn lifecycle_tree() -> Vec<Span> {
        let identity = || {
            vec![
                attr("e2e.test", serde_json::json!({ "stringValue": "logs in" })),
                attr("e2e.retry", serde_json::json!({ "intValue": "0" })),
            ]
        };
        let line = serde_json::json!({
            "resourceSpans": [{
                "scopeSpans": [{
                    "spans": [
                        timed_span(LIFECYCLE_SPAN_NAME, "aa", "", 1_000, 1_500, identity()),
                        // 1000-1200 and 1150-1300 overlap by 50ms: union is 300,
                        // a naive sum would say 350.
                        timed_span("e2e.context_mint", "bb", "aa", 1_000, 1_200, vec![]),
                        timed_span("e2e.test", "cc", "aa", 1_150, 1_300, vec![]),
                    ]
                }]
            }]
        });
        parse_spans(&line.to_string(), &Filters::default(), "coverage").unwrap()
    }

    fn reported(ms: f64) -> ReportedDurations {
        ReportedDurations::from_value(&serde_json::json!({
            "suites": [{
                "specs": [{
                    "title": "logs in",
                    "tests": [{
                        "projectName": "chromium",
                        "results": [{ "retry": 0, "duration": ms }]
                    }]
                }]
            }]
        }))
    }

    #[test]
    fn span_coverage_unions_overlapping_children() {
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert_eq!(rows.len(), 1);
        // The point of the section: overlapping phases are counted once.
        assert_eq!(rows[0].covered_ms, 300.0);
        assert_eq!(rows[0].reported_ms, 500.0);
        assert_eq!(rows[0].uncovered_ms, 200.0);
        assert!((rows[0].uncovered_pct - 40.0).abs() < 1e-9);
    }

    #[test]
    fn span_coverage_measures_children_not_the_envelope() {
        // The envelope spans 1000-1500 by construction, so measuring IT would
        // report full coverage however much time sat between the phases.
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert!(
            rows[0].covered_ms < 500.0,
            "covered must come from the named phases, not the envelope",
        );
    }

    #[test]
    fn span_coverage_clamps_a_negative_remainder() {
        // Clock skew can put covered marginally above reported; a negative
        // "uncovered" would render as nonsense.
        let rows = span_coverage(&lifecycle_tree(), &reported(100.0));
        assert_eq!(rows[0].uncovered_ms, 0.0);
    }

    #[test]
    fn span_coverage_skips_a_test_absent_from_the_report() {
        let rows = span_coverage(&lifecycle_tree(), &ReportedDurations::default());
        assert!(
            rows.is_empty(),
            "no denominator means no row — never a zero-denominator row",
        );
    }

    #[test]
    fn coverage_note_distinguishes_no_report_from_no_lifecycle_spans() {
        // An empty section and a missing report must not look alike.
        let coverage = span_coverage(&lifecycle_tree(), &ReportedDurations::default());
        let no_report_note =
            coverage_note(&lifecycle_tree(), &ReportedDurations::default(), &coverage);
        assert!(
            no_report_note
                .as_deref()
                .unwrap()
                .contains("playwright-report")
        );

        let coverage = span_coverage(&fixture_spans(), &reported(500.0));
        let no_lifecycle_note = coverage_note(&fixture_spans(), &reported(500.0), &coverage);
        assert!(no_lifecycle_note.as_deref().unwrap().contains("lifecycle"));
    }

    #[test]
    fn coverage_note_is_absent_when_the_section_has_rows() {
        let coverage = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert!(coverage_note(&lifecycle_tree(), &reported(500.0), &coverage).is_none());
        assert_eq!(coverage.len(), 1);
    }

    #[test]
    fn retry_attempts_join_separately() {
        // Spans are exported per attempt; joining a retry's spans against attempt
        // 0's wall-clock would silently mis-state coverage.
        let rows = span_coverage(&lifecycle_tree(), &reported(500.0));
        assert_eq!(rows.len(), 1, "retry 0 matched exactly one report entry");

        let only_retry_one = ReportedDurations::from_value(&serde_json::json!({
            "suites": [{
                "specs": [{
                    "title": "logs in",
                    "tests": [{
                        "projectName": "chromium",
                        "results": [{ "retry": 1, "duration": 500.0 }]
                    }]
                }]
            }]
        }));
        assert!(
            span_coverage(&lifecycle_tree(), &only_retry_one).is_empty(),
            "a retry-0 span tree must not match a retry-1 report entry",
        );
    }
}
