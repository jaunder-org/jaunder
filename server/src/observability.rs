use axum::Router;
use axum::http::HeaderName;
use host::capture;
use opentelemetry::propagation::Extractor;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::registry::LookupSpan;

#[derive(Clone, Copy)]
enum FallbackKind {
    DiagLogOpen,
    PanicDiagWrite,
}

impl FallbackKind {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::DiagLogOpen => (
                "server.observability.diag_log_open",
                "diagnostic log disabled",
            ),
            Self::PanicDiagWrite => (
                "server.observability.panic_diag_write",
                "diagnostic write failed",
            ),
        }
    }
}

fn write_fallback(mut writer: impl std::io::Write, kind: FallbackKind) -> std::io::Result<()> {
    let (context, message) = kind.parts();
    writeln!(writer, "{context}: {message}")
}

#[cfg(test)]
struct TestFallbackCapture {
    owner: std::thread::ThreadId,
    output: Vec<u8>,
}

#[cfg(test)]
static TEST_FALLBACK_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
static TEST_FALLBACK_OUTPUT: std::sync::Mutex<Option<TestFallbackCapture>> =
    std::sync::Mutex::new(None);

#[cfg(test)]
fn capture_fallbacks<R>(operation: impl FnOnce() -> R) -> (R, String) {
    let _serial = TEST_FALLBACK_SERIAL
        .lock()
        .expect("fallback capture serial");
    {
        let mut capture = TEST_FALLBACK_OUTPUT.lock().expect("fallback capture lock");
        assert!(capture.is_none(), "nested fallback capture");
        *capture = Some(TestFallbackCapture {
            owner: std::thread::current().id(),
            output: Vec::new(),
        });
    }
    let result = operation();
    let output = TEST_FALLBACK_OUTPUT
        .lock()
        .expect("fallback capture lock")
        .take()
        .expect("capture")
        .output;
    (result, String::from_utf8(output).expect("fallback utf8"))
}

fn fallback(kind: FallbackKind) {
    #[cfg(test)]
    let captured = {
        let owner = std::thread::current().id();
        let mut output = TEST_FALLBACK_OUTPUT.lock().expect("fallback capture lock");
        output
            .as_mut()
            .filter(|capture| capture.owner == owner)
            .is_some_and(|capture| write_fallback(&mut capture.output, kind).is_ok())
    };
    #[cfg(test)]
    if captured {
        return;
    }
    let _ = write_fallback(std::io::stderr().lock(), kind);
}

/// The scoped diagnostic-log path (`<JAUNDER_CAPTURE_DIR>/diag.log`), if capture is on.
/// When set (e2e only), the server appends a small JSONL file of WARN+ events plus panic
/// records to it — a purpose-built, low-noise artifact the e2e zero-panic gate
/// consumes, demoting the kernel-laden journal to a fallback (issue #144). Unset in
/// production, so the whole feature is inert there (see the `host` crate).
fn diag_log_file() -> Option<std::path::PathBuf> {
    capture::file(capture::Stream::Diag)
}

fn open_diag_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}
fn open_diag_file_with<T>(
    path: &std::path::Path,
    open: impl FnOnce(&std::path::Path) -> std::io::Result<T>,
    mut warn: impl FnMut(),
) -> Option<T> {
    if let Ok(value) = open(path) {
        Some(value)
    } else {
        warn();
        None
    }
}

/// Build the scoped diagnostic layer: a JSON `fmt` layer writing to `make_writer`,
/// gated to WARN and above by its **own per-layer filter**.
///
/// The `.with_filter(LevelFilter::WARN)` is load-bearing and must stay a *per-layer*
/// filter (`Filtered`), never a second global `.with(LevelFilter::WARN)` on the
/// registry: a global level would clamp the whole subscriber to WARN+, silencing INFO
/// to the fmt/OTel sinks. As a per-layer filter it narrows only this sink, so the diag
/// file captures `WARN+ ∩ global-filter` while the other layers keep their own levels
/// (issue #144).
fn diag_layer<S, W>(make_writer: W) -> impl Layer<S>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> fmt::MakeWriter<'writer> + 'static,
{
    fmt::layer()
        .json()
        .with_writer(make_writer)
        .with_filter(tracing::level_filters::LevelFilter::WARN)
}

