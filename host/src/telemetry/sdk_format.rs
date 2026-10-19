//! Local formatting for the six remote-detail diagnostics in OTLP 0.32.0.
//! This adapter is not a universal SDK sanitizer. Other events use the caller's
//! formatter unchanged; selected diagnostics never include span field copies.
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    fmt::{FmtContext, FormatEvent, FormatFields, format::Writer},
    registry::LookupSpan,
};

/// Local-sink formatter for the six pinned OTLP remote-detail diagnostics.
/// All other events delegate unchanged to `inner`. Selected records omit span
/// copies and remote text; this is not a universal SDK sanitizer.
pub struct SafeSdkFormat<F> {
    inner: F,
    json: bool,
}

impl<F> SafeSdkFormat<F> {
    /// Wrap an existing formatter, selecting the sink's JSON or text encoding.
    pub fn new(inner: F, json: bool) -> Self {
        Self { inner, json }
    }
}

impl<S, N, F> FormatEvent<S, N> for SafeSdkFormat<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        if meta.target() != "opentelemetry-otlp" {
            return self.inner.format_event(ctx, writer, event);
        }
        let count_field = match meta.name() {
            "TonicLogsClient.ExportFailed"
            | "TonicTracesClient.ExportFailed"
            | "TonicMetricsClient.ExportFailed" => None,
            "TonicLogsClient.PartialSuccess" => Some("rejected_log_records"),
            "TonicTracesClient.PartialSuccess" => Some("rejected_spans"),
            "TonicMetricsClient.PartialSuccess" => Some("rejected_data_points"),
            _ => return self.inner.format_event(ctx, writer, event),
        };
        let mut fields = SafeFields {
            count_field,
            ..SafeFields::default()
        };
        event.record(&mut fields);
        let selected = fields.selected();
        if self.json {
            write!(
                writer,
                "{{\"level\":\"{}\",\"target\":\"opentelemetry-otlp\",\"fields\":{{\"name\":\"{}\"",
                meta.level(),
                meta.name()
            )?;
        } else {
            write!(
                writer,
                "{} opentelemetry-otlp: name={}",
                meta.level(),
                meta.name()
            )?;
        }
        for (name, value) in selected.into_iter().flatten() {
            let encoded = match (self.json, value) {
                (true, SafeValue::Code(value) | SafeValue::Withheld(value)) => {
                    format!(",\"{name}\":\"{value}\"")
                }
                (true, SafeValue::Count(value)) => format!(",\"{name}\":{value}"),
                (false, SafeValue::Code(value)) => format!(" {name}={value}"),
                (false, SafeValue::Withheld(value)) => format!(" {name}=\"{value}\""),
                (false, SafeValue::Count(value)) => format!(" {name}={value}"),
            };
            writer.write_str(&encoded)?;
        }
        if self.json {
            write!(writer, "}}}}")?;
        }
        writeln!(writer)
    }
}

#[derive(Default)]
struct SafeFields {
    code: Option<&'static str>,
    count_field: Option<&'static str>,
    count: Option<i64>,
}

