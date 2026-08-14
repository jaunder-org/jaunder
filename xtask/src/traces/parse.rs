//! JSONL parsing for the OTel trace analyzer (`cargo xtask traces analyze`).
//!
//! Reads the OpenTelemetry JSONL the e2e VM collector exports and flattens it to
//! a `Vec<Span>`. Port of the parsing half of `scripts/analyze-otel-traces`:
//! `getAttr`/`parseDurationMs`/`readSpans`. The heavier per-attribute JSON helpers
//! (`parse_json_attr`, `to_url_path`) land alongside their first callers in the
//! hotspot sections.

use std::{fmt, path::Path};

use anyhow::{Context, Result};
use serde_json::Value;
use url::Url;

/// A single flattened span with the scalar fields the reports read, its e2e
/// project, and the raw span object for on-demand `e2e.*_json` reads (only
/// `e2e.test` spans carry those, so they are parsed lazily by the sections that
/// need them).
///
/// `span_id`/`parent_span_id` were originally omitted (no report read them), but
/// the flow-coverage gate (#681) walks `parent_span_id` upward from a server-fn
/// hit to the `e2e.test` span that caused it — an instrument span's parent is the
/// request span, not the test — so the identity edges are retained. Absent ids are
/// the empty string, matching the other string-typed fields.
#[derive(Debug, Clone)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub method: String,
    pub uri: String,
    pub project: String,
    pub busy_ns: String,
    pub idle_ns: String,
    pub duration_ms: f64,
    pub source: String,
    pub raw: Value,
}

/// A span's `[start, end)` in milliseconds, read from the raw OTLP nanosecond
/// fields.
///
/// `Span` carries only `duration_ms`, which is enough to rank but not to take a
/// union: two 100 ms children of one parent cover 200 ms if disjoint and 100 ms
/// if they overlap, and only the timestamps distinguish those. The span-coverage
/// section needs the union, so it needs these (#794).
///
/// Returns `None` when either bound is absent or unparseable, so a malformed span
/// is excluded from coverage rather than silently contributing a zero-length or
/// wildly-wrong interval.
pub fn span_interval_ms(raw: &Value) -> Option<(f64, f64)> {
    let nanos = |key: &str| -> Option<f64> {
        let value = raw.get(key)?;
        // OTLP encodes uint64 as either a JSON string or a number.
        value
            .as_str()
            .and_then(|text| text.parse::<f64>().ok())
            .or_else(|| value.as_f64())
    };
    let start = nanos("startTimeUnixNano")? / 1_000_000.0;
    let end = nanos("endTimeUnixNano")? / 1_000_000.0;
    if !start.is_finite() || !end.is_finite() || end < start {
        return None;
    }
    Some((start, end))
}