/// A single scoped-diagnostic panic record (issue #144). Serialized to one JSONL
/// line by [`DiagPanicRecord::to_line`] and appended to the scoped diag log by
/// the panic hook. `kind: "panic"` discriminates these from the WARN+ tracing events
/// in the same file; `message` carries the literal `panicked at <location>` substring
/// the e2e zero-panic gate greps for, and `location` is `Location::to_string()`
/// verbatim so it is byte-identical to what the default hook prints to the journal
/// (the gate de-dups the two sources by location).
#[derive(serde::Serialize)]
struct DiagPanicRecord<'a> {
    timestamp: &'a str,
    level: &'a str,
    kind: &'a str,
    target: &'a str,
    message: String,
    location: String,
    thread: String,
}

/// Best-effort human-readable panic payload. Panics carry either `&str` (from
/// `panic!("literal")`) or `String` (from `panic!("{}", x)`); anything else is rare
/// and rendered as a placeholder rather than lost.
fn panic_payload_str(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

impl<'a> DiagPanicRecord<'a> {
    /// Build a record from a panic. `timestamp` (RFC3339 UTC) is injected so the
    /// formatting is deterministic under test; the installed hook supplies `now`.
    fn from_panic(info: &std::panic::PanicHookInfo<'_>, thread: &str, timestamp: &'a str) -> Self {
        let location = info.location().map(ToString::to_string).unwrap_or_default();
        let payload = panic_payload_str(info);
        DiagPanicRecord {
            timestamp,
            level: "ERROR",
            kind: "panic",
            target: "panic",
            message: format!("panicked at {location}: {payload}"),
            location,
            thread: thread.to_owned(),
        }
    }

    /// One physical JSONL line (serde escapes any newline in the payload, so a
    /// multi-line panic message stays a single line). Runs inside the panic hook, so
    /// it must never itself panic: serializing this fixed struct cannot fail, and the
    /// workspace denies `.unwrap()`/`.expect()` in non-test code — hence
    /// `unwrap_or_default()` (an unreachable `""` fallback).
    fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

/// Install a panic hook that appends a scoped [`DiagPanicRecord`] to `path`, when a
/// path is given (`None` — the production default with `JAUNDER_CAPTURE_DIR` unset —
/// leaves the existing hook untouched). Taking the `Option` here keeps the enablement
/// check with the installer, mirroring how the diag *layer* is an `Option`.
///
/// DEADLOCK-SAFETY (load-bearing — do not "simplify" this to share the diag layer's
/// writer or to call `tracing::error!`): the hook opens its **own** `File` in append
/// mode and writes directly. If it instead shared a `Mutex<File>` with the tracing
/// layer, a thread that panics *while holding that mutex* (or while the subscriber
/// holds an internal lock, were we to route through `tracing`) would deadlock when the
/// hook re-acquired the lock on the panicking thread — a captured panic would become a
/// silent hang, the worst outcome for a diagnostics feature. `O_APPEND` on a regular
/// file positions each `write()` at EOF atomically; the whole record goes out in one
/// `write_all`, so it interleaves with the layer's WARN+ lines at line boundaries
/// without any shared lock. We chain to the previous hook so the default stderr →
/// journald path still fires — the journal stays the fallback artifact and catches any
/// panic that fires before this hook is installed (issue #144).
type WritePanicDiagOperation = fn(&mut std::fs::File, &[u8]) -> std::io::Result<()>;

fn write_panic_diag(file: &mut std::fs::File, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    file.write_all(bytes)
}
fn install_diag_panic_hook_with(
    path: Option<std::path::PathBuf>,
    open: fn(&std::path::Path) -> std::io::Result<std::fs::File>,
    write: WritePanicDiagOperation,
    warn: fn(FallbackKind),
) {
    let Some(path) = path else {
        return;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        match open(&path) {
            Ok(mut file) => {
                let timestamp =
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
                let thread = std::thread::current()
                    .name()
                    .unwrap_or("unnamed")
                    .to_owned();
                let line = DiagPanicRecord::from_panic(info, &thread, &timestamp).to_line();
                if write(&mut file, line.as_bytes()).is_err() {
                    warn(FallbackKind::PanicDiagWrite);
                }
            }
            Err(_) => warn(FallbackKind::PanicDiagWrite),
        }
        previous(info);
    }));
}

