//! host-proof's spans in a `dev` run's trace (#1965).
//!
//! `dev` names its run directory in `JAUNDER_TRACE_DIR` and passes the step
//! that launched host-proof as `TRACEPARENT`/`TRACESTATE`. When that directory
//! is set, host-proof appends a start record as each span opens and an end
//! record, with the span's attributes, as it closes, to its own
//! `<pid>-<random>.jsonl` file there; `dev` merges every file into the run's
//! trace. The record format is `tools/dev`'s, locked for both writers by
//! `tools/dev/tests/golden/span-records.jsonl`.
//!
//! Unset, host-proof is not part of a `dev` run, and empty, `dev` disabled
//! tracing: either way spans go nowhere. Tracing never changes host-proof's
//! output or status; a failed write warns once on stderr and stops tracing.

use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::Write,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use opentelemetry::{
    Context, ContextGuard, KeyValue, Value, global,
    propagation::TextMapPropagator,
    trace::{SpanId, TraceContextExt, Tracer},
};
use opentelemetry_sdk::{
    error::OTelSdkResult,
    propagation::TraceContextPropagator,
    trace::{SdkTracerProvider, Span, SpanData, SpanProcessor},
};
use serde::{Serialize, Serializer};

const TRACE_DIR: &str = "JAUNDER_TRACE_DIR";
const STATUS: &str = "jaunder.status";

/// Joins the caller's `dev` run, if there is one, for the rest of the process.
///
/// Spans started while the returned guard lives are children of the caller's
/// step.
pub fn join_run() -> Option<ContextGuard> {
    let directory = std::env::var_os(TRACE_DIR).filter(|value| !value.is_empty())?;
    let file = match tempfile::Builder::new()
        .prefix(&format!("{}-", std::process::id()))
        .suffix(".jsonl")
        .tempfile_in(&directory)
        .and_then(|file| file.keep().map_err(|error| error.error))
    {
        Ok((file, _path)) => file,
        Err(error) => {
            warn(&error);
            return None;
        }
    };
    global::set_tracer_provider(
        SdkTracerProvider::builder()
            .with_span_processor(RecordProcessor::new(file))
            .build(),
    );
    Some(caller_context(|name| std::env::var(name).ok()).attach())
}

/// The caller's span, carried in the W3C environment variables
/// `TRACEPARENT` and `TRACESTATE`.
fn caller_context(variable: impl Fn(&str) -> Option<String>) -> Context {
    let carrier: HashMap<String, String> = ["traceparent", "tracestate"]
        .into_iter()
        .filter_map(|key| Some((key.to_owned(), variable(&key.to_uppercase())?)))
        .collect();
    TraceContextPropagator::new().extract(&carrier)
}

fn warn(error: &dyn std::fmt::Display) {
    eprintln!("host-proof: tracing disabled: {error}");
}

/// One synchronous span, the current parent until it finishes.
pub(crate) struct Phase {
    context: Context,
    _attached: ContextGuard,
}

pub(crate) fn phase(name: impl Into<Cow<'static, str>>) -> Phase {
    let context = Context::current_with_span(global::tracer("host-proof").start(name));
    Phase {
        _attached: context.clone().attach(),
        context,
    }
}

impl Phase {
    pub fn finish(self, ok: bool) {
        let span = self.context.span();
        span.set_attribute(status(ok));
        span.end();
    }
}

fn status(ok: bool) -> KeyValue {
    KeyValue::new(STATUS, if ok { "ok" } else { "failed" })
}

/// Writes each span's start and end records as they happen, so a killed
/// process still leaves the spans it had opened.
#[derive(Debug)]
struct RecordProcessor {
    file: Mutex<Option<File>>,
}

impl RecordProcessor {
    fn new(file: File) -> Self {
        Self {
            file: Mutex::new(Some(file)),
        }
    }

    /// Appends `record`; any failure, including a record that could not be
    /// made, disables the writer after one warning.
    fn write(&self, record: std::io::Result<Record<'_>>) {
        // A writer that panicked mid-line may have left a partial record, so a
        // poisoned lock stops tracing like any other failure.
        let (mut file, poisoned) = match self.file.lock() {
            Ok(file) => (file, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };
        let Some(open) = file.as_mut() else {
            return;
        };
        let written = if poisoned {
            Err(std::io::Error::other("a span record write panicked"))
        } else {
            // One write per line, so a SIGKILL cuts at most the line being written.
            record
                .and_then(|record| serde_json::to_vec(&record).map_err(std::io::Error::from))
                .and_then(|mut line| {
                    line.push(b'\n');
                    open.write_all(&line)
                })
        };
        if let Err(error) = written {
            *file = None;
            warn(&error);
        }
    }
}

impl SpanProcessor for RecordProcessor {
    fn on_start(&self, span: &mut Span, _context: &Context) {
        if let Some(data) = span.exported_data() {
            self.write(Record::new("start", &data, data.start_time, None));
        }
    }

    fn on_end(&self, span: SpanData) {
        self.write(Record::new(
            "end",
            &span,
            span.end_time,
            Some(Attributes(&span.attributes)),
        ));
    }

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        Ok(())
    }
}

/// One line of a run directory's span records; field order follows `tools/dev`.
#[derive(Serialize)]
struct Record<'a> {
    kind: &'static str,
    trace_id: String,
    span_id: String,
    parent_id: Option<String>,
    name: &'a str,
    time_ns: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    attributes: Option<Attributes<'a>>,
}

