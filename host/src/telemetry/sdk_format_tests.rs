use super::*;
use std::sync::{Arc, Mutex};
use tracing_subscriber::{fmt, prelude::*};
#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Memory {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> fmt::MakeWriter<'a> for Memory {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}
impl Memory {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}
fn selected(code: &str, count: i64) {
    tracing::debug!(name:"TonicLogsClient.ExportFailed",target:"opentelemetry-otlp",grpc_code=code,grpc_message="WITHHELD",grpc_details="WITHHELD",arbitrary="WITHHELD");
    tracing::debug!(name:"TonicTracesClient.ExportFailed",target:"opentelemetry-otlp",grpc_code=code,grpc_message="WITHHELD",grpc_details="WITHHELD");
    tracing::debug!(name:"TonicMetricsClient.ExportFailed",target:"opentelemetry-otlp",grpc_code=code,grpc_message="WITHHELD",grpc_details="WITHHELD");
    tracing::warn!(name:"TonicLogsClient.PartialSuccess",target:"opentelemetry-otlp",rejected_log_records=count,error_message="WITHHELD");
    tracing::warn!(name:"TonicTracesClient.PartialSuccess",target:"opentelemetry-otlp",rejected_spans=count,error_message="WITHHELD");
    tracing::warn!(name:"TonicMetricsClient.PartialSuccess",target:"opentelemetry-otlp",rejected_data_points=count,error_message="WITHHELD");
}
const CODES: [&str; 17] = [
    "Ok",
    "Cancelled",
    "Unknown",
    "InvalidArgument",
    "DeadlineExceeded",
    "NotFound",
    "AlreadyExists",
    "PermissionDenied",
    "ResourceExhausted",
    "FailedPrecondition",
    "Aborted",
    "OutOfRange",
    "Unimplemented",
    "Internal",
    "Unavailable",
    "DataLoss",
    "Unauthenticated",
];
#[test]
fn exact_six_metadata_shapes_keep_severity_identity_closed_codes_and_signed_scalars() {
    for json in [true, false] {
        let output = Memory::default();
        let subscriber = tracing_subscriber::registry().with(
            fmt::layer()
                .event_format(SafeSdkFormat::new(fmt::format().without_time(), json))
                .with_writer(output.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("private", secret = "WITHHELD");
            let _span = span.enter();
            for (n, code) in CODES.into_iter().enumerate() {
                selected(code, i64::try_from(n).unwrap() - 8);
            }
            selected("WITHHELD", i64::MIN);
            selected("WITHHELD", i64::MAX);
        });
        let text = output.text();
        assert!(!text.contains("WITHHELD"));
        assert!(!text.contains("private"));
        assert_eq!(text.lines().count(), 19 * 6);
        let expected_codes = CODES;
        for (group_index, group) in text.lines().collect::<Vec<_>>().chunks(6).enumerate() {
            for (n, line) in group.iter().enumerate() {
                assert!(line.contains(if n < 3 { "DEBUG" } else { "WARN" }));
                assert!(line.contains("opentelemetry-otlp"));
                assert!(line.contains(if n < 3 {
                    "ExportFailed"
                } else {
                    "PartialSuccess"
                }));
                if json {
                    let record: serde_json::Value = serde_json::from_str(line).unwrap();
                    assert!(record.get("span").is_none());
                    assert!(record["fields"]["name"].is_string());
                    if n < 3 {
                        if group_index < 17 {
                            assert_eq!(record["fields"]["grpc_code"], expected_codes[group_index]);
                        } else {
                            assert!(record["fields"].get("grpc_code").is_none());
                        }
                        assert_eq!(record["fields"]["grpc_message"], "remote detail withheld");
                        assert_eq!(record["fields"]["grpc_details"], "remote detail withheld");
                    } else {
                        let field = [
                            "rejected_log_records",
                            "rejected_spans",
                            "rejected_data_points",
                        ][n - 3];
                        let expected = match group_index {
                            17 => i64::MIN,
                            18 => i64::MAX,
                            _ => i64::try_from(group_index).unwrap() - 8,
                        };
                        assert_eq!(record["fields"][field], expected);
                        assert_eq!(record["fields"]["error_message"], "remote detail withheld");
                    }
                } else if n < 3 {
                    if group_index < 17 {
                        assert!(
                            line.contains(&format!("grpc_code={}", expected_codes[group_index]))
                        );
                    } else {
                        assert!(!line.contains("grpc_code="));
                    }
                }
            }
        }
        assert!(text.contains(&i64::MIN.to_string()));
        assert!(text.contains(&i64::MAX.to_string()));
    }
}
struct FailingWriter {
    fail_on: &'static str,
    written: String,
    failed: bool,
}
impl std::fmt::Write for FailingWriter {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        if text.contains(self.fail_on) {
            self.failed = true;
            return Err(std::fmt::Error);
        }
        self.written.push_str(text);
        Ok(())
    }
}

struct ForwardToFailingWriter<F> {
    safe: SafeSdkFormat<F>,
    fail_on: &'static str,
    outcomes: Arc<Mutex<Vec<(std::fmt::Result, FailingWriter)>>>,
}
impl<S, N, F> FormatEvent<S, N> for ForwardToFailingWriter<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        _: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let mut output = FailingWriter {
            fail_on: self.fail_on,
            written: String::new(),
            failed: false,
        };
        let result = self.safe.format_event(ctx, Writer::new(&mut output), event);
        self.outcomes.lock().unwrap().push((result, output));
        Ok(())
    }
}

