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
        if self.json {
            write!(
                writer,
                "{{\"level\":\"{}\",\"target\":\"opentelemetry-otlp\",\"fields\":{{\"name\":\"{}\"",
                meta.level(),
                meta.name()
            )?;
            if let Some(field) = count_field {
                if let Some(count) = fields.count {
                    write!(writer, ",\"{field}\":{count}")?;
                }
                write!(writer, ",\"error_message\":\"remote detail withheld\"")?;
            } else {
                if let Some(code) = fields.code {
                    write!(writer, ",\"grpc_code\":\"{code}\"")?;
                }
                write!(
                    writer,
                    ",\"grpc_message\":\"remote detail withheld\",\"grpc_details\":\"remote detail withheld\""
                )?;
            }
            writeln!(writer, "}}}}")
        } else {
            write!(
                writer,
                "{} opentelemetry-otlp: name={}",
                meta.level(),
                meta.name()
            )?;
            if let Some(field) = count_field {
                if let Some(count) = fields.count {
                    write!(writer, " {field}={count}")?;
                }
                writeln!(writer, " error_message=\"remote detail withheld\"")
            } else {
                if let Some(code) = fields.code {
                    write!(writer, " grpc_code={code}")?;
                }
                writeln!(
                    writer,
                    " grpc_message=\"remote detail withheld\" grpc_details=\"remote detail withheld\""
                )
            }
        }
    }
}

#[cfg(test)]
#[path = "sdk_format_tests.rs"]
mod tests;

#[derive(Default)]
struct SafeFields {
    code: Option<&'static str>,
    count_field: Option<&'static str>,
    count: Option<i64>,
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
