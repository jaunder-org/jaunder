//! Shared host-process telemetry setup and shutdown.
//!
//! This module owns the OpenTelemetry providers used by native binaries that
//! write application state: `jaunder serve`, one-shot `jaunder` CLI commands,
//! and `test-support`. It installs tracing/log/metric layers when an OTLP
//! endpoint is configured, keeps setup failures non-fatal, and returns a
//! [`TelemetryGuard`] whose drop path flushes short-lived process telemetry
//! before exit. Server-only request diagnostics stay in `server::observability`.

use std::time::{Duration, Instant};

use anyhow::Context as _;
use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::Resource;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

fn default_filter(verbose: bool) -> EnvFilter {
    if verbose {
        EnvFilter::new(
            "jaunder=debug,host=debug,web=debug,common=debug,tower_http=debug,sqlx=info,storage=debug",
        )
    } else {
        EnvFilter::new(
            "jaunder=warn,host=warn,web=warn,common=warn,tower_http=warn,sqlx=warn,storage=info",
        )
    }
}

const E2E_SEED_PROCESS_ENV: &str = "JAUNDER_E2E_SEED_PROCESS";
const E2E_SEED_PROCESS_ATTR: &str = "jaunder.e2e.seed_process";
const E2E_SEED_PROCESS_JAUNDER: &str = "e2e.seed.jaunder";
const E2E_SEED_PROCESS_TEST_SUPPORT: &str = "e2e.seed.test-support";

fn read_env(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error @ std::env::VarError::NotUnicode(_)) => Err(error),
    }
}

#[derive(Clone, Copy)]
enum FallbackKind {
    TracerExporterSetup,
    MeterExporterSetup,
    MeterShutdown,
    TracerShutdown,
    SubscriberInstall,
    OtlpEndpoint,
    LogFilter,
    SlowThreshold,
    LogFormat,
}

impl FallbackKind {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::TracerExporterSetup => (
                "host.telemetry.tracer_exporter_setup",
                "tracing export disabled",
            ),
            Self::MeterExporterSetup => (
                "host.telemetry.meter_exporter_setup",
                "metrics export disabled",
            ),
            Self::MeterShutdown => ("host.telemetry.meter_shutdown", "telemetry shutdown failed"),
            Self::TracerShutdown => (
                "host.telemetry.tracer_shutdown",
                "telemetry shutdown failed",
            ),
            Self::SubscriberInstall => (
                "host.telemetry.subscriber_install",
                "subscriber installation failed; continuing",
            ),
            Self::OtlpEndpoint => (
                "host.telemetry.otlp_endpoint",
                "invalid configured value; export disabled",
            ),
            Self::LogFilter => (
                "host.telemetry.log_filter",
                "invalid configured value; using default filter",
            ),
            Self::SlowThreshold => (
                "host.telemetry.slow_threshold",
                "invalid configured value; using 5s",
            ),
            Self::LogFormat => (
                "host.telemetry.log_format",
                "invalid configured value; using pretty format",
            ),
        }
    }
}

fn write_fallback(mut writer: impl std::io::Write, kind: FallbackKind) -> std::io::Result<()> {
    let (context, message) = kind.parts();
    writeln!(writer, "{context}: {message}")
}
#[cfg(test)]
mod fallback_capture {
    use super::{FallbackKind, write_fallback};

    struct Capture {
        owner: std::thread::ThreadId,
        output: Vec<u8>,
    }

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static OUTPUT: std::sync::Mutex<Option<Capture>> = std::sync::Mutex::new(None);

    pub(super) fn capture<R>(operation: impl FnOnce() -> R) -> (R, String) {
        let _serial = SERIAL.lock().expect("fallback capture serial");
        {
            let mut capture = OUTPUT.lock().expect("fallback capture lock");
            assert!(capture.is_none(), "nested fallback capture");
            *capture = Some(Capture {
                owner: std::thread::current().id(),
                output: Vec::new(),
            });
        }
        let result = operation();
        let output = OUTPUT
            .lock()
            .expect("fallback capture lock")
            .take()
            .expect("capture")
            .output;
        (result, String::from_utf8(output).expect("fallback utf8"))
    }