impl<'a> Record<'a> {
    fn new(
        kind: &'static str,
        span: &'a SpanData,
        time: SystemTime,
        attributes: Option<Attributes<'a>>,
    ) -> std::io::Result<Self> {
        // The record format has no value for a time it cannot hold; a guessed
        // one would misplace the span, so the writer stops instead.
        let time_ns = time
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .and_then(|since| u64::try_from(since.as_nanos()).ok())
            .ok_or_else(|| {
                std::io::Error::other(format!(
                    "span {:?} time {time:?} is outside the record format's range",
                    span.name
                ))
            })?;
        Ok(Self {
            kind,
            trace_id: span.span_context.trace_id().to_string(),
            span_id: span.span_context.span_id().to_string(),
            parent_id: (span.parent_span_id != SpanId::INVALID)
                .then(|| span.parent_span_id.to_string()),
            name: &span.name,
            time_ns,
            attributes,
        })
    }
}

/// Attributes as one JSON object, in the order the span recorded them.
struct Attributes<'a>(&'a [KeyValue]);

impl Serialize for Attributes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|attribute| {
            let value = match &attribute.value {
                Value::Bool(value) => serde_json::Value::from(*value),
                Value::I64(value) => serde_json::Value::from(*value),
                Value::F64(value) => serde_json::Value::from(*value),
                other => serde_json::Value::from(other.to_string()),
            };
            (attribute.key.as_str(), value)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXIT_CODE: &str = "process.exit.code";
    use opentelemetry::trace::{Span as _, TraceId, TracerProvider as _};
    use opentelemetry_sdk::trace::IdGenerator;

    /// The ids the golden records use.
    #[derive(Debug)]
    struct GoldenIds(Mutex<Vec<u64>>);

    impl IdGenerator for GoldenIds {
        fn new_trace_id(&self) -> TraceId {
            TraceId::from(0x0af7_6519_16cd_43dd_8448_eb21_1c80_319c)
        }

        fn new_span_id(&self) -> SpanId {
            SpanId::from(self.0.lock().unwrap().remove(0))
        }
    }

    fn at(nanos: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos)
    }

    #[test]
    fn writer_emits_the_golden_records() {
        let golden = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../dev/tests/golden/span-records.jsonl"),
        )
        .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        let provider = SdkTracerProvider::builder()
            .with_id_generator(GoldenIds(Mutex::new(vec![
                0x00f0_67aa_0ba9_02b7,
                0x1111_1111_1111_1111,
            ])))
            .with_span_processor(RecordProcessor::new(file.reopen().unwrap()))
            .build();
        let tracer = provider.tracer("test");

        let root = Context::new().with_span(
            tracer
                .span_builder("dev static tsc")
                .with_start_time(at(1000))
                .start(&tracer),
        );
        let mut child = tracer
            .span_builder("tsc")
            .with_start_time(at(2000))
            .start_with_context(&tracer, &root);
        child.set_attributes([
            KeyValue::new(
                "process.command_line",
                "tsc --noEmit -p end2end/tsconfig.json",
            ),
            KeyValue::new(EXIT_CODE, 2),
            status(false),
        ]);
        child.end_with_timestamp(at(8000));
        root.span()
            .set_attributes([KeyValue::new(EXIT_CODE, 2), status(false)]);
        root.span().end_with_timestamp(at(9000));

        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), golden);
    }

    #[test]
    fn caller_context_continues_the_callers_span() {
        let context = caller_context(|name| {
            (name == "TRACEPARENT")
                .then(|| "00-0af7651916cd43dd8448eb211c80319c-00f067aa0ba902b7-01".to_owned())
        });
        let span = context.span();
        assert!(span.span_context().is_remote());
        assert_eq!(
            span.span_context().span_id().to_string(),
            "00f067aa0ba902b7"
        );
        assert!(!caller_context(|_| None).has_active_span());
    }

    #[test]
    fn a_failed_write_disables_the_writer() {
        let file = tempfile::NamedTempFile::new().unwrap();
        // A read-only handle makes every write fail.
        let processor = RecordProcessor::new(File::open(file.path()).unwrap());
        processor.write(Ok(Record {
            kind: "start",
            trace_id: String::new(),
            span_id: String::new(),
            parent_id: None,
            name: "phase",
            time_ns: 0,
            attributes: None,
        }));
        assert!(processor.file.lock().unwrap().is_none());
    }

    #[test]
    fn a_writer_poisoned_by_a_panic_is_disabled() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let processor = RecordProcessor::new(file.reopen().unwrap());
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let _held = processor.file.lock().unwrap();
                    panic!("poison the writer");
                })
                .join()
                .unwrap_err();
        });
        processor.write(Ok(Record {
            kind: "start",
            trace_id: String::new(),
            span_id: String::new(),
            parent_id: None,
            name: "phase",
            time_ns: 0,
            attributes: None,
        }));
        let writer = processor
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(writer.is_none());
    }

    #[test]
    fn a_time_before_the_epoch_disables_the_writer() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let tracer = SdkTracerProvider::builder()
            .with_span_processor(RecordProcessor::new(file.reopen().unwrap()))
            .build()
            .tracer("test");
        tracer
            .span_builder("early")
            .with_start_time(SystemTime::UNIX_EPOCH - Duration::from_secs(1))
            .start(&tracer)
            .end();
        tracer.start("later").end();
        assert_eq!(std::fs::read_to_string(file.path()).unwrap(), "");
    }
}