fn install_diag_panic_hook(path: Option<std::path::PathBuf>) {
    install_diag_panic_hook_with(path, open_diag_file, write_panic_diag, fallback);
}

fn init_tracing_impl(verbose: bool) -> host::telemetry::TelemetryGuard {
    // Scoped diagnostic capture (issue #144): when JAUNDER_CAPTURE_DIR is set,
    // append WARN+ events as JSONL to it via a synchronous `Arc<File>` sink —
    // deliberately not a buffered/non-blocking writer, so a `panic = abort`
    // cannot drop the very lines the feature exists to keep. An open failure
    // disables the sink (non-fatal) rather than taking down startup. The path is
    // resolved once here and reused for the panic hook installed below.
    let diag_path = diag_log_file();
    let diag_log_layer = diag_path.as_ref().and_then(|path| {
        open_diag_file_with(path, open_diag_file, || fallback(FallbackKind::DiagLogOpen))
            .map(|file| diag_layer(std::sync::Arc::new(file)).boxed())
    });

    let guard = host::telemetry::init_tracing_with_layer(verbose, diag_log_layer);

    // Install the scoped-diag panic hook (a no-op when disabled). It is
    // independent of the subscriber above and deliberately does not route
    // through it — see `install_diag_panic_hook` for the deadlock-safety
    // reasoning.
    install_diag_panic_hook(diag_path);

    guard
}

#[must_use]
pub fn init_server_tracing(verbose: bool) -> host::telemetry::TelemetryGuard {
    init_tracing_impl(verbose)
}

/// Trace context extracted from inbound request headers (W3C `traceparent`),
/// stashed in request extensions so the request span can adopt it as parent.
#[derive(Clone)]
struct ExtractedTraceContext(opentelemetry::Context);

struct HeaderExtractor<'a>(&'a axum::http::HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(axum::http::HeaderName::as_str).collect()
    }
}

async fn extract_trace_context(
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    request
        .extensions_mut()
        .insert(ExtractedTraceContext(context));
    next.run(request).await
}

/// Builds the per-request tracing span, adopting any extracted upstream trace
/// context as its parent.
fn make_request_span(request: &axum::extract::Request) -> tracing::Span {
    let span = tracing::span!(
        Level::INFO,
        "request",
        method = %request.method(),
        uri = %request.uri(),
        version = ?request.version(),
        headers = ?request.headers(),
    );
    if let Some(parent) = request.extensions().get::<ExtractedTraceContext>() {
        span.set_parent(parent.0.clone());
    }
    span
}