    pub(super) fn try_capture(kind: FallbackKind) -> bool {
        let owner = std::thread::current().id();
        let mut output = OUTPUT.lock().expect("fallback capture lock");
        output
            .as_mut()
            .filter(|capture| capture.owner == owner)
            .is_some_and(|capture| write_fallback(&mut capture.output, kind).is_ok())
    }
}

fn fallback(kind: FallbackKind) {
    #[cfg(test)]
    let captured = fallback_capture::try_capture(kind);
    #[cfg(test)]
    if captured {
        return;
    }
    let _ = write_fallback(std::io::stderr().lock(), kind);
}

fn write_exporter_fallback(
    writer: impl std::io::Write,
    kind: FallbackKind,
    _error: &anyhow::Error,
) -> std::io::Result<()> {
    write_fallback(writer, kind)
}

fn exporter_fallback(kind: FallbackKind, error: &anyhow::Error) {
    let _ = write_exporter_fallback(std::io::stderr().lock(), kind, error);
}

fn resolved_filter_with(
    verbose: bool,
    mut read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> EnvFilter {
    for name in ["JAUNDER_LOG_FILTER", "RUST_LOG"] {
        match read(name) {
            Ok(Some(value)) if value.trim().is_empty() => warn(),
            Ok(Some(value)) => match EnvFilter::try_new(&value) {
                Ok(filter) => return filter,
                Err(_) => warn(),
            },
            Ok(None) => {}
            Err(_) => warn(),
        }
    }
    default_filter(verbose)
}

fn resolved_filter(verbose: bool) -> EnvFilter {
    resolved_filter_with(verbose, read_env, || fallback(FallbackKind::LogFilter))
}

fn use_json_format_with(
    mut read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> bool {
    match read("JAUNDER_LOG_FORMAT") {
        Ok(Some(value)) if matches!(value.as_str(), "json" | "JSON") => true,
        Ok(Some(value)) if value == "pretty" => false,
        Ok(Some(_)) | Err(_) => {
            warn();
            false
        }
        Ok(None) => false,
    }
}

fn use_json_format() -> bool {
    use_json_format_with(read_env, || fallback(FallbackKind::LogFormat))
}

/// Trim an optional env value and drop it if it is empty (or whitespace-only) —
/// the shared tail of the `JAUNDER_*` readers below, so "blank means unset" stays
/// one rule.
fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn otel_exporter_otlp_endpoint_with(
    mut read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> Option<String> {
    match read("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(Some(value)) => {
            return if let Some(value) = trimmed_non_empty(Some(value)) {
                Some(value)
            } else {
                warn();
                None
            };
        }
        Err(_) => {
            warn();
            return None;
        }
        Ok(None) => {}
    }

    match read("OTEL_EXPORTER_OTLP_ENDPOINT") {
        Ok(Some(value)) => {
            if let Some(value) = trimmed_non_empty(Some(value)) {
                Some(value)
            } else {
                warn();
                None
            }
        }
        Ok(None) => None,
        Err(_) => {
            warn();
            None
        }
    }
}

fn otel_exporter_otlp_endpoint() -> Option<String> {
    otel_exporter_otlp_endpoint_with(read_env, || fallback(FallbackKind::OtlpEndpoint))
}
fn e2e_seed_process_attribute_with(
    mut read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
) -> Option<KeyValue> {
    let value = trimmed_non_empty(read(E2E_SEED_PROCESS_ENV).ok()?)?;
    if matches!(
        value.as_str(),
        E2E_SEED_PROCESS_JAUNDER | E2E_SEED_PROCESS_TEST_SUPPORT
    ) {
        Some(KeyValue::new(E2E_SEED_PROCESS_ATTR, value))
    } else {
        None
    }
}

fn telemetry_resource_with(
    read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
) -> Resource {
    let builder = Resource::builder();
    if let Some(attribute) = e2e_seed_process_attribute_with(read) {
        builder.with_attribute(attribute).build()
    } else {
        builder.build()
    }
}

fn telemetry_resource() -> Resource {
    telemetry_resource_with(read_env)
}

fn build_otel_tracer(
    endpoint: &str,
    resource: Resource,
) -> anyhow::Result<opentelemetry_sdk::trace::SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .context("failed to build OTLP span exporter")?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(exporter)
        .build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    Ok(provider)
}

fn build_otel_meter(
    endpoint: &str,
    resource: Resource,
) -> anyhow::Result<opentelemetry_sdk::metrics::SdkMeterProvider> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .context("failed to build OTLP metric exporter")?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_resource(resource)
        .with_periodic_exporter(exporter)
        .build();
    opentelemetry::global::set_meter_provider(provider.clone());
    Ok(provider)
}
fn setup_otel_tracer_with<T>(
    endpoint: &str,
    build: impl FnOnce(&str) -> anyhow::Result<T>,
    on_error: impl FnOnce(&anyhow::Error),
) -> Option<T> {
    match build(endpoint) {
        Ok(provider) => Some(provider),
        Err(error) => {
            on_error(&error);
            None
        }
    }
}