/// Total length covered by `intervals`, counting overlaps once.
///
/// Sorting by start and merging is what makes this a union rather than a sum —
/// the distinction the section depends on, since `e2e.test` and an `e2e.page`
/// span for a concurrently-driven context genuinely overlap in time.
pub fn interval_union_ms(mut intervals: Vec<(f64, f64)>) -> f64 {
    if intervals.is_empty() {
        return 0.0;
    }
    intervals.sort_by(|left, right| {
        left.0
            .partial_cmp(&right.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut total = 0.0;
    let (mut current_start, mut current_end) = intervals[0];
    for &(start, end) in &intervals[1..] {
        if start > current_end {
            total += current_end - current_start;
            current_start = start;
            current_end = end;
        } else if end > current_end {
            current_end = end;
        }
    }
    total + (current_end - current_start)
}

/// The two span filters `traces analyze` accepts.
#[derive(Debug, Default, Clone)]
pub struct Filters {
    pub trace: Option<String>,
    pub project: Option<String>,
}

/// Read a string attribute from a span's `attributes[]` list: `stringValue` if
/// present, else the stringified `intValue` (OTel encodes int64 as either a JSON
/// number or a string), else `""`.
pub fn get_attr(span: &Value, key: &str) -> String {
    let Some(attrs) = span.get("attributes").and_then(Value::as_array) else {
        return String::new();
    };
    for attr in attrs {
        if attr.get("key").and_then(Value::as_str) != Some(key) {
            continue;
        }
        let Some(value) = attr.get("value") else {
            return String::new();
        };
        if let Some(s) = value.get("stringValue").and_then(Value::as_str) {
            return s.to_string();
        }
        if let Some(iv) = value.get("intValue") {
            if let Some(n) = iv.as_i64() {
                return n.to_string();
            }
            if let Some(s) = iv.as_str() {
                return s.to_string();
            }
        }
        return String::new();
    }
    String::new()
}

/// Span duration in milliseconds from `(endTimeUnixNano - startTimeUnixNano)`.
/// The nano fields are int64 encoded as JSON strings; parse as `i128` (Node uses
/// `BigInt`) and divide by 1e6. A missing/unparseable field yields `0.0` (Node's
/// `BigInt` throws and aborts the whole run here — we deliberately degrade instead
/// of aborting). `saturating_sub` guards the subtraction: real u64-range nanos
/// never saturate, but a crafted ~39-digit value can't panic the tool.
pub fn parse_duration_ms(span: &Value) -> f64 {
    let nanos = |k: &str| -> i128 {
        span.get(k)
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i128>().ok())
            .or_else(|| span.get(k).and_then(Value::as_i64).map(i128::from))
            .unwrap_or(0)
    };
    let delta = nanos("endTimeUnixNano").saturating_sub(nanos("startTimeUnixNano"));
    delta as f64 / 1_000_000.0
}

/// A present `e2e.*_json` attribute whose string is not valid JSON.
///
/// This dedicated type is the dispatch boundary: only this correctness failure
/// becomes a failed trace `StepResult`; file, report, JSONL, and Nix failures keep
/// the trace command's established top-level error contract.
#[derive(Debug)]
pub struct MalformedJsonAttr {
    key: String,
    span_source: String,
    source: serde_json::Error,
}
impl fmt::Display for MalformedJsonAttr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "parsing JSON trace attribute {} from {}",
            self.key, self.span_source
        )
    }
}
impl std::error::Error for MalformedJsonAttr {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Parse a JSON-string attribute (the `e2e.*_json` blobs). Absence is optional;
/// malformed present JSON is a correctness error retaining the serde source.
pub fn parse_json_attr(span: &Value, key: &str, span_source: &str) -> Result<Option<Value>> {
    let present = span
        .get("attributes")
        .and_then(Value::as_array)
        .is_some_and(|attrs| {
            attrs
                .iter()
                .any(|attr| attr.get("key").and_then(Value::as_str) == Some(key))
        });
    if !present {
        return Ok(None);
    }
    let raw = get_attr(span, key);
    serde_json::from_str(&raw).map(Some).map_err(|source| {
        MalformedJsonAttr {
            key: key.to_owned(),
            span_source: span_source.to_owned(),
            source,
        }
        .into()
    })
}

/// Normalize a URL to `host[:port]/path`:
/// a parseable URL → `host_str` + the non-default `:port` + `path` (always at
/// least `/`); an unparseable value → the raw string; empty → `""`.
pub fn to_url_path(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    match Url::parse(value) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("");
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            format!("{host}{port}{}", url.path())
        }
        Err(_) => value.to_string(),
    }
}

/// Whether `span` passes the filters. Trace filter: drop when `traceId` differs.
/// Project filter: drop **only** an `e2e.`-named span whose `e2e.project` differs
/// — HTTP/server spans always pass.
fn passes(span: &Value, name: &str, project: &str, filters: &Filters) -> bool {
    if let Some(trace) = &filters.trace
        && span.get("traceId").and_then(Value::as_str).unwrap_or("") != trace
    {
        return false;
    }
    if let Some(want) = &filters.project
        && name.starts_with("e2e.")
        && project != want
    {
        return false;
    }
    true
}

