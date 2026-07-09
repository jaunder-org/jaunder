use std::time::{Duration, Instant};

use axum::http::HeaderName;
use axum::Router;
use host::capture;
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn default_filter(verbose: bool) -> EnvFilter {
    if verbose {
        EnvFilter::new("jaunder=debug,web=debug,common=debug,tower_http=debug,sqlx=info")
    } else {
        EnvFilter::new("jaunder=warn,web=warn,common=warn,tower_http=warn,sqlx=warn")
    }
}

fn resolved_filter(verbose: bool) -> EnvFilter {
    EnvFilter::try_from_env("JAUNDER_LOG_FILTER")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| default_filter(verbose))
}

fn use_json_format() -> bool {
    matches!(
        std::env::var("JAUNDER_LOG_FORMAT")
            .unwrap_or_else(|_| "pretty".to_owned())
            .as_str(),
        "json" | "JSON"
    )
}

/// Trim an optional env value and drop it if it is empty (or whitespace-only) —
/// the shared tail of the `JAUNDER_*` readers below, so "blank means unset" stays
/// one rule.
fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn otel_exporter_otlp_endpoint() -> Option<String> {
    // Precedence is by presence: the fallback runs on the raw `.ok()` before the
    // trim, so a *set* primary var wins even if it later trims to empty.
    trimmed_non_empty(
        std::env::var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()),
    )
}

/// The scoped diagnostic-log path (`<JAUNDER_CAPTURE_DIR>/diag.log`), if capture is on.
/// When set (e2e only), the server appends a small JSONL file of WARN+ events plus panic
/// records to it — a purpose-built, low-noise artifact the e2e zero-panic gate
/// consumes, demoting the kernel-laden journal to a fallback (issue #144). Unset in
/// production, so the whole feature is inert there (see the `host` crate).
fn diag_log_file() -> Option<std::path::PathBuf> {
    capture::file(capture::Stream::Diag)
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
fn install_diag_panic_hook(path: Option<std::path::PathBuf>) {
    let Some(path) = path else {
        return;
    };
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            use std::io::Write;
            let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
            let thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned();
            let _ = file.write_all(
                DiagPanicRecord::from_panic(info, &thread, &timestamp)
                    .to_line()
                    .as_bytes(),
            );
        }
        previous(info);
    }));
}

fn build_otel_tracer(
    endpoint: &str,
) -> Result<opentelemetry_sdk::trace::SdkTracerProvider, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP span exporter: {error}"))?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    // Clone into the global registry; keep the original handle so the caller can
    // flush it on exit (a one-shot process exits before the batch processor's
    // interval fires).
    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}

fn build_otel_meter(
    endpoint: &str,
) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider, String> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP metric exporter: {error}"))?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .build();
    // Clone into the global registry; keep the original handle for flush-on-exit
    // (the periodic reader otherwise only exports on its interval).
    opentelemetry::global::set_meter_provider(provider.clone());
    Ok(provider)
}

pub fn slow_op_threshold() -> Duration {
    std::env::var("JAUNDER_SLOW_OP_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or(Duration::from_secs(5), Duration::from_millis)
}

#[derive(Clone, Copy)]
struct SpanStartedAt(Instant);

struct SlowSpanLayer {
    threshold: Duration,
}

impl SlowSpanLayer {
    fn new(threshold: Duration) -> Self {
        Self { threshold }
    }
}

impl<S> Layer<S> for SlowSpanLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        _attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanStartedAt(Instant::now()));
        }
    }

    fn on_close(&self, id: tracing::span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            unreachable!("the tracing Registry guarantees the span is live in on_close")
        };

        let started_at = span.extensions().get::<SpanStartedAt>().copied();
        if let Some((elapsed_ms, threshold_ms)) = slow_span_report(started_at, self.threshold) {
            let metadata = span.metadata();
            let span_name = metadata.name();
            let span_target = metadata.target();
            tracing::warn!(
                span_name,
                span_target,
                elapsed_ms,
                threshold_ms,
                "slow span detected"
            );
        }
    }
}

fn slow_span_values(elapsed: Duration, threshold: Duration) -> Option<(u64, u64)> {
    if elapsed >= threshold {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "latency/threshold durations; their millisecond counts fit u64 for \
                      any realistic elapsed time"
        )]
        Some((elapsed.as_millis() as u64, threshold.as_millis() as u64))
    } else {
        None
    }
}