enum SafeValue {
    Code(&'static str),
    Count(i64),
    Withheld(&'static str),
}

impl SafeFields {
    fn selected(&self) -> [Option<(&'static str, SafeValue)>; 3] {
        let withheld = || SafeValue::Withheld("remote detail withheld");
        if let Some(field) = self.count_field {
            [
                self.count.map(|count| (field, SafeValue::Count(count))),
                Some(("error_message", withheld())),
                None,
            ]
        } else {
            [
                self.code.map(|code| ("grpc_code", SafeValue::Code(code))),
                Some(("grpc_message", withheld())),
                Some(("grpc_details", withheld())),
            ]
        }
    }
}

impl Visit for SafeFields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        if Some(field.name()) == self.count_field {
            self.count = Some(value);
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "grpc_code" {
            // OTLP 0.32.0 records the Debug name of tonic::Code as a String.
            // Match that closed value set, never print an arbitrary incoming string.
            self.code = [
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
            ]
            .into_iter()
            .find(|code| *code == value);
        }
    }
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
}

#[cfg(test)]
mod tests {
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
        tracing::debug!(
            name: "TonicLogsClient.ExportFailed",
            target: "opentelemetry-otlp",
            grpc_code = code,
            grpc_message = "WITHHELD",
            grpc_details = "WITHHELD",
            arbitrary = "WITHHELD",
        );
        tracing::debug!(
            name: "TonicTracesClient.ExportFailed",
            target: "opentelemetry-otlp",
            grpc_code = code,
            grpc_message = "WITHHELD",
            grpc_details = "WITHHELD",
        );
        tracing::debug!(
            name: "TonicMetricsClient.ExportFailed",
            target: "opentelemetry-otlp",
            grpc_code = code,
            grpc_message = "WITHHELD",
            grpc_details = "WITHHELD",
        );
        tracing::warn!(
            name: "TonicLogsClient.PartialSuccess",
            target: "opentelemetry-otlp",
            rejected_log_records = count,
            error_message = "WITHHELD",
        );
        tracing::warn!(
            name: "TonicTracesClient.PartialSuccess",
            target: "opentelemetry-otlp",
            rejected_spans = count,
            error_message = "WITHHELD",
        );
        tracing::warn!(
            name: "TonicMetricsClient.PartialSuccess",
            target: "opentelemetry-otlp",
            rejected_data_points = count,
            error_message = "WITHHELD",
        );
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
                                assert_eq!(
                                    record["fields"]["grpc_code"],
                                    expected_codes[group_index]
                                );
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
                                line.contains(&format!(
                                    "grpc_code={}",
                                    expected_codes[group_index]
                                ))
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
    #[test]
    fn selected_text_and_json_match_fixed_bytes() {
        let expected_text = concat!(
            "DEBUG opentelemetry-otlp: name=TonicLogsClient.ExportFailed grpc_code=Unavailable grpc_message=\"remote detail withheld\" grpc_details=\"remote detail withheld\"\n",
            "DEBUG opentelemetry-otlp: name=TonicTracesClient.ExportFailed grpc_code=Unavailable grpc_message=\"remote detail withheld\" grpc_details=\"remote detail withheld\"\n",
            "DEBUG opentelemetry-otlp: name=TonicMetricsClient.ExportFailed grpc_code=Unavailable grpc_message=\"remote detail withheld\" grpc_details=\"remote detail withheld\"\n",
            "WARN opentelemetry-otlp: name=TonicLogsClient.PartialSuccess rejected_log_records=-17 error_message=\"remote detail withheld\"\n",
            "WARN opentelemetry-otlp: name=TonicTracesClient.PartialSuccess rejected_spans=-17 error_message=\"remote detail withheld\"\n",
            "WARN opentelemetry-otlp: name=TonicMetricsClient.PartialSuccess rejected_data_points=-17 error_message=\"remote detail withheld\"\n",
        );
        let expected_json = concat!(
            "{\"level\":\"DEBUG\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicLogsClient.ExportFailed\",\"grpc_code\":\"Unavailable\",\"grpc_message\":\"remote detail withheld\",\"grpc_details\":\"remote detail withheld\"}}\n",
            "{\"level\":\"DEBUG\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicTracesClient.ExportFailed\",\"grpc_code\":\"Unavailable\",\"grpc_message\":\"remote detail withheld\",\"grpc_details\":\"remote detail withheld\"}}\n",
            "{\"level\":\"DEBUG\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicMetricsClient.ExportFailed\",\"grpc_code\":\"Unavailable\",\"grpc_message\":\"remote detail withheld\",\"grpc_details\":\"remote detail withheld\"}}\n",
            "{\"level\":\"WARN\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicLogsClient.PartialSuccess\",\"rejected_log_records\":-17,\"error_message\":\"remote detail withheld\"}}\n",
            "{\"level\":\"WARN\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicTracesClient.PartialSuccess\",\"rejected_spans\":-17,\"error_message\":\"remote detail withheld\"}}\n",
            "{\"level\":\"WARN\",\"target\":\"opentelemetry-otlp\",\"fields\":{\"name\":\"TonicMetricsClient.PartialSuccess\",\"rejected_data_points\":-17,\"error_message\":\"remote detail withheld\"}}\n",
        );
        for (json, expected) in [(false, expected_text), (true, expected_json)] {
            let output = Memory::default();
            let subscriber = tracing_subscriber::registry().with(
                fmt::layer()
                    .event_format(SafeSdkFormat::new(fmt::format().without_time(), json))
                    .with_writer(output.clone()),
            );
            tracing::subscriber::with_default(subscriber, || selected("Unavailable", -17));
            assert_eq!(output.text(), expected);
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
                tracing::debug!(
                    name: "TonicLogsClient.ExportFailed",
                    target: "opentelemetry-otlp",
                    grpc_message = "WITHHELD",
                    grpc_details = "WITHHELD",
                );
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
            tracing::info!(
                name: "TonicLogsClient.ExportFailed",
                target: "app",
                grpc_message = "ordinary remote-like application field",
            );
            tracing::warn!(
                name: "Unrelated",
                target: "opentelemetry-otlp",
                arbitrary = "ordinary SDK field",
            );
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
                tracing::debug!(
                    name: "TonicLogsClient.ExportFailed",
                    target: "opentelemetry-otlp",
                    grpc_code = ?Some("WITHHELD"),
                    grpc_message = "WITHHELD",
                );
                tracing::warn!(
                    name: "TonicLogsClient.PartialSuccess",
                    target: "opentelemetry-otlp",
                    rejected_log_records = "WITHHELD",
                    error_message = "WITHHELD",
                    arbitrary = "WITHHELD",
                );
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
            tracing::warn!(
                target: "app",
                field = "value",
                "ordinary",
            );
            let warning = tracing::warn_span!("warning_context", retained = "warn-span");
            let _warning = warning.enter();
            tracing::error!(
                name: "TonicLogsClient.PartialSuccess",
                target: "app",
                error_message = "ordinary raw field",
            );
            tracing::warn!(
                name: "OtherSdkEvent",
                target: "opentelemetry-otlp",
                ordinary = "ordinary SDK field",
            );
        });
        assert_eq!(stock.text(), safe.text());
        let text = safe.text();
        assert!(!text.contains("/synthetic"));
        assert!(text.contains("warn-span"));
        assert!(text.contains("ordinary raw field"));
        assert!(text.contains("ordinary SDK field"));
    }
}