/// Parse JSONL `content` into spans, applying `filters`. `source` labels both the
/// parse-error context and each resulting `Span.source`. A malformed line is a
/// hard error; blank lines are skipped.
pub fn parse_spans(content: &str, filters: &Filters, source: &str) -> Result<Vec<Span>> {
    let mut spans = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value = serde_json::from_str(line)
            .with_context(|| format!("failed to parse JSON line in {source}"))?;
        let resource_spans = record
            .get("resourceSpans")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for resource_span in &resource_spans {
            let scope_spans = resource_span
                .get("scopeSpans")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for scope_span in &scope_spans {
                let nested = scope_span
                    .get("spans")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for span in nested {
                    let name = span
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let project = get_attr(&span, "e2e.project");
                    if !passes(&span, &name, &project, filters) {
                        continue;
                    }
                    spans.push(Span {
                        trace_id: span
                            .get("traceId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        span_id: span
                            .get("spanId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        parent_span_id: span
                            .get("parentSpanId")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        method: get_attr(&span, "method"),
                        uri: get_attr(&span, "uri"),
                        busy_ns: get_attr(&span, "busy_ns"),
                        idle_ns: get_attr(&span, "idle_ns"),
                        duration_ms: parse_duration_ms(&span),
                        source: source.to_string(),
                        name,
                        project,
                        raw: span,
                    });
                }
            }
        }
    }
    Ok(spans)
}

/// Read a file and `parse_spans` its content. Errors name the path (missing file,
/// unreadable, or a malformed line).
pub fn read_spans(path: &Path, filters: &Filters) -> Result<Vec<Span>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading trace file {}", path.display()))?;
    parse_spans(&content, filters, &path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(spans: Value) -> String {
        json!({ "resourceSpans": [{ "scopeSpans": [{ "spans": spans }] }] }).to_string()
    }

    #[test]
    fn interval_union_counts_overlap_once() {
        // The whole reason coverage is a union and not a sum: e2e.test and an
        // e2e.page span for a concurrently-driven context overlap in real runs.
        assert_eq!(interval_union_ms(vec![(0.0, 100.0), (50.0, 150.0)]), 150.0);
    }

    #[test]
    fn interval_union_sums_disjoint_and_ignores_order() {
        assert_eq!(interval_union_ms(vec![(200.0, 250.0), (0.0, 100.0)]), 150.0);
    }

    #[test]
    fn interval_union_absorbs_a_contained_interval() {
        assert_eq!(interval_union_ms(vec![(0.0, 100.0), (25.0, 50.0)]), 100.0);
    }

    #[test]
    fn interval_union_of_nothing_is_zero() {
        assert_eq!(interval_union_ms(vec![]), 0.0);
    }

    #[test]
    fn span_interval_reads_string_and_numeric_nanos() {
        let as_string = json!({
            "startTimeUnixNano": "1000000",
            "endTimeUnixNano": "3000000",
        });
        assert_eq!(span_interval_ms(&as_string), Some((1.0, 3.0)));
        let as_number = json!({
            "startTimeUnixNano": 1_000_000u64,
            "endTimeUnixNano": 3_000_000u64,
        });
        assert_eq!(span_interval_ms(&as_number), Some((1.0, 3.0)));
    }

    #[test]
    fn span_interval_rejects_missing_or_inverted_bounds() {
        // Excluded from coverage rather than contributing a bogus interval.
        assert_eq!(span_interval_ms(&json!({})), None);
        assert_eq!(
            span_interval_ms(&json!({
                "startTimeUnixNano": "3000000",
                "endTimeUnixNano": "1000000",
            })),
            None,
        );
    }

    #[test]
    fn get_attr_string_then_int_then_empty() {
        let span = json!({
            "attributes": [
                { "key": "method", "value": { "stringValue": "GET" } },
                { "key": "n", "value": { "intValue": 42 } },
                { "key": "s", "value": { "intValue": "99" } },
            ]
        });
        assert_eq!(get_attr(&span, "method"), "GET");
        assert_eq!(get_attr(&span, "n"), "42");
        assert_eq!(get_attr(&span, "s"), "99");
        assert_eq!(get_attr(&span, "missing"), "");
    }

    #[test]
    fn trace_json_attr_distinguishes_absent_malformed_and_valid() {
        let with =
            |s: &str| json!({ "attributes": [{ "key": "e2e.x", "value": { "stringValue": s } }] });
        assert_eq!(
            parse_json_attr(&json!({}), "e2e.x", "source").unwrap(),
            None
        );

        let error = parse_json_attr(&with("{not json"), "e2e.x", "source.jsonl").unwrap_err();
        assert!(format!("{error:#}").contains("e2e.x"));
        let malformed = error.downcast_ref::<MalformedJsonAttr>().unwrap();
        assert_eq!(malformed.span_source, "source.jsonl");
        assert!(malformed.source.is_syntax());

        assert_eq!(
            parse_json_attr(&with("[1,2]"), "e2e.x", "source").unwrap(),
            Some(json!([1, 2]))
        );
    }

    #[test]
    fn to_url_path_cases() {
        assert_eq!(to_url_path("https://h:8080/a/b?q=1"), "h:8080/a/b");
        assert_eq!(to_url_path("not a url"), "not a url");
        assert_eq!(to_url_path(""), "");
    }

    #[test]
    fn parse_duration_ms_from_unix_nanos() {
        let span = json!({ "startTimeUnixNano": "1000000", "endTimeUnixNano": "2500000" });
        assert_eq!(parse_duration_ms(&span), 1.5);
    }

    #[test]
    fn parse_duration_ms_saturates_instead_of_panicking() {
        // Crafted extreme nanos (i128::MAX minus i128::MIN) must not overflow-panic;
        // saturating_sub clamps and we still return a finite ms. Real u64 nanos are
        // nowhere near this and are unaffected.
        let span = json!({
            "endTimeUnixNano": "170141183460469231731687303715884105727",
            "startTimeUnixNano": "-170141183460469231731687303715884105728",
        });
        assert!(parse_duration_ms(&span).is_finite());
    }

    #[test]
    fn parse_spans_walks_resource_scope_spans() {
        let content = line(json!([
            { "traceId": "aa", "name": "a" },
            { "traceId": "aa", "name": "b" },
        ]));
        let spans = parse_spans(&content, &Filters::default(), "sample").unwrap();
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|s| s.source == "sample"));
    }

    #[test]
    fn span_ids_are_parsed() {
        // The flow-coverage gate (#681) joins on these edges.
        let content = line(json!([
            { "traceId": "aa", "spanId": "bb", "parentSpanId": "cc", "name": "request" },
        ]));
        let spans = parse_spans(&content, &Filters::default(), "t").unwrap();
        assert_eq!(spans[0].span_id, "bb");
        assert_eq!(spans[0].parent_span_id, "cc");
    }

    #[test]
    fn missing_parent_span_id_is_empty_not_an_error() {
        // A root span legitimately has no parent; that must not fail parsing.
        let content = line(json!([
            { "traceId": "aa", "spanId": "bb", "name": "request" },
        ]));
        let spans = parse_spans(&content, &Filters::default(), "t").unwrap();
        assert_eq!(spans[0].parent_span_id, "");
    }

    #[test]
    fn parse_spans_malformed_line_is_hard_error() {
        let err = parse_spans("{bad json\n", &Filters::default(), "t").unwrap_err();
        assert!(
            err.to_string().contains('t'),
            "error names the source: {err}"
        );
    }

    #[test]
    fn parse_spans_empty_content_is_empty_vec() {
        assert!(
            parse_spans("", &Filters::default(), "t")
                .unwrap()
                .is_empty()
        );
        assert!(
            parse_spans("\n  \n", &Filters::default(), "t")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn parse_spans_trace_filter() {
        let content = line(json!([
            { "traceId": "aa", "name": "a" },
            { "traceId": "bb", "name": "b" },
        ]));
        let filters = Filters {
            trace: Some("aa".into()),
            project: None,
        };
        let spans = parse_spans(&content, &filters, "t").unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].trace_id, "aa");
    }

    #[test]
    fn parse_spans_project_filter_only_affects_e2e_spans() {
        let e2e = |proj: &str| {
            json!({
                "name": "e2e.test",
                "attributes": [{ "key": "e2e.project", "value": { "stringValue": proj } }]
            })
        };
        let http = json!({
            "name": "GET",
            "attributes": [{ "key": "method", "value": { "stringValue": "GET" } }]
        });
        let content = line(json!([e2e("firefox"), e2e("chromium"), http]));
        let filters = Filters {
            trace: None,
            project: Some("firefox".into()),
        };
        let spans = parse_spans(&content, &filters, "t").unwrap();
        // firefox e2e.test kept, chromium e2e.test dropped, HTTP span always kept:
        // exactly one e2e.test survives (the firefox one) plus the GET span.
        assert_eq!(spans.len(), 2);
        assert_eq!(spans.iter().filter(|s| s.name == "e2e.test").count(), 1);
        assert!(spans.iter().any(|s| s.name == "GET"));
    }

    #[test]
    fn read_spans_file_not_found_errors() {
        let err = read_spans(Path::new("/no/such/trace.jsonl"), &Filters::default()).unwrap_err();
        assert!(
            err.to_string().contains("/no/such/trace.jsonl"),
            "names the path: {err}"
        );
    }
}