/// Pure slow-span decision used by [`SlowSpanLayer::on_close`]: reports the
/// `(elapsed_ms, threshold_ms)` to log when a span both recorded its start time
/// and ran for at least `threshold`.
///
/// The `started_at`-absent guard lives here, behind `?`, rather than inline in
/// the layer: a live registry always inserts `SpanStartedAt` in `on_new_span`,
/// so that branch is unreachable through the layer and only this free function
/// can exercise it under test.
fn slow_span_report(started_at: Option<SpanStartedAt>, threshold: Duration) -> Option<(u64, u64)> {
    slow_span_values(started_at?.0.elapsed(), threshold)
}

fn init_tracing_impl(verbose: bool) -> TelemetryGuard {
    // Forward any existing `log` macros to tracing so we can migrate in
    // phases without duplicate logging calls. A failure here is non-fatal (it
    // means a `log` bridge is already installed), but tracing isn't up yet, so
    // we report it to stderr rather than silently dropping it.
    if let Err(error) = tracing_log::LogTracer::init() {
        eprintln!("log-to-tracing bridge init failed (continuing without it): {error}");
    }
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let env_filter = resolved_filter(verbose);
    let slow_span_layer = SlowSpanLayer::new(slow_op_threshold());

    // Box the fmt layer so the json/pretty variants share one type, and carry
    // OTel as an `Option` layer (absent or failed setup is a no-op). This lets
    // every {OTel present/failed/none} × {json/pretty} combination flow through
    // a single registry-build chain.
    let fmt_layer = if use_json_format() {
        fmt::layer().json().boxed()
    } else {
        fmt::layer().boxed()
    };

    // Scoped diagnostic capture (issue #144): when JAUNDER_CAPTURE_DIR is set, append
    // WARN+ events as JSONL to it via a synchronous `Arc<File>` sink — deliberately not
    // a buffered/non-blocking writer, so a `panic = abort` can't drop the very lines the
    // feature exists to keep. An open failure disables the sink (non-fatal) rather than
    // taking down startup. `Option<Layer>` is a no-op when absent, mirroring `otel_layer`.
    // The path is resolved once here and reused for the panic hook installed below.
    let diag_path = diag_log_file();
    let diag_log_layer = diag_path.as_ref().and_then(|path| {
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(file) => Some(diag_layer(std::sync::Arc::new(file))),
            Err(error) => {
                eprintln!(
                    "diag log disabled; could not open {}: {error}",
                    path.display()
                );
                None
            }
        }
    });

    // Resolve the endpoint once; traces and metrics share it. The provider
    // handles are retained in the returned guard so a one-shot process can flush
    // them before exit.
    let endpoint = otel_exporter_otlp_endpoint();

    let tracer = endpoint
        .as_deref()
        .and_then(|endpoint| match build_otel_tracer(endpoint) {
            Ok(provider) => Some(provider),
            Err(error) => {
                eprintln!(
                    "OTel disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                None
            }
        });
    let otel_layer = tracer
        .as_ref()
        .map(|provider| tracing_opentelemetry::layer().with_tracer(provider.tracer("jaunder")));

    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    let meter = endpoint
        .as_deref()
        .and_then(|endpoint| match build_otel_meter(endpoint) {
            Ok(provider) => Some(provider),
            Err(error) => {
                eprintln!(
                    "OTel metrics disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                None
            }
        });

    // `try_init` fails only if a global subscriber is already installed. That
    // leaves the process running without our configured layers, which is worth
    // knowing about; emit to stderr since tracing itself is what failed to come
    // up.
    if let Err(error) = tracing_subscriber::registry()
        .with(env_filter)
        .with(slow_span_layer)
        .with(fmt_layer)
        .with(diag_log_layer)
        .with(otel_layer)
        .try_init()
    {
        eprintln!("tracing subscriber init failed (continuing without it): {error}");
    }

    // Install the scoped-diag panic hook (a no-op when disabled). It is independent of
    // the subscriber above and deliberately does not route through it — see
    // `install_diag_panic_hook` for the deadlock-safety reasoning.
    install_diag_panic_hook(diag_path);

    TelemetryGuard { meter, tracer }
}

#[must_use]
pub fn init_tracing(verbose: bool) -> TelemetryGuard {
    // Called once per process from `run` (production), for every command —
    // `serve` included. The previous `Once` guard is gone because returning an
    // owned guard is incompatible with `call_once`; repeat installs (only seen in
    // tests that dispatch twice in one process) are already reported non-fatally
    // by `try_init`/`LogTracer::init`.
    init_tracing_impl(verbose)
}

/// Owns the OTLP providers installed by [`init_tracing`] so a short-lived
/// process flushes buffered telemetry before exit. The periodic metric reader
/// and batch span processor only export on their interval — which a one-shot
/// CLI command exits long before — so without this the CLI's metric and span
/// emits are silently dropped. Holding the guard for the command's scope and
/// letting `Drop` run `shutdown()` (force-flush + shutdown) exports them on
/// every exit path: success, `?` error-return, and panic unwind.
///
/// Both fields are `None` when no OTLP endpoint is configured, making the guard
/// an inert no-op (the common dev/test case).
pub struct TelemetryGuard {
    meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        // A telemetry-export failure (e.g. the collector is unreachable) must
        // never change the command's outcome, so errors are logged, not
        // propagated — mirroring the non-fatal exporter-setup handling in
        // `init_tracing_impl`.
        if let Some(meter) = self.meter.take() {
            if let Err(error) = meter.shutdown() {
                eprintln!("OTel meter provider shutdown failed during flush: {error}");
            }
        }
        if let Some(tracer) = self.tracer.take() {
            if let Err(error) = tracer.shutdown() {
                eprintln!("OTel tracer provider shutdown failed during flush: {error}");
            }
        }
    }
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
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use std::sync::{Arc, Mutex, MutexGuard};
    use tower::ServiceExt;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        match ENV_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

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
        use std::io::Write;
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut writer = Shared(buf.clone());
        writer.write_all(b"captured").expect("write");
        writer.flush().expect("flush");
        assert_eq!(&*buf.lock().expect("lock"), b"captured");
    }

    #[test]
    fn diag_layer_captures_warn_and_above_not_info_under_global_info_filter() {
        // The load-bearing AND-gate check: the diag layer's per-layer WARN filter must
        // narrow only its own sink, under the same global `info` filter e2e uses — INFO
        // stays out of the diag file but still reaches the other layers.
        let _lock = lock_env();
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

        let diag = String::from_utf8(diag_buf.lock().expect("diag lock").clone()).expect("utf8");
        let other = String::from_utf8(other_buf.lock().expect("other lock").clone()).expect("utf8");

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
    }

    #[test]
    fn diag_log_file_is_none_when_env_unset() {
        let _lock = lock_env();
        std::env::remove_var(host::capture::DIR_ENV);
        assert!(diag_log_file().is_none());
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
        assert!(parsed["message"]
            .as_str()
            .expect("message string")
            .contains("panicked at"));
    }

    #[test]
    fn installed_diag_panic_hook_appends_record_and_restores() {
        let _lock = lock_env();
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
        assert!(messages
            .iter()
            .all(|message| message.contains("panicked at")));
        assert!(messages[0].contains("boom-under-test"));
        assert!(messages[1].contains("formatted-payload"));
        assert!(messages[2].contains("<non-string panic payload>"));
    }

    #[test]
    fn diag_panic_hook_tolerates_unwritable_path() {
        // A directory can't be opened as a file, so the hook's append fails — it must
        // swallow that and let the panic propagate cleanly (covers the open-failure
        // arm inside the hook, which the writable-path test never reaches).
        let _lock = lock_env();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let previous = std::panic::take_hook();
        install_diag_panic_hook(Some(dir.path().to_path_buf()));
        let result = std::panic::catch_unwind(|| panic!("boom-into-directory"));
        std::panic::set_hook(previous);
        assert!(result.is_err(), "panic still propagates when capture fails");
    }

    #[test]
    fn slow_span_values_returns_none_when_below_threshold() {
        let values = slow_span_values(Duration::from_millis(499), Duration::from_millis(500));
        assert!(values.is_none());
    }

    #[test]
    fn slow_span_values_returns_some_when_equal_or_above_threshold() {
        let equal = slow_span_values(Duration::from_millis(500), Duration::from_millis(500));
        assert_eq!(equal, Some((500, 500)));

        let above = slow_span_values(Duration::from_millis(750), Duration::from_millis(500));
        assert_eq!(above, Some((750, 500)));
    }

    #[test]
    fn slow_span_report_is_none_when_start_time_absent() {
        // A live registry always records SpanStartedAt in on_new_span, so this
        // guard is unreachable through the layer; cover it directly here.
        assert_eq!(slow_span_report(None, Duration::from_millis(1)), None);
    }

    #[test]
    fn slow_span_report_reports_when_started_span_exceeds_threshold() {
        let started_at = SpanStartedAt(
            Instant::now()
                .checked_sub(Duration::from_secs(10))
                .expect("monotonic clock far enough past epoch"),
        );
        assert!(slow_span_report(Some(started_at), Duration::from_millis(1)).is_some());
    }

    #[test]
    fn slow_op_threshold_defaults_to_five_seconds() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_SLOW_OP_MS");
        assert_eq!(slow_op_threshold(), Duration::from_secs(5));
    }

    #[test]
    fn slow_op_threshold_reads_environment_override() {
        let _guard = lock_env();
        std::env::set_var("JAUNDER_SLOW_OP_MS", "1234");
        assert_eq!(slow_op_threshold(), Duration::from_millis(1234));
        std::env::remove_var("JAUNDER_SLOW_OP_MS");
    }

    #[test]
    fn otlp_endpoint_prefers_jaunder_specific_setting() {
        let _guard = lock_env();
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://fallback:4317");
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://preferred:4317",
        );

        assert_eq!(
            otel_exporter_otlp_endpoint().as_deref(),
            Some("http://preferred:4317")
        );

        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn otlp_endpoint_falls_back_to_standard_env_var() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://fallback:4317");

        assert_eq!(
            otel_exporter_otlp_endpoint().as_deref(),
            Some("http://fallback:4317")
        );

        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn use_json_format_defaults_to_pretty() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_LOG_FORMAT");
        assert!(!use_json_format());
    }

    #[test]
    fn use_json_format_accepts_json() {
        let _guard = lock_env();
        std::env::set_var("JAUNDER_LOG_FORMAT", "json");
        assert!(use_json_format());
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }

    #[tokio::test]
    async fn build_otel_tracer_accepts_valid_endpoint() {
        let tracer = build_otel_tracer("http://127.0.0.1:4317");
        assert!(tracer.is_ok());
    }

    #[tokio::test]
    async fn build_otel_meter_accepts_valid_endpoint() {
        assert!(build_otel_meter("http://127.0.0.1:4317").is_ok());
    }

    #[tokio::test]
    async fn build_otel_meter_with_endpoint_is_wired_by_init() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://127.0.0.1:4317",
        );
        // The returned TelemetryGuard is an unbound temporary that drops here,
        // so this (and the other valid-endpoint init_tracing_impl tests below)
        // performs a real shutdown()/force-flush against 127.0.0.1:4317. It
        // returns promptly because the connection is refused — if one of these
        // ever hangs in CI, an unreachable-but-not-refused endpoint is the place
        // to look.
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "not a valid endpoint",
        );
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[test]
    fn init_tracing_impl_creates_diag_file_when_env_set() {
        let _guard = lock_env();
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::env::set_var(host::capture::DIR_ENV, dir.path());
        let path = dir.path().join("diag.log");
        // `init_tracing_impl` installs the global panic hook when the env is set;
        // save/restore it so it can't fire on a later test writing to this TempDir.
        let previous = std::panic::take_hook();
        // `OpenOptions::create` makes the file on open — independent of whether this
        // process's `try_init` wins the global-subscriber slot — so the sink's file
        // exists even when a prior test already installed the subscriber.
        init_tracing_impl(false);
        std::panic::set_hook(previous);
        std::env::remove_var(host::capture::DIR_ENV);
        assert!(path.exists(), "diag file should be created when env is set");
    }

    #[test]
    fn init_tracing_impl_survives_unopenable_diag_path() {
        let _guard = lock_env();
        // Point JAUNDER_CAPTURE_DIR at a regular FILE: `capture::file` can't create the
        // dir and opening `<file>/diag.log` fails, exercising the non-fatal
        // `Err`/`eprintln` arm without taking down startup. (Pointing at a directory
        // would now succeed — `capture::file` create_dir_all's it and joins `diag.log`.)
        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::env::set_var(host::capture::DIR_ENV, file.path());
        let previous = std::panic::take_hook();
        init_tracing_impl(false);
        std::panic::set_hook(previous);
        std::env::remove_var(host::capture::DIR_ENV);
    }

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint_with_json_output() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "still not a valid endpoint",
        );
        std::env::set_var("JAUNDER_LOG_FORMAT", "json");
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }

    #[test]
    fn init_tracing_impl_handles_no_otel_endpoint_with_json_output() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::set_var("JAUNDER_LOG_FORMAT", "json");
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }

    #[tokio::test]
    async fn init_tracing_impl_handles_valid_otel_endpoint_with_pretty_output() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://127.0.0.1:4317",
        );
        std::env::remove_var("JAUNDER_LOG_FORMAT");
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
    }

    #[tokio::test]
    async fn init_tracing_impl_handles_valid_otel_endpoint_with_json_output() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "http://127.0.0.1:4317",
        );
        std::env::set_var("JAUNDER_LOG_FORMAT", "json");
        init_tracing_impl(false);
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }

    #[test]
    fn init_tracing_impl_reports_failure_when_already_initialized() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        // First call installs the global subscriber and log bridge; the second
        // finds both already set and exercises the non-fatal error branches
        // (reported to stderr rather than silently dropped).
        init_tracing_impl(false);
        init_tracing_impl(false);
    }

    #[test]
    fn default_filter_verbose_sets_debug() {
        let filter = default_filter(true);
        let debug_str = format!("{filter:?}");
        assert!(
            debug_str.contains("LevelFilter::DEBUG"),
            "debug_str: {debug_str}"
        );
    }

    #[test]
    fn default_filter_quiet_sets_warn() {
        let filter = default_filter(false);
        let warn_str = format!("{filter:?}");
        assert!(
            warn_str.contains("LevelFilter::WARN"),
            "warn_str: {warn_str}"
        );
    }

    #[test]
    fn slow_span_layer_records_started_at_and_warns_when_elapsed_exceeds_threshold() {
        let layer = SlowSpanLayer::new(Duration::from_nanos(1));
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!("slow_test_span");
        let entered = span.enter();
        std::thread::sleep(Duration::from_millis(2));
        drop(entered);
        drop(span);
    }

    #[test]
    fn slow_span_layer_skips_warning_when_below_threshold() {
        let layer = SlowSpanLayer::new(Duration::from_hours(1));
        let subscriber = tracing_subscriber::registry().with(layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        let span = tracing::info_span!("fast_test_span");
        drop(span);
    }

    #[test]
    fn lock_env_recovers_from_poisoned_mutex() {
        // Poison ENV_LOCK by panicking while holding it inside catch_unwind.
        let _ = std::panic::catch_unwind(|| {
            let _guard = ENV_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            panic!("poison the mutex");
        });
        // lock_env() must recover gracefully from the poisoned mutex.
        let _guard = lock_env();
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

    #[tokio::test]
    async fn guard_drop_flushes_meter_provider() {
        use opentelemetry::metrics::MeterProvider as _;
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();

        // Emit a counter on this provider, then let the guard's Drop flush it.
        let counter = provider.meter("test").u64_counter("test.counter").build();
        counter.add(1, &[]);
        drop(TelemetryGuard {
            meter: Some(provider),
            tracer: None,
        });

        let metrics = exporter.get_finished_metrics().expect("metrics");
        let found = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .any(|metric| metric.name() == "test.counter");
        assert!(found, "metric not exported on guard drop");
    }

    #[tokio::test]
    async fn guard_drop_flushes_tracer_provider() {
        use opentelemetry::trace::{Tracer as _, TracerProvider as _};
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter.clone())
            .build();

        provider.tracer("test").in_span("test-span", |_cx| {});
        drop(TelemetryGuard {
            meter: None,
            tracer: Some(provider),
        });

        let spans = exporter.get_finished_spans().expect("spans");
        assert!(
            spans.iter().any(|span| span.name == "test-span"),
            "span not exported on guard drop"
        );
    }

    #[test]
    fn guard_drop_is_noop_when_inert() {
        // No OTLP endpoint configured -> both providers None -> Drop does nothing
        // and must not panic.
        drop(TelemetryGuard {
            meter: None,
            tracer: None,
        });
    }

    #[tokio::test]
    async fn guard_drop_swallows_shutdown_errors() {
        let meter = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(InMemoryMetricExporter::default()).build())
            .build();
        let tracer = SdkTracerProvider::builder()
            .with_batch_exporter(InMemorySpanExporter::default())
            .build();

        // Shut both down once cleanly, then assert a second shutdown reports an
        // error — that is exactly the condition the guard's Drop must swallow.
        // Asserting it here keeps the test meaningful even if a future OTel
        // version made shutdown() idempotently return Ok (the Drop Err arms would
        // otherwise go silently uncovered).
        meter.shutdown().expect("first meter shutdown succeeds");
        tracer.shutdown().expect("first tracer shutdown succeeds");
        assert!(
            meter.shutdown().is_err(),
            "second meter shutdown should error"
        );
        assert!(
            tracer.shutdown().is_err(),
            "second tracer shutdown should error"
        );

        // The guard's Drop now calls shutdown() on already-shut-down providers; it
        // must log and swallow the error, not panic or propagate. Covers both Err
        // arms in Drop.
        drop(TelemetryGuard {
            meter: Some(meter),
            tracer: Some(tracer),
        });
    }
}