#[test]
fn selected_event_writer_errors_reach_the_format_event_caller() {
    for (json, fail_on, expected_prefix) in [
        (true, "{\"level\":\"", ""),
        (
            true,
            ",\"grpc_message\":",
            "{\"level\":\"DEBUG\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicLogsClient.ExportFailed\"",
        ),
        (false, " opentelemetry-otlp: name=", "DEBUG"),
    ] {
        let outcomes = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(fmt::layer().event_format(
            ForwardToFailingWriter {
                safe: SafeSdkFormat::new(fmt::format().without_time(), json),
                fail_on,
                outcomes: outcomes.clone(),
            },
        ));
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(name:"TonicLogsClient.ExportFailed",target:"opentelemetry-otlp",grpc_message="WITHHELD",grpc_details="WITHHELD");
        });
        let outcomes = outcomes.lock().unwrap();
        assert_eq!(outcomes.len(), 1);
        let (result, output) = &outcomes[0];
        assert_eq!(*result, Err(std::fmt::Error));
        assert!(
            output.failed,
            "the requested write must be reached: {fail_on}"
        );
        assert_eq!(output.written, expected_prefix);
        assert!(!output.written.contains("WITHHELD"));
    }
}

#[test]
fn ordinary_text_bytes_are_identical_and_invalid_selected_carriers_are_not_rendered() {
    let stock = Memory::default();
    let safe = Memory::default();
    let subscriber = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .without_time()
                .with_ansi(false)
                .with_writer(stock.clone()),
        )
        .with(
            fmt::layer()
                .with_ansi(false)
                .event_format(SafeSdkFormat::new(fmt::format().without_time(), false))
                .with_writer(safe.clone()),
        );
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("ordinary_context", field = "ordinary raw");
        let _span = span.enter();
        tracing::info!(name:"TonicLogsClient.ExportFailed",target:"app",grpc_message="ordinary remote-like application field");
        tracing::warn!(name:"Unrelated",target:"opentelemetry-otlp",arbitrary="ordinary SDK field");
    });
    assert_eq!(stock.text(), safe.text());
    for json in [true, false] {
        let invalid = Memory::default();
        let subscriber = tracing_subscriber::registry().with(
            fmt::layer()
                .event_format(SafeSdkFormat::new(fmt::format().without_time(), json))
                .with_writer(invalid.clone()),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(name:"TonicLogsClient.ExportFailed",target:"opentelemetry-otlp",grpc_code=?Some("WITHHELD"),grpc_message="WITHHELD");
            tracing::warn!(name:"TonicLogsClient.PartialSuccess",target:"opentelemetry-otlp",rejected_log_records="WITHHELD",error_message="WITHHELD",arbitrary="WITHHELD");
        });
        let text = invalid.text();
        assert!(!text.contains("WITHHELD"));
        assert!(!text.contains("arbitrary"));
        if json {
            let records: Vec<serde_json::Value> = text
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert!(records[0]["fields"].get("grpc_code").is_none());
            assert!(records[1]["fields"].get("rejected_log_records").is_none());
        } else {
            assert_eq!(
                text.lines().collect::<Vec<_>>(),
                [
                    "DEBUG opentelemetry-otlp: name=TonicLogsClient.ExportFailed grpc_message=\"remote detail withheld\" grpc_details=\"remote detail withheld\"",
                    "WARN opentelemetry-otlp: name=TonicLogsClient.PartialSuccess error_message=\"remote detail withheld\"",
                ]
            );
            assert!(!text.contains("grpc_code="));
            assert!(!text.contains("rejected_"));
        }
    }
}

#[test]
fn ordinary_complete_json_bytes_and_warn_visible_context_equal_stock() {
    let stock = Memory::default();
    let safe = Memory::default();
    let subscriber = tracing_subscriber::registry()
        .with(
            fmt::layer()
                .json()
                .without_time()
                .with_writer(stock.clone())
                .with_filter(tracing::level_filters::LevelFilter::WARN),
        )
        .with(
            fmt::layer()
                .json()
                .event_format(SafeSdkFormat::new(
                    fmt::format().json().without_time(),
                    true,
                ))
                .with_writer(safe.clone())
                .with_filter(tracing::level_filters::LevelFilter::WARN),
        );
    tracing::subscriber::with_default(subscriber, || {
        let info = tracing::info_span!("request", uri = "/synthetic");
        let _info = info.enter();
        tracing::warn!(target:"app",field="value","ordinary");
        let warning = tracing::warn_span!("warning_context", retained = "warn-span");
        let _warning = warning.enter();
        tracing::error!(name:"TonicLogsClient.PartialSuccess",target:"app",error_message="ordinary raw field");
        tracing::warn!(name:"OtherSdkEvent",target:"opentelemetry-otlp",ordinary="ordinary SDK field");
    });
    assert_eq!(stock.text(), safe.text());
    let text = safe.text();
    assert!(!text.contains("/synthetic"));
    assert!(text.contains("warn-span"));
    assert!(text.contains("ordinary raw field"));
    assert!(text.contains("ordinary SDK field"));
}
