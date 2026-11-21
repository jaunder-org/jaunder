//! Refuse a capture whose server reported losing spans (#2029).
//!
//! The server exports spans through the OpenTelemetry SDK's `BatchSpanProcessor`.
//! It loses spans when its queue is full, when an export fails, and when its
//! shutdown flush times out. A lost `request` span whose server-fn child survived
//! makes that child an `unknown-parent` orphan, so a truncated capture looks
//! exactly like an attribution leak. The SDK records every such loss in the
//! server's WARN+ `diag.log`, which rides the same capture bundle; refusing on it
//! turns the loss into a named failure instead of a misleading snapshot diff.

use anyhow::{Result, bail};
use serde_json::Value;

/// The `tracing` target the SDK's internal diagnostics carry.
const SDK_TARGET: &str = "opentelemetry_sdk";
/// Every span-loss diagnostic the SDK's batch span processor emits is named under
/// this prefix (`SpanDroppingStarted`, `SpansDropped`, `ExportError`,
/// `Shutdown.Timeout`, …).
const PROCESSOR_PREFIX: &str = "BatchSpanProcessor.";

/// Fail when `diag_log` (one JSON record per line) holds an SDK span-loss record.
///
/// Lines that are not JSON are not SDK records and are ignored. The reported count
/// is a lower bound: export errors and shutdown timeouts lose spans the SDK's
/// `dropped_span_count` does not include.
pub fn refuse_span_loss(diag_log: &str) -> Result<()> {
    let mut names = Vec::new();
    let mut dropped: u64 = 0;
    for record in diag_log
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        let level = record["level"].as_str().unwrap_or_default();
        let name = record["fields"]["name"].as_str().unwrap_or_default();
        if record["target"] == SDK_TARGET
            && matches!(level, "WARN" | "ERROR")
            && name.starts_with(PROCESSOR_PREFIX)
        {
            names.push(name.to_owned());
            dropped = dropped.max(record["fields"]["dropped_span_count"].as_u64().unwrap_or(0));
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    let lost = if dropped == 0 {
        "an unknown number of".to_owned()
    } else {
        format!("at least {dropped}")
    };
    bail!(
        "the capture is incomplete: the server's OpenTelemetry SDK lost {lost} span(s) ({}), \
         which would surface as misattributed orphans — re-run the lane",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTED: &str = r#"{"timestamp":"2026-11-21T22:00:15.892302Z","level":"WARN","fields":{"name":"BatchSpanProcessor.SpanDroppingStarted","message":"BatchSpanProcessor dropped a Span due to queue full."},"target":"opentelemetry_sdk"}"#;
    const DROPPED: &str = r#"{"timestamp":"2026-11-21T22:13:07.800956Z","level":"WARN","fields":{"name":"BatchSpanProcessor.SpansDropped","dropped_span_count":280,"max_queue_size":16,"message":"Spans were dropped due to a queue being full."},"target":"opentelemetry_sdk"}"#;
    const EXPORT_ERROR: &str = r#"{"level":"ERROR","fields":{"name":"BatchSpanProcessor.ExportError","error":"timed out"},"target":"opentelemetry_sdk"}"#;
    const UNRELATED_WARN: &str =
        r#"{"level":"WARN","fields":{"message":"slow statement"},"target":"sqlx::query"}"#;

    #[test]
    fn a_queue_full_drop_is_refused_with_its_lower_bound() {
        let err = refuse_span_loss(&format!("{STARTED}\n{DROPPED}\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("at least 280"), "{err}");
        assert!(err.contains("BatchSpanProcessor.SpansDropped"), "{err}");
    }

    #[test]
    fn an_export_error_is_refused_without_a_count() {
        let err = refuse_span_loss(EXPORT_ERROR).unwrap_err().to_string();
        assert!(err.contains("an unknown number of"), "{err}");
        assert!(err.contains("BatchSpanProcessor.ExportError"), "{err}");
    }

    #[test]
    fn an_empty_or_clean_log_passes() {
        refuse_span_loss("").unwrap();
        refuse_span_loss(&format!("{UNRELATED_WARN}\nnot json\n")).unwrap();
    }
}