fn setup_otel_meter_with<T>(
    endpoint: &str,
    build: impl FnOnce(&str) -> anyhow::Result<T>,
    on_error: impl FnOnce(&anyhow::Error),
) -> Option<T> {
    match build(endpoint) {
        Ok(provider) => Some(provider),
        Err(error) => {
            on_error(&error);
            None
        }
    }
}

fn slow_op_threshold_with(
    mut read: impl FnMut(&str) -> Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> Duration {
    match read("JAUNDER_SLOW_OP_MS") {
        Ok(Some(value)) => {
            if let Ok(value) = value.parse::<u64>() {
                Duration::from_millis(value)
            } else {
                warn();
                Duration::from_secs(5)
            }
        }
        Ok(None) => Duration::from_secs(5),
        Err(_) => {
            warn();
            Duration::from_secs(5)
        }
    }
}

fn slow_op_threshold() -> Duration {
    slow_op_threshold_with(read_env, || fallback(FallbackKind::SlowThreshold))
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
        // `Duration::as_millis` is `u128`. Saturating rather than truncating: a
        // duration past `u64::MAX` milliseconds (~584 million years) cannot occur,
        // but if the arithmetic ever produced one, reporting the maximum is honest
        // where a wrapped value would read as a fast span and hide the very thing
        // this layer exists to surface.
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        Some((ms(elapsed), ms(threshold)))
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

fn install_subscriber_with<E>(install: impl FnOnce() -> Result<(), E>, mut warn: impl FnMut()) {
    if install().is_err() {
        warn();
    }
}

fn init_tracing_impl_with_layer<L>(verbose: bool, extra_layer: Option<L>) -> TelemetryGuard
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
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

    // Resolve the endpoint once; traces and metrics share it. The provider
    // handles are retained in the returned guard so a one-shot process can flush
    // them before exit.
    let endpoint = otel_exporter_otlp_endpoint();
    let resource = telemetry_resource();

    let tracer = endpoint.as_deref().and_then(|endpoint| {
        setup_otel_tracer_with(
            endpoint,
            |endpoint| build_otel_tracer(endpoint, resource.clone()),
            |error| {
                exporter_fallback(FallbackKind::TracerExporterSetup, error);
            },
        )
    });
    let otel_layer = tracer
        .as_ref()
        .map(|provider| tracing_opentelemetry::layer().with_tracer(provider.tracer("jaunder")));

    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    let meter = endpoint.as_deref().and_then(|endpoint| {
        setup_otel_meter_with(
            endpoint,
            |endpoint| build_otel_meter(endpoint, resource),
            |error| {
                exporter_fallback(FallbackKind::MeterExporterSetup, error);
            },
        )
    });

    // `try_init` fails only if a global subscriber is already installed. That
    // leaves the process running without our configured layers, which is worth
    // knowing about; emit to stderr since tracing itself is what failed to come
    // up.
    install_subscriber_with(
        || {
            tracing_subscriber::registry()
                .with(extra_layer)
                .with(env_filter)
                .with(slow_span_layer)
                .with(fmt_layer)
                .with(otel_layer)
                .try_init()
        },
        || fallback(FallbackKind::SubscriberInstall),
    );

    TelemetryGuard {
        meter,
        tracer,
        meter_shutdown: shutdown_meter,
        tracer_shutdown: shutdown_tracer,
    }
}

fn init_tracing_impl(verbose: bool) -> TelemetryGuard {
    init_tracing_impl_with_layer::<tracing_subscriber::layer::Identity>(verbose, None)
}

/// Install the process-wide tracing/logging/metrics subscriber and return its
/// shutdown guard.
///
/// The setup is intentionally best-effort: malformed OTLP endpoints, exporter
/// construction failures, and duplicate subscriber installs are recorded through
/// local diagnostics but do not prevent the command from running.
#[must_use]
pub fn init_tracing(verbose: bool) -> TelemetryGuard {
    // Called once per process from `run` (production), for every command —
    // `serve` included. No `Once` guard: returning an owned guard is incompatible
    // with `call_once`, and repeat installs (only seen in tests that dispatch twice
    // in one process) are already reported non-fatally by
    // `try_init`/`LogTracer::init`.
    init_tracing_impl(verbose)
}

/// Install tracing with a caller-owned extra subscriber layer and return the
/// same shutdown guard as [`init_tracing`].
#[must_use]
pub fn init_tracing_with_layer<L>(verbose: bool, layer: Option<L>) -> TelemetryGuard
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    init_tracing_impl_with_layer(verbose, layer)
}

