//! JSONL parsing for the OTel trace analyzer (`cargo xtask traces analyze`).
//!
//! Reads the OpenTelemetry JSONL the e2e VM collector exports and flattens it to
//! a `Vec<Span>`. Port of the parsing half of `scripts/analyze-otel-traces`:
//! `getAttr`/`parseDurationMs`/`readSpans`. The heavier per-attribute JSON helpers
//! (`parse_json_attr`, `to_url_path`) land alongside their first callers in the
//! hotspot sections.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

/// A single flattened span with the scalar fields the reports read, its e2e
/// project, and the raw span object for on-demand `e2e.*_json` reads (only
/// `e2e.test` spans carry those, so they are parsed lazily by the sections that
/// need them). Node keeps `spanId`/`parentSpanId` too, but no report reads them,
/// so they are omitted.
#[derive(Debug, Clone)]
pub struct Span {
    pub trace_id: String,
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

/// The two span filters `traces analyze` accepts.
#[derive(Debug, Default, Clone)]
pub struct Filters {
    pub trace: Option<String>,
    pub project: Option<String>,
}

/// Read a string attribute from a span's `attributes[]` list: `stringValue` if
/// present, else the stringified `intValue` (OTel encodes int64 as either a JSON
/// number or a string), else `""`. Mirrors Node `getAttr` (:70-83).
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
/// `BigInt`) and divide by 1e6. A missing/unparseable field yields `0.0`.
pub fn parse_duration_ms(span: &Value) -> f64 {
    let nanos = |k: &str| -> i128 {
        span.get(k)
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i128>().ok())
            .or_else(|| span.get(k).and_then(Value::as_i64).map(i128::from))
            .unwrap_or(0)
    };
    let delta = nanos("endTimeUnixNano") - nanos("startTimeUnixNano");
    delta as f64 / 1_000_000.0
}

/// Whether `span` passes the filters. Trace filter: drop when `traceId` differs.
/// Project filter: drop **only** an `e2e.`-named span whose `e2e.project` differs
/// — HTTP/server spans always pass (Node `readSpans` :131-142).
fn passes(span: &Value, name: &str, project: &str, filters: &Filters) -> bool {
    if let Some(trace) = &filters.trace {
        if span.get("traceId").and_then(Value::as_str).unwrap_or("") != trace {
            return false;
        }
    }
    if let Some(want) = &filters.project {
        if name.starts_with("e2e.") && project != want {
            return false;
        }
    }
    true
}

/// Parse JSONL `content` into spans, applying `filters`. `source` labels both the
/// parse-error context and each resulting `Span.source`. A malformed line is a
/// hard error (Node :113-117); blank lines are skipped.
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
    fn parse_duration_ms_from_unix_nanos() {
        let span = json!({ "startTimeUnixNano": "1000000", "endTimeUnixNano": "2500000" });
        assert_eq!(parse_duration_ms(&span), 1.5);
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
    fn parse_spans_malformed_line_is_hard_error() {
        let err = parse_spans("{bad json\n", &Filters::default(), "t").unwrap_err();
        assert!(
            err.to_string().contains('t'),
            "error names the source: {err}"
        );
    }

    #[test]
    fn parse_spans_empty_content_is_empty_vec() {
        assert!(parse_spans("", &Filters::default(), "t")
            .unwrap()
            .is_empty());
        assert!(parse_spans("\n  \n", &Filters::default(), "t")
            .unwrap()
            .is_empty());
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