/// Applies the HTTP observability middleware stack — trace-context extraction,
/// request-id set/propagate, and the per-request tracing span — to `router`.
/// Kept here so all OTel/tower tracing construction lives with the rest of the
/// tracing setup (§1.7).
pub fn with_http_observability<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let request_id_header = HeaderName::from_static("x-request-id");
    let layer = ServiceBuilder::new()
        .layer(axum::middleware::from_fn(extract_trace_context))
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(request_id_header))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(make_request_span)
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        );
    router.layer(layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderMap, Request, StatusCode};
    use common::test_support::with_env;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    /// An in-memory `MakeWriter` capturing every write into a shared buffer, so a
    /// layer's output can be asserted on. `Arc<Mutex<Vec<u8>>>` is not itself a
    /// `MakeWriter`, and `fmt::TestWriter` targets std{out,err} (uncapturable), so a
    /// small newtype is required.
    #[derive(Clone)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("shared buffer lock")
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> fmt::MakeWriter<'writer> for Shared {
        type Writer = Shared;
        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn shared_writer_captures_writes() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut writer = Shared(buf.clone());
        writer.write_all(b"captured").expect("write");
        writer.flush().expect("flush");
        assert_eq!(&*buf.lock().expect("lock"), b"captured");
    }

    fn assert_error_metric_count<R>(expected: usize, operation: impl FnOnce() -> R) -> R {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        opentelemetry::global::set_meter_provider(provider.clone());
        let result = operation();
        provider.force_flush().expect("flush error metrics");
        let metrics = exporter.get_finished_metrics().expect("metrics");
        let points = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .filter(|metric| metric.name() == "jaunder.errors")
            .count();
        assert_eq!(points, expected, "unexpected jaunder.errors metric count");
        result
    }

    fn assert_zero_error_metrics<R>(operation: impl FnOnce() -> R) -> R {
        assert_error_metric_count(0, operation)
    }

    fn assert_fixed_fallback(output: &str, kind: FallbackKind) {
        let (context, message) = kind.parts();
        assert_eq!(output, format!("{context}: {message}\n"));
    }

    fn fail_diag_open(_path: &std::path::Path) -> std::io::Result<std::fs::File> {
        Err(std::io::Error::other(
            "injected panic diagnostic open failure",
        ))
    }

    fn fail_diag_write(_file: &mut std::fs::File, _bytes: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "injected panic diagnostic write failure",
        ))
    }

    #[test]
    fn diagnostic_open_failure_continues_with_one_fixed_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let opened = assert_zero_error_metrics(|| {
            open_diag_file_with(
                std::path::Path::new("injected-diag-path"),
                |_| Err::<(), _>(std::io::Error::other("injected open failure")),
                || {
                    write_fallback(&mut output, FallbackKind::DiagLogOpen).expect("write fallback");
                },
            )
        });
        assert!(
            opened.is_none(),
            "startup continuation disables only diag log"
        );
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::DiagLogOpen,
        );
    }

    #[test]
    fn diag_layer_captures_warn_and_above_not_info_under_global_info_filter() {
        // The load-bearing AND-gate check: the diag layer's per-layer WARN filter must
        // narrow only its own sink, under the same global `info` filter e2e uses — INFO
        // stays out of the diag file but still reaches the other layers.
        with_env(|_env| {
            let diag_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let other_buf = Arc::new(Mutex::new(Vec::<u8>::new()));
            let subscriber = tracing_subscriber::registry()
                .with(EnvFilter::new("info"))
                .with(
                    fmt::layer()
                        .with_ansi(false)
                        .with_writer(Shared(other_buf.clone())),
                )
                .with(diag_layer(Shared(diag_buf.clone())));
            tracing::subscriber::with_default(subscriber, || {
                tracing::info!("info-line");
                tracing::warn!("warn-line");
                tracing::error!("error-line");
            });

            let diag =
                String::from_utf8(diag_buf.lock().expect("diag lock").clone()).expect("utf8");
            let other =
                String::from_utf8(other_buf.lock().expect("other lock").clone()).expect("utf8");

            assert!(!diag.contains("info-line"), "diag sink must drop INFO");
            assert!(diag.contains("warn-line"), "diag sink must keep WARN");
            assert!(diag.contains("error-line"), "diag sink must keep ERROR");
            for line in diag.lines() {
                serde_json::from_str::<serde_json::Value>(line).expect("diag line is valid JSONL");
            }
            // The other sink still sees INFO: we narrowed only the diag layer, not the registry.
            assert!(
                other.contains("info-line"),
                "global filter must not be clamped to WARN"
            );
        });
    }

    #[test]
    fn diag_log_file_is_none_when_env_unset() {
        with_env(|env| {
            env.remove(host::capture::DIR_ENV);
            assert!(diag_log_file().is_none());
        });
    }

    #[test]
    fn diag_panic_record_is_one_json_line_with_panicked_at() {
        // A newline-bearing payload must stay a single physical JSONL line (serde
        // escapes the embedded newline), and the record must carry the gate's
        // `panicked at` substring plus the verbatim location.
        let record = DiagPanicRecord {
            timestamp: "2026-07-04T12:00:00Z",
            level: "ERROR",
            kind: "panic",
            target: "panic",
            message: "panicked at server/src/foo.rs:42:5: boom\nsecond line".to_owned(),
            location: "server/src/foo.rs:42:5".to_owned(),
            thread: "main".to_owned(),
        };
        let line = record.to_line();
        assert_eq!(
            line.matches('\n').count(),
            1,
            "exactly one physical line — the payload newline is JSON-escaped"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(line.trim_end()).expect("valid JSON line");
        assert_eq!(parsed["kind"], "panic");
        assert_eq!(parsed["level"], "ERROR");
        assert_eq!(parsed["target"], "panic");
        assert_eq!(parsed["location"], "server/src/foo.rs:42:5");
        assert!(
            parsed["message"]
                .as_str()
                .expect("message string")
                .contains("panicked at")
        );
    }

    #[test]
    fn installed_diag_panic_hook_appends_record_and_restores() {
        with_env(|_env| {
            let dir = tempfile::TempDir::new().expect("tempdir");
            let path = dir.path().join("diag.log");
            // Save/restore the process-global hook so it can't fire on a later test
            // writing to this now-deleted TempDir.
            let previous = std::panic::take_hook();
            install_diag_panic_hook(Some(path.clone()));
            // Exercise every payload branch: `&str`, `String`, and a non-string payload.
            let dynamic = String::from("formatted-payload");
            let _ = std::panic::catch_unwind(|| panic!("boom-under-test"));
            let _ = std::panic::catch_unwind(|| panic!("{dynamic}"));
            let _ = std::panic::catch_unwind(|| std::panic::panic_any(42u32));
            std::panic::set_hook(previous);

            let content = std::fs::read_to_string(&path).expect("read diag");
            let records: Vec<serde_json::Value> = content
                .lines()
                .map(|line| serde_json::from_str(line).expect("valid JSON"))
                .collect();
            assert_eq!(records.len(), 3, "one record per panic");
            assert!(records.iter().all(|record| record["kind"] == "panic"));
            let messages: Vec<&str> = records
                .iter()
                .map(|record| record["message"].as_str().expect("message string"))
                .collect();
            assert!(
                messages
                    .iter()
                    .all(|message| message.contains("panicked at"))
            );
            assert!(messages[0].contains("boom-under-test"));
            assert!(messages[1].contains("formatted-payload"));
            assert!(messages[2].contains("<non-string panic payload>"));
        });
    }

    #[test]
    fn panic_diagnostic_writer_failure_chains_hook_once_with_fixed_fallback_and_zero_metrics() {
        with_env(|_env| {
            let original = std::panic::take_hook();
            let chained_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let chained_calls_in_hook = chained_calls.clone();
            std::panic::set_hook(Box::new(move |_| {
                chained_calls_in_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }));
            let dir = tempfile::TempDir::new().expect("tempdir");
            install_diag_panic_hook_with(
                Some(dir.path().join("panic.jsonl")),
                open_diag_file,
                fail_diag_write,
                fallback,
            );
            let (result, output) = assert_zero_error_metrics(|| {
                capture_fallbacks(|| std::panic::catch_unwind(|| panic!("boom-under-test")))
            });
            std::panic::set_hook(original);

            assert!(result.is_err(), "panic still propagates when capture fails");
            assert_eq!(
                chained_calls.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "the original hook must run exactly once"
            );
            assert_fixed_fallback(&output, FallbackKind::PanicDiagWrite);
        });
    }

    #[test]
    fn panic_diagnostic_open_failure_uses_fixed_fallback_and_zero_metrics() {
        with_env(|_env| {
            let original = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            install_diag_panic_hook_with(
                Some(std::path::PathBuf::from("injected-panic-diag")),
                fail_diag_open,
                write_panic_diag,
                fallback,
            );
            let (result, output) = assert_zero_error_metrics(|| {
                capture_fallbacks(|| std::panic::catch_unwind(|| panic!("boom-under-test")))
            });
            std::panic::set_hook(original);

            assert!(result.is_err(), "panic still propagates when capture fails");
            assert_fixed_fallback(&output, FallbackKind::PanicDiagWrite);
        });
    }

    #[test]
    fn init_tracing_impl_creates_diag_file_when_env_set() {
        with_env(|env| {
            let dir = tempfile::TempDir::new().expect("tempdir");
            env.set(host::capture::DIR_ENV, dir.path());
            let path = dir.path().join("diag.log");
            let previous = std::panic::take_hook();
            init_tracing_impl(false);
            std::panic::set_hook(previous);
            assert!(path.exists(), "diag file should be created when env is set");
        });
    }

    #[test]
    fn init_tracing_impl_survives_unopenable_diag_path() {
        const CHILD: &str = "JAUNDER_TEST_DIAG_OPEN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            assert_error_metric_count(1, || {
                let guard = init_tracing_impl(false);
                drop(guard);
            });
            return;
        }

        let file = tempfile::NamedTempFile::new().expect("temp file");
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .arg("--exact")
            .arg("observability::tests::init_tracing_impl_survives_unopenable_diag_path")
            .arg("--nocapture")
            .env(CHILD, "1")
            .env(host::capture::DIR_ENV, file.path())
            .env_remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .output()
            .expect("run isolated diag-open test");
        assert!(output.status.success(), "child status: {}", output.status);
        let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
        assert_eq!(
            stderr.matches("server.observability.diag_log_open").count(),
            1,
            "stderr: {stderr}"
        );
    }

    #[test]
    fn header_extractor_reads_known_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .expect("valid traceparent header"),
        );

        let extractor = HeaderExtractor(&headers);
        assert_eq!(
            extractor.get("traceparent"),
            Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")
        );
        assert!(extractor.keys().contains(&"traceparent"));
    }

    #[tokio::test]
    async fn trace_context_middleware_inserts_extension() {
        let app = Router::new()
            .route(
                "/",
                axum::routing::get(|req: axum::extract::Request| async move {
                    if req.extensions().get::<ExtractedTraceContext>().is_some() {
                        StatusCode::OK
                    } else {
                        unreachable!("extract_trace_context always inserts ExtractedTraceContext")
                    }
                }),
            )
            .layer(axum::middleware::from_fn(extract_trace_context));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(
                        "traceparent",
                        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
                    )
                    .body(Body::empty())
                    .expect("failed to build request"),
            )
            .await
            .expect("failed to get response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn make_request_span_builds_span_with_and_without_parent_context() {
        // No extracted parent: the span is built without adopting a parent.
        let request = Request::builder()
            .method("GET")
            .uri("/x")
            .body(Body::empty())
            .expect("request");
        let _span = make_request_span(&request);

        // Extracted parent present: exercises the `set_parent` branch.
        let mut request = Request::builder()
            .method("GET")
            .uri("/x")
            .body(Body::empty())
            .expect("request");
        request
            .extensions_mut()
            .insert(ExtractedTraceContext(opentelemetry::Context::new()));
        let _span = make_request_span(&request);
    }
}