type MeterShutdownOperation =
    fn(&opentelemetry_sdk::metrics::SdkMeterProvider) -> opentelemetry_sdk::error::OTelSdkResult;
type TracerShutdownOperation =
    fn(&opentelemetry_sdk::trace::SdkTracerProvider) -> opentelemetry_sdk::error::OTelSdkResult;

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
    meter_shutdown: MeterShutdownOperation,
    tracer_shutdown: TracerShutdownOperation,
}

fn shutdown_meter(
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
) -> opentelemetry_sdk::error::OTelSdkResult {
    provider.shutdown()
}

fn shutdown_tracer(
    provider: &opentelemetry_sdk::trace::SdkTracerProvider,
) -> opentelemetry_sdk::error::OTelSdkResult {
    provider.shutdown()
}

fn finish_meter_shutdown(
    provider: &opentelemetry_sdk::metrics::SdkMeterProvider,
    operation: MeterShutdownOperation,
) {
    if operation(provider).is_err() {
        fallback(FallbackKind::MeterShutdown);
    }
}

fn finish_tracer_shutdown(
    provider: &opentelemetry_sdk::trace::SdkTracerProvider,
    operation: TracerShutdownOperation,
) {
    if operation(provider).is_err() {
        fallback(FallbackKind::TracerShutdown);
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(meter) = self.meter.as_ref() {
            finish_meter_shutdown(meter, self.meter_shutdown);
        }
        if let Some(tracer) = self.tracer.as_ref() {
            finish_tracer_shutdown(tracer, self.tracer_shutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::test_support::with_env;
    use opentelemetry::Value;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

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
    fn invalid_unicode_env() -> std::env::VarError {
        std::env::VarError::NotUnicode(std::ffi::OsString::from("injected invalid unicode"))
    }

    fn push_fallback(output: &mut Vec<u8>, kind: FallbackKind) {
        write_fallback(output, kind).expect("write fallback");
    }

    #[test]
    fn process_telemetry_does_not_create_diag_file_when_capture_dir_is_set() {
        with_env(|env| {
            let dir = tempfile::TempDir::new().expect("tempdir");
            env.set(crate::capture::DIR_ENV, dir.path());
            env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
            env.remove("OTEL_EXPORTER_OTLP_ENDPOINT");
            let _guard = init_tracing(false);
            assert!(!dir.path().join("diag.log").exists());
        });
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
        with_env(|env| {
            env.remove("JAUNDER_SLOW_OP_MS");
            assert_eq!(slow_op_threshold(), Duration::from_secs(5));
        });
    }

    #[test]
    fn slow_op_threshold_reads_environment_override() {
        with_env(|env| {
            env.set("JAUNDER_SLOW_OP_MS", "1234");
            assert_eq!(slow_op_threshold(), Duration::from_millis(1234));
        });
    }

    #[test]
    fn nonnumeric_slow_threshold_uses_default_with_one_fixed_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let threshold = assert_zero_error_metrics(|| {
            slow_op_threshold_with(
                |name| {
                    assert_eq!(name, "JAUNDER_SLOW_OP_MS");
                    Ok(Some("not-a-number".to_owned()))
                },
                || push_fallback(&mut output, FallbackKind::SlowThreshold),
            )
        });
        assert_eq!(threshold, Duration::from_secs(5));
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::SlowThreshold,
        );
    }

    #[test]
    fn invalid_unicode_slow_threshold_uses_default_with_one_redacted_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let threshold = assert_zero_error_metrics(|| {
            slow_op_threshold_with(
                |name| {
                    assert_eq!(name, "JAUNDER_SLOW_OP_MS");
                    Err(invalid_unicode_env())
                },
                || push_fallback(&mut output, FallbackKind::SlowThreshold),
            )
        });
        assert_eq!(threshold, Duration::from_secs(5));
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::SlowThreshold,
        );
    }
    #[test]
    fn otlp_endpoint_prefers_jaunder_specific_setting() {
        with_env(|env| {
            env.set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://fallback:4317");
            env.set(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "http://preferred:4317",
            );
            assert_eq!(
                otel_exporter_otlp_endpoint().as_deref(),
                Some("http://preferred:4317")
            );
        });
    }

    #[test]
    fn otlp_endpoint_falls_back_to_standard_env_var() {
        with_env(|env| {
            env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
            env.set("OTEL_EXPORTER_OTLP_ENDPOINT", "http://fallback:4317");
            assert_eq!(
                otel_exporter_otlp_endpoint().as_deref(),
                Some("http://fallback:4317")
            );
        });
    }

    #[test]
    fn blank_primary_endpoint_disables_export_with_one_fixed_fallback() {
        let mut output = Vec::new();
        let endpoint = otel_exporter_otlp_endpoint_with(
            |name| {
                assert_eq!(
                    name, "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                    "blank primary endpoint must stop precedence resolution"
                );
                Ok(Some("   ".to_owned()))
            },
            || push_fallback(&mut output, FallbackKind::OtlpEndpoint),
        );
        assert!(endpoint.is_none());
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::OtlpEndpoint,
        );
    }

    #[test]
    fn blank_secondary_endpoint_disables_export_with_one_fixed_fallback() {
        let mut output = Vec::new();
        let endpoint = otel_exporter_otlp_endpoint_with(
            |name| match name {
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT" => Ok(None),
                "OTEL_EXPORTER_OTLP_ENDPOINT" => Ok(Some("   ".to_owned())),
                _ => unreachable!("the endpoint reader receives only the two known names"),
            },
            || push_fallback(&mut output, FallbackKind::OtlpEndpoint),
        );
        assert!(endpoint.is_none());
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::OtlpEndpoint,
        );
    }

    #[test]
    fn invalid_unicode_primary_endpoint_does_not_select_secondary_and_records_zero_metrics() {
        let mut output = Vec::new();
        let endpoint = assert_zero_error_metrics(|| {
            otel_exporter_otlp_endpoint_with(
                |name| {
                    assert_eq!(
                        name, "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                        "invalid primary endpoint must stop precedence resolution"
                    );
                    Err(invalid_unicode_env())
                },
                || push_fallback(&mut output, FallbackKind::OtlpEndpoint),
            )
        });
        assert!(endpoint.is_none(), "invalid primary disables export");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::OtlpEndpoint,
        );
    }

    #[test]
    fn invalid_unicode_secondary_endpoint_disables_export_and_records_zero_metrics() {
        let mut output = Vec::new();
        let endpoint = assert_zero_error_metrics(|| {
            otel_exporter_otlp_endpoint_with(
                |name| match name {
                    "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT" => Ok(None),
                    "OTEL_EXPORTER_OTLP_ENDPOINT" => Err(invalid_unicode_env()),
                    _ => unreachable!("the endpoint reader receives only the two known names"),
                },
                || push_fallback(&mut output, FallbackKind::OtlpEndpoint),
            )
        });
        assert!(endpoint.is_none(), "invalid secondary disables export");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::OtlpEndpoint,
        );
    }

    #[test]
    fn use_json_format_defaults_to_pretty() {
        with_env(|env| {
            env.remove("JAUNDER_LOG_FORMAT");
            assert!(!use_json_format());
        });
    }

    #[test]
    fn use_json_format_accepts_json() {
        with_env(|env| {
            env.set("JAUNDER_LOG_FORMAT", "json");
            assert!(use_json_format());
        });
    }

    #[test]
    fn use_json_format_accepts_pretty() {
        with_env(|env| {
            env.set("JAUNDER_LOG_FORMAT", "pretty");
            assert!(!use_json_format());
        });
    }

    #[cfg(unix)]
    #[test]
    fn production_env_reader_rejects_invalid_unicode() {
        use std::os::unix::ffi::OsStringExt as _;

        with_env(|env| {
            env.set(
                "JAUNDER_LOG_FORMAT",
                std::ffi::OsString::from_vec(vec![0xff]),
            );
            assert!(!use_json_format());
        });
    }

    #[test]
    fn invalid_unicode_log_format_uses_pretty_with_one_redacted_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let use_json = assert_zero_error_metrics(|| {
            use_json_format_with(
                |name| {
                    assert_eq!(name, "JAUNDER_LOG_FORMAT");
                    Err(invalid_unicode_env())
                },
                || push_fallback(&mut output, FallbackKind::LogFormat),
            )
        });
        assert!(!use_json, "pretty format remains the invalid-value default");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::LogFormat,
        );
    }

    #[test]
    fn resolved_filter_accepts_valid_jaunder_directive() {
        with_env(|env| {
            env.set("JAUNDER_LOG_FILTER", "jaunder=info");
            env.remove("RUST_LOG");
            assert_eq!(
                format!("{:?}", resolved_filter(false)),
                format!("{:?}", EnvFilter::new("jaunder=info"))
            );
        });
    }

    #[test]
    fn invalid_log_filter_directive_uses_default_with_one_fixed_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let filter = assert_zero_error_metrics(|| {
            resolved_filter_with(
                false,
                |name| match name {
                    "JAUNDER_LOG_FILTER" => Ok(Some("[not-a-directive".to_owned())),
                    "RUST_LOG" => Ok(None),
                    _ => unreachable!("the filter reader receives only the two known names"),
                },
                || push_fallback(&mut output, FallbackKind::LogFilter),
            )
        });
        assert_eq!(
            format!("{filter:?}"),
            format!("{:?}", default_filter(false))
        );
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::LogFilter,
        );
    }

    #[test]
    fn invalid_unicode_jaunder_log_filter_uses_default_with_one_redacted_fallback_and_zero_metrics()
    {
        let mut output = Vec::new();
        let filter = assert_zero_error_metrics(|| {
            resolved_filter_with(
                false,
                |name| match name {
                    "JAUNDER_LOG_FILTER" => Err(invalid_unicode_env()),
                    "RUST_LOG" => Ok(None),
                    _ => unreachable!("the filter reader receives only the two known names"),
                },
                || push_fallback(&mut output, FallbackKind::LogFilter),
            )
        });
        assert_eq!(
            format!("{filter:?}"),
            format!("{:?}", default_filter(false))
        );
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::LogFilter,
        );
    }

    #[test]
    fn invalid_unicode_rust_log_uses_default_with_one_redacted_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let filter = assert_zero_error_metrics(|| {
            resolved_filter_with(
                false,
                |name| match name {
                    "JAUNDER_LOG_FILTER" => Ok(None),
                    "RUST_LOG" => Err(invalid_unicode_env()),
                    _ => unreachable!("the filter reader receives only the two known names"),
                },
                || push_fallback(&mut output, FallbackKind::LogFilter),
            )
        });
        assert_eq!(
            format!("{filter:?}"),
            format!("{:?}", default_filter(false))
        );
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::LogFilter,
        );
    }

    #[test]
    fn telemetry_resource_records_closed_e2e_seed_process_marker() {
        let resource = telemetry_resource_with(|name| {
            assert_eq!(name, E2E_SEED_PROCESS_ENV);
            Ok(Some(E2E_SEED_PROCESS_TEST_SUPPORT.to_owned()))
        });
        assert_eq!(
            resource.get(&opentelemetry::Key::from_static_str(E2E_SEED_PROCESS_ATTR)),
            Some(Value::from(E2E_SEED_PROCESS_TEST_SUPPORT))
        );
    }

    #[test]
    fn telemetry_resource_ignores_unrecognised_e2e_seed_process_marker() {
        let resource = telemetry_resource_with(|name| {
            assert_eq!(name, E2E_SEED_PROCESS_ENV);
            Ok(Some("user-controlled".to_owned()))
        });
        assert!(
            resource
                .get(&opentelemetry::Key::from_static_str(E2E_SEED_PROCESS_ATTR))
                .is_none()
        );
    }

    #[tokio::test]
    async fn build_otel_tracer_accepts_valid_endpoint() {
        let tracer = build_otel_tracer("http://127.0.0.1:4317", Resource::builder().build());
        assert!(tracer.is_ok());
    }

    #[test]
    fn tracer_exporter_failure_preserves_typed_source_at_fallback_and_records_zero_metrics() {
        let injected = build_otel_tracer("not a valid endpoint", Resource::builder().build())
            .expect_err("invalid endpoint");
        let mut output = Vec::new();
        let source_seen = std::cell::Cell::new(false);
        let provider = assert_zero_error_metrics(|| {
            setup_otel_tracer_with(
                "injected endpoint",
                move |_| Err::<(), _>(injected),
                |error| {
                    assert_eq!(error.to_string(), "failed to build OTLP span exporter");
                    assert!(
                        error
                            .downcast_ref::<opentelemetry_otlp::ExporterBuildError>()
                            .is_some(),
                        "concrete OTLP exporter error was erased before fallback"
                    );
                    source_seen.set(true);
                    write_exporter_fallback(&mut output, FallbackKind::TracerExporterSetup, error)
                        .expect("write fallback");
                },
            )
        });
        assert!(
            provider.is_none(),
            "startup continues with tracing export disabled"
        );
        assert!(source_seen.get(), "typed source reached fallback seam");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::TracerExporterSetup,
        );
    }

    #[tokio::test]
    async fn build_otel_meter_accepts_valid_endpoint() {
        assert!(build_otel_meter("http://127.0.0.1:4317", Resource::builder().build()).is_ok());
    }

    #[test]
    fn meter_exporter_failure_preserves_typed_source_at_fallback_and_records_zero_metrics() {
        let injected = build_otel_meter("not a valid endpoint", Resource::builder().build())
            .expect_err("invalid endpoint");
        let mut output = Vec::new();
        let source_seen = std::cell::Cell::new(false);
        let provider = assert_zero_error_metrics(|| {
            setup_otel_meter_with(
                "injected endpoint",
                move |_| Err::<(), _>(injected),
                |error| {
                    assert_eq!(error.to_string(), "failed to build OTLP metric exporter");
                    assert!(
                        error
                            .downcast_ref::<opentelemetry_otlp::ExporterBuildError>()
                            .is_some(),
                        "concrete OTLP exporter error was erased before fallback"
                    );
                    source_seen.set(true);
                    write_exporter_fallback(&mut output, FallbackKind::MeterExporterSetup, error)
                        .expect("write fallback");
                },
            )
        });
        assert!(
            provider.is_none(),
            "startup continues with metrics export disabled"
        );
        assert!(source_seen.get(), "typed source reached fallback seam");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::MeterExporterSetup,
        );
    }

    #[tokio::test]
    async fn build_otel_meter_with_endpoint_is_wired_by_init() {
        with_env(|env| {
            env.set(
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
        });
    }

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint() {
        with_env(|env| {
            env.set(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "not a valid endpoint",
            );
            init_tracing_impl(false);
        });
    }

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint_with_json_output() {
        with_env(|env| {
            env.set(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "still not a valid endpoint",
            );
            env.set("JAUNDER_LOG_FORMAT", "json");
            init_tracing_impl(false);
        });
    }

    #[test]
    fn init_tracing_impl_handles_no_otel_endpoint_with_json_output() {
        with_env(|env| {
            env.remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
            env.remove("OTEL_EXPORTER_OTLP_ENDPOINT");
            env.set("JAUNDER_LOG_FORMAT", "json");
            init_tracing_impl(false);
        });
    }

    #[tokio::test]
    async fn init_tracing_impl_handles_valid_otel_endpoint_with_pretty_output() {
        with_env(|env| {
            env.set(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "http://127.0.0.1:4317",
            );
            env.remove("JAUNDER_LOG_FORMAT");
            init_tracing_impl(false);
        });
    }

    #[tokio::test]
    async fn init_tracing_impl_handles_valid_otel_endpoint_with_json_output() {
        with_env(|env| {
            env.set(
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
                "http://127.0.0.1:4317",
            );
            env.set("JAUNDER_LOG_FORMAT", "json");
            init_tracing_impl(false);
        });
    }

    #[test]
    fn subscriber_install_failure_is_nonfatal_in_subprocess() {
        const CHILD: &str = "JAUNDER_TEST_SUBSCRIBER_INSTALL_CHILD";
        if std::env::var_os(CHILD).is_some() {
            tracing::subscriber::set_global_default(tracing_subscriber::registry())
                .expect("install test subscriber");
            assert_zero_error_metrics(|| {
                let guard = init_tracing_impl(false);
                drop(guard);
            });
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .arg("--exact")
            .arg("telemetry::tests::subscriber_install_failure_is_nonfatal_in_subprocess")
            .arg("--nocapture")
            .env(CHILD, "1")
            .env_remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .output()
            .expect("run isolated subscriber test");
        assert!(output.status.success(), "child status: {}", output.status);
        let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
        assert_fixed_fallback(&stderr, FallbackKind::SubscriberInstall);
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
    fn default_filter_keeps_host_error_reports() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let subscriber = tracing_subscriber::registry()
            .with(default_filter(false))
            .with(
                fmt::layer()
                    .with_ansi(false)
                    .with_writer(Shared(output.clone())),
            );

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: "host::error", "host-warning");
        });

        let output = String::from_utf8(output.lock().expect("trace lock").clone()).expect("utf8");
        assert!(output.contains("host-warning"), "trace: {output}");
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
            meter_shutdown: shutdown_meter,
            tracer_shutdown: shutdown_tracer,
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
            meter_shutdown: shutdown_meter,
            tracer_shutdown: shutdown_tracer,
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
            meter_shutdown: shutdown_meter,
            tracer_shutdown: shutdown_tracer,
        });
    }

    #[tokio::test]
    async fn meter_shutdown_failure_preserves_primary_result_with_one_fallback_and_zero_metrics() {
        let meter = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(InMemoryMetricExporter::default()).build())
            .build();
        let (primary, output) = assert_zero_error_metrics(|| {
            fallback_capture::capture(|| {
                let primary: Result<&str, &str> = Ok("preserved");
                drop(TelemetryGuard {
                    meter: Some(meter),
                    tracer: None,
                    meter_shutdown: |_| {
                        Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                            "injected meter shutdown failure".to_owned(),
                        ))
                    },
                    tracer_shutdown: shutdown_tracer,
                });
                primary
            })
        });
        assert_eq!(primary, Ok("preserved"));
        assert_fixed_fallback(&output, FallbackKind::MeterShutdown);
    }

    #[tokio::test]
    async fn tracer_shutdown_failure_preserves_primary_result_with_one_fallback_and_zero_metrics() {
        let tracer = SdkTracerProvider::builder()
            .with_batch_exporter(InMemorySpanExporter::default())
            .build();
        let (primary, output) = assert_zero_error_metrics(|| {
            fallback_capture::capture(|| {
                let primary: Result<&str, &str> = Ok("preserved");
                drop(TelemetryGuard {
                    meter: None,
                    tracer: Some(tracer),
                    meter_shutdown: shutdown_meter,
                    tracer_shutdown: |_| {
                        Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                            "injected tracer shutdown failure".to_owned(),
                        ))
                    },
                });
                primary
            })
        });
        assert_eq!(primary, Ok("preserved"));
        assert_fixed_fallback(&output, FallbackKind::TracerShutdown);
    }
}
