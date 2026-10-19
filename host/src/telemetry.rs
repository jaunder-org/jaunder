//! Shared host-process telemetry setup and shutdown.
//!
//! This module owns the OpenTelemetry providers used by native binaries that
//! write application state: `jaunder serve`, one-shot `jaunder` CLI commands,
//! and `test-support`. It installs tracing/log/metric layers when an OTLP
//! endpoint is configured, keeps setup failures non-fatal, and returns a
//! [`TelemetryGuard`] whose drop path flushes short-lived process telemetry
//! before exit. Server-only request diagnostics stay in `server::observability`.

mod native_logs;
mod sdk_boundary;
mod sdk_format;
mod span_fields;

pub use sdk_format::SafeSdkFormat;
use std::time::{Duration, Instant};

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    Resource,
    logs::{BatchConfig, BatchLogProcessor, LogExporter, SdkLoggerProvider},
};
use tracing_error::ErrorLayer;
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

const E2E_SEED_PROCESS_ATTR: &str = "jaunder.e2e.seed_process";
const E2E_SEED_PROCESS_JAUNDER: &str = "e2e.seed.jaunder";
const E2E_SEED_PROCESS_TEST_SUPPORT: &str = "e2e.seed.test-support";

/// Raw inherited values collected by an executable composition root.
pub struct TelemetryRawConfig {
    pub log_filter: Result<Option<String>, std::env::VarError>,
    pub rust_log: Result<Option<String>, std::env::VarError>,
    pub log_format: Result<Option<String>, std::env::VarError>,
    pub jaunder_otlp_endpoint: Result<Option<String>, std::env::VarError>,
    pub otlp_endpoint: Result<Option<String>, std::env::VarError>,
    pub slow_op_ms: Result<Option<String>, std::env::VarError>,
    pub e2e_seed_process: Result<Option<String>, std::env::VarError>,
}

/// Immutable telemetry policy resolved before asynchronous work begins.
pub struct TelemetryConfig {
    filter: EnvFilter,
    json_format: bool,
    otlp_endpoint: Option<String>,
    slow_op_threshold: Duration,
    e2e_seed_process: Option<String>,
}

impl TelemetryConfig {
    /// Resolves inherited telemetry values while preserving their established
    /// precedence and malformed-value fallback policy.
    #[must_use]
    pub fn from_raw(verbose: bool, raw: TelemetryRawConfig) -> Self {
        Self {
            filter: resolved_filter(verbose, raw.log_filter, raw.rust_log, || {
                fallback(FallbackKind::LogFilter);
            }),
            json_format: use_json_format(raw.log_format, || fallback(FallbackKind::LogFormat)),
            otlp_endpoint: otel_exporter_otlp_endpoint(
                raw.jaunder_otlp_endpoint,
                raw.otlp_endpoint,
                || fallback(FallbackKind::OtlpEndpoint),
            ),
            slow_op_threshold: slow_op_threshold(raw.slow_op_ms, || {
                fallback(FallbackKind::SlowThreshold);
            }),
            e2e_seed_process: e2e_seed_process(raw.e2e_seed_process),
        }
    }

    /// Local-log admission only; OTLP output must not consume this policy.
    #[must_use]
    pub fn local_log_filter(&self) -> EnvFilter {
        self.filter.clone()
    }

    /// Whether the resolved endpoint enables OTLP-dependent services.
    #[must_use]
    pub fn otlp_endpoint_configured(&self) -> bool {
        self.otlp_endpoint.is_some()
    }
}

#[derive(Clone, Copy)]
enum FallbackKind {
    ExporterRuntimeSetup,
    ExporterRuntimeShutdown,
    TracerExporterSetup,
    MeterExporterSetup,
    MeterShutdown,
    TracerShutdown,
    SubscriberInstall,
    OtlpEndpoint,
    LogFilter,
    SpanFieldFormat,
    LoggerSetup,
    LoggerShutdown,
    SlowThreshold,
    LogFormat,
}

impl FallbackKind {
    fn parts(self) -> (&'static str, &'static str) {
        match self {
            Self::ExporterRuntimeSetup => (
                "host.telemetry.exporter_runtime_setup",
                "telemetry export disabled",
            ),
            Self::ExporterRuntimeShutdown => (
                "host.telemetry.exporter_runtime_shutdown",
                "telemetry shutdown failed",
            ),
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
            Self::SpanFieldFormat => (
                "host.telemetry.span_fields",
                "diagnostic span field formatting failed",
            ),
            Self::LoggerSetup => ("host.telemetry.logger_setup", "native log export disabled"),
            Self::LoggerShutdown => (
                "host.telemetry.logger_shutdown",
                "native log shutdown failed",
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
    #[cfg(test)]
    if fallback_capture::try_capture(kind) {
        return;
    }
    let _ = write_exporter_fallback(std::io::stderr().lock(), kind, error);
}

fn resolved_filter(
    verbose: bool,
    values: Result<Option<String>, std::env::VarError>,
    fallback: Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> EnvFilter {
    for value in [values, fallback] {
        match value {
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

fn use_json_format(
    value: Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> bool {
    match value {
        Ok(Some(value)) if matches!(value.as_str(), "json" | "JSON") => true,
        Ok(Some(value)) if value == "pretty" => false,
        Ok(Some(_)) | Err(_) => {
            warn();
            false
        }
        Ok(None) => false,
    }
}

/// Trim an optional env value and drop it if it is empty (or whitespace-only) —
/// the shared tail of the `JAUNDER_*` readers below, so "blank means unset" stays
/// one rule.
fn trimmed_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn otel_exporter_otlp_endpoint(
    preferred: Result<Option<String>, std::env::VarError>,
    fallback: Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> Option<String> {
    match preferred {
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

    match fallback {
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

fn e2e_seed_process(value: Result<Option<String>, std::env::VarError>) -> Option<String> {
    let value = trimmed_non_empty(value.ok()?)?;
    matches!(
        value.as_str(),
        E2E_SEED_PROCESS_JAUNDER | E2E_SEED_PROCESS_TEST_SUPPORT
    )
    .then_some(value)
}

fn telemetry_resource(seed_process: Option<String>) -> Resource {
    let builder = Resource::builder();
    if let Some(value) = seed_process {
        builder
            .with_attribute(KeyValue::new(E2E_SEED_PROCESS_ATTR, value))
            .build()
    } else {
        builder.build()
    }
}

pub(crate) fn tracer_provider_builder() -> opentelemetry_sdk::trace::TracerProviderBuilder {
    opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
}

fn build_otel_tracer(
    endpoint: &str,
    resource: Resource,
) -> anyhow::Result<opentelemetry_sdk::trace::SdkTracerProvider> {
    let exporter = anyhow::Context::context(
        opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build(),
        "failed to build OTLP span exporter",
    )?;
    let provider = tracer_provider_builder()
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
    let exporter = anyhow::Context::context(
        opentelemetry_otlp::MetricExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build(),
        "failed to build OTLP metric exporter",
    )?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_resource(resource)
        .with_periodic_exporter(exporter)
        .build();
    opentelemetry::global::set_meter_provider(provider.clone());
    Ok(provider)
}

// Resolve inherited batch settings once at process initialization; the stock
// processor builder receives that complete snapshot.
struct BatchPolicy(BatchConfig);

impl BatchPolicy {
    fn resolve() -> Self {
        Self(BatchConfig::default())
    }
}
fn logger_provider_with<E: LogExporter + 'static>(
    resource: Resource,
    policy: BatchPolicy,
    exporter: E,
) -> SdkLoggerProvider {
    let processor = BatchLogProcessor::builder(exporter)
        .with_batch_config(policy.0)
        .build();
    SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_log_processor(processor)
        .build()
}
fn build_otel_logger(
    endpoint: &str,
    resource: Resource,
    policy: BatchPolicy,
) -> anyhow::Result<SdkLoggerProvider> {
    let exporter = anyhow::Context::context(
        opentelemetry_otlp::LogExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build(),
        "failed to build OTLP log exporter",
    )?;
    Ok(logger_provider_with(resource, policy, exporter))
}
fn shutdown_logger(provider: &SdkLoggerProvider) {
    // Synchronous SDK teardown must not feed the providers being drained.
    let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
    if provider.shutdown().is_err() {
        fallback(FallbackKind::LoggerShutdown);
    }
}

fn setup_provider_with<T>(
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

fn slow_op_threshold(
    value: Result<Option<String>, std::env::VarError>,
    mut warn: impl FnMut(),
) -> Duration {
    match value {
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

fn install_subscriber_with<E>(
    install: impl FnOnce() -> Result<(), E>,
    mut warn: impl FnMut(),
) -> bool {
    if install().is_ok() {
        true
    } else {
        warn();
        false
    }
}

// Context observers must not enable ordinary events without an OTLP tracer
// when local output is filtered or absent.
fn span_metadata_only(metadata: &tracing::Metadata<'_>) -> bool {
    metadata.is_span()
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn compose_subscriber<L, W>(
    config: &TelemetryConfig,
    extra_layer: Option<L>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracer>,
    output: Option<W>,
) -> impl tracing::Subscriber + Send + Sync + 'static
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
    W: for<'writer> fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    compose_subscriber_with_logger(config, extra_layer, tracer, output, None)
}

fn compose_subscriber_with_logger<L, W>(
    config: &TelemetryConfig,
    extra_layer: Option<L>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracer>,
    output: Option<W>,
    logger: Option<&opentelemetry_sdk::logs::SdkLoggerProvider>,
) -> impl tracing::Subscriber + Send + Sync + 'static
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
    W: for<'writer> fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    // An absent Option<Layer> advertises Interest::always. Use a filtered
    // identity for absent sinks, leaving each present sink's own filter intact.
    let extra_layer = extra_layer.map_or_else(
        || {
            tracing_subscriber::layer::Identity::default()
                .with_filter(tracing::level_filters::LevelFilter::OFF)
                .boxed()
        },
        Layer::boxed,
    );
    let activation = sdk_boundary::Activation::new();
    let native_enabled = logger.is_some();
    let native_logs = native_logs::EventContextBridge::new(logger)
        // Absence disables only this sink, before event fields are evaluated.
        // Filtered still forwards registration to the owning context adapter.
        .with_filter(tracing_subscriber::filter::filter_fn(move |_| {
            native_enabled
        }));
    // Endpoint selection owns routing even if native setup failed. Restoring
    // ordinary stdout would silently create a second ingestion path.
    let fmt_layer = output
        .filter(|_| !config.otlp_endpoint_configured())
        .map_or_else(
            || {
                tracing_subscriber::layer::Identity::default()
                    .with_filter(tracing::level_filters::LevelFilter::OFF)
                    .boxed()
            },
            |writer| {
                if config.json_format {
                    fmt::layer()
                        .json()
                        .event_format(SafeSdkFormat::new(fmt::format().json(), true))
                        .with_writer(writer)
                        .with_filter(config.local_log_filter())
                        .boxed()
                } else {
                    fmt::layer()
                        .event_format(SafeSdkFormat::new(fmt::format(), false))
                        .with_writer(writer)
                        .with_filter(config.local_log_filter())
                        .boxed()
                }
            },
        );
    let otel_layer = tracer.map_or_else(
        || {
            tracing_subscriber::layer::Identity::default()
                .with_filter(tracing::level_filters::LevelFilter::OFF)
                .boxed()
        },
        |tracer| {
            tracing_opentelemetry::layer()
                .with_tracer(tracer)
                .with_context_activation(false)
                // SDK-owned suppression is fixed isolation, never a local verbosity policy.
                .with_filter(tracing_subscriber::filter::dynamic_filter_fn(|_, _| {
                    !opentelemetry::Context::is_current_telemetry_suppressed()
                }))
                .boxed()
        },
    );
    tracing_subscriber::registry()
        .with(extra_layer)
        .with(
            SlowSpanLayer::new(config.slow_op_threshold)
                .with_filter(tracing_subscriber::filter::filter_fn(span_metadata_only)),
        )
        .with(
            span_fields::SpanFieldUpdates
                .and_then(ErrorLayer::new(span_fields::SpanTraceFields))
                .with_filter(tracing_subscriber::filter::filter_fn(span_metadata_only)),
        )
        .with(fmt_layer)
        .with(otel_layer)
        .with(native_logs)
        // Both context layers are registered directly: Option<Layer> does not
        // forward dispatch registration to its inner layer.
        .with(activation.with_filter(tracing_subscriber::filter::filter_fn(span_metadata_only)))
}

fn setup_exporter_runtime_with<T>(
    endpoint: Option<&str>,
    build: impl FnOnce() -> std::io::Result<T>,
) -> Option<T> {
    endpoint.and_then(|_| match build() {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            exporter_fallback(FallbackKind::ExporterRuntimeSetup, &error.into());
            None
        }
    })
}

fn init_tracing_impl_with_layer<L>(
    config: &TelemetryConfig,
    extra_layer: Option<L>,
) -> TelemetryGuard
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    // Resolve the owned SDK policy at the process initialization boundary,
    // before exporter/provider work. The pinned builder's constructor-only
    // default read is discarded by the complete snapshot override.
    let log_batch_policy = config.otlp_endpoint_configured().then(BatchPolicy::resolve);
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

    // Traces and metrics share the endpoint resolved at the process root.
    let endpoint = config.otlp_endpoint.clone();
    let resource = telemetry_resource(config.e2e_seed_process.clone());

    let exporter_runtime =
        setup_exporter_runtime_with(endpoint.as_deref(), sdk_boundary::ExporterRuntime::new);
    let tracer =
        endpoint
            .as_deref()
            .zip(exporter_runtime.as_ref())
            .and_then(|(endpoint, runtime)| {
                setup_provider_with(
                    endpoint,
                    |endpoint| runtime.in_scope(|| build_otel_tracer(endpoint, resource.clone())),
                    |error| {
                        exporter_fallback(FallbackKind::TracerExporterSetup, error);
                    },
                )
            });
    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    let meter =
        endpoint
            .as_deref()
            .zip(exporter_runtime.as_ref())
            .and_then(|(endpoint, runtime)| {
                setup_provider_with(
                    endpoint,
                    |endpoint| runtime.in_scope(|| build_otel_meter(endpoint, resource.clone())),
                    |error| {
                        exporter_fallback(FallbackKind::MeterExporterSetup, error);
                    },
                )
            });

    let logger = endpoint
        .as_deref()
        .zip(exporter_runtime.as_ref())
        .zip(log_batch_policy)
        .and_then(|((endpoint, runtime), policy)| {
            setup_provider_with(
                endpoint,
                |endpoint| runtime.in_scope(|| build_otel_logger(endpoint, resource, policy)),
                |error| exporter_fallback(FallbackKind::LoggerSetup, error),
            )
        });

    let subscriber_installed = install_subscriber_with(
        || {
            tracing::subscriber::set_global_default(compose_subscriber_with_logger(
                config,
                extra_layer,
                tracer.as_ref().map(|provider| provider.tracer("jaunder")),
                Some(std::io::stdout),
                logger.as_ref(),
            ))
        },
        || fallback(FallbackKind::SubscriberInstall),
    );

    TelemetryGuard::new(
        exporter_runtime,
        meter,
        tracer,
        logger,
        subscriber_installed,
    )
}

fn init_tracing_impl(config: &TelemetryConfig) -> TelemetryGuard {
    init_tracing_impl_with_layer::<tracing_subscriber::layer::Identity>(config, None)
}

/// Install the process-wide tracing/logging/metrics subscriber and return its
/// shutdown guard.
///
/// The setup is intentionally best-effort: exporter construction failures and
/// duplicate subscriber installs are recorded through local diagnostics but do
/// not prevent the command from running.
#[must_use]
pub fn init_tracing(config: &TelemetryConfig) -> TelemetryGuard {
    init_tracing_impl(config)
}

/// Install tracing with a caller-owned extra subscriber layer and return the
/// same shutdown guard as [`init_tracing`].
#[must_use]
pub fn init_tracing_with_layer<L>(config: &TelemetryConfig, layer: Option<L>) -> TelemetryGuard
where
    L: Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
{
    init_tracing_impl_with_layer(config, layer)
}

type MeterShutdownOperation =
    fn(&opentelemetry_sdk::metrics::SdkMeterProvider) -> opentelemetry_sdk::error::OTelSdkResult;
type TracerShutdownOperation =
    fn(&opentelemetry_sdk::trace::SdkTracerProvider) -> opentelemetry_sdk::error::OTelSdkResult;

/// Owns every installed signal provider across process or command execution.
/// SDK shutdown drains buffered telemetry using each provider's existing bounded
/// budget; no unbounded pre-shutdown flush is added. Cleanup runs on ordinary
/// return, error return and panic unwind without changing the primary outcome.
/// Logger shutdown acknowledgment alone is not export/delivery proof: actual
/// export failures and partial rejection remain stock SDK diagnostics. The
/// owning local formatters withhold their remote detail.
///
/// Providers are absent without configured OTLP. The private transport runtime
/// outlives all provider shutdown attempts and retains its fixed best-effort
/// five-second cleanup policy, not a guarantee that residual blocking work ends.
pub struct TelemetryGuard {
    exporter_runtime: Option<sdk_boundary::ExporterRuntime>,
    meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
    logger: Option<opentelemetry_sdk::logs::SdkLoggerProvider>,
    subscriber_installed: bool,
    meter_shutdown: MeterShutdownOperation,
    tracer_shutdown: TracerShutdownOperation,
}

impl TelemetryGuard {
    fn new(
        exporter_runtime: Option<sdk_boundary::ExporterRuntime>,
        meter: Option<opentelemetry_sdk::metrics::SdkMeterProvider>,
        tracer: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
        logger: Option<SdkLoggerProvider>,
        subscriber_installed: bool,
    ) -> Self {
        Self {
            exporter_runtime,
            meter,
            tracer,
            logger,
            subscriber_installed,
            meter_shutdown: shutdown_meter,
            tracer_shutdown: shutdown_tracer,
        }
    }

    /// Whether initialization installed an OpenTelemetry tracing provider and
    /// its subscriber layer.
    #[must_use]
    pub fn otel_tracing_enabled(&self) -> bool {
        self.subscriber_installed && self.tracer.is_some()
    }
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

#[cfg(test)]
fn default_telemetry_config() -> TelemetryConfig {
    TelemetryConfig::from_raw(
        false,
        TelemetryRawConfig {
            log_filter: Ok(None),
            rust_log: Ok(None),
            log_format: Ok(None),
            jaunder_otlp_endpoint: Ok(None),
            otlp_endpoint: Ok(None),
            slow_op_ms: Ok(None),
            e2e_seed_process: Ok(None),
        },
    )
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
        if let Some(logger) = self.logger.as_ref() {
            shutdown_logger(logger);
        }
        // Every installed provider can use transport until shutdown finishes.
        drop(self.exporter_runtime.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::{
        Value,
        logs::{AnyValue, LogRecord as _, Logger as _, LoggerProvider as _},
        trace::{SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState},
    };
    use opentelemetry_proto::tonic::collector::logs::v1::{
        ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    };
    use opentelemetry_sdk::trace::{
        InMemorySpanExporter, SdkTracerProvider, SpanData, SpanExporter,
    };
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        logs::{LogBatch, SdkLogRecord},
        metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider},
    };
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    /// Keeps exported spans available after processor shutdown. The SDK's
    /// in-memory exporter intentionally clears its records during shutdown.
    #[derive(Clone, Debug, Default)]
    struct RetainingSpanExporter(InMemorySpanExporter);

    impl SpanExporter for RetainingSpanExporter {
        async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
            self.0.export(batch).await
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
    fn native_logger_guard_shutdown_drains_short_lived_records() {
        use opentelemetry_sdk::logs::{InMemoryLogExporter, LogBatch, LogExporter};
        #[derive(Clone, Debug)]
        struct Retained(InMemoryLogExporter);
        impl LogExporter for Retained {
            async fn export(&self, batch: LogBatch<'_>) -> opentelemetry_sdk::error::OTelSdkResult {
                self.0.export(batch).await
            }
        }
        let exporter = InMemoryLogExporter::default();
        let logger = logger_provider_with(
            Resource::builder().build(),
            BatchPolicy::resolve(),
            Retained(exporter.clone()),
        );
        let mut config = configured_local_policy(false, Some("off"), None);
        config.otlp_endpoint = Some("http://127.0.0.1:4317".to_owned());
        let subscriber = compose_subscriber_with_logger(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<Shared>,
            Some(&logger),
        );
        tracing::subscriber::with_default(
            subscriber,
            || tracing::info!(target: "host::application", "one-shot completion"),
        );
        drop(TelemetryGuard::new(None, None, None, Some(logger), true));
        assert_eq!(exporter.get_emitted_logs().expect("drained logs").len(), 1);
    }

    #[test]
    fn native_sink_configured_endpoint_never_mirrors_or_restores_stdout() {
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
        for installed in [true, false] {
            let mut config = configured_local_policy(false, Some("trace"), None);
            config.otlp_endpoint = Some("http://127.0.0.1:4317".to_owned());
            let output = Arc::new(Mutex::new(Vec::new()));
            let exporter = InMemoryLogExporter::default();
            let provider = SdkLoggerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let subscriber = compose_subscriber_with_logger(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                None,
                Some(Shared(output.clone())),
                installed.then_some(&provider),
            );
            tracing::subscriber::with_default(
                subscriber,
                || tracing::info!(target: "host::application", "ordinary native message"),
            );
            assert!(
                output.lock().expect("stdout").is_empty(),
                "configured endpoint must never restore stdout even when logger setup fails"
            );
            assert_eq!(
                exporter.get_emitted_logs().expect("logs").len(),
                usize::from(installed)
            );
        }
    }

    #[test]
    fn native_projection_exports_exact_message_and_actual_severity() {
        use opentelemetry::logs::{AnyValue, Severity};
        use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = compose_subscriber_with_logger(
            &configured_local_policy(false, Some("off"), None),
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<Shared>,
            Some(&provider),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "host::application", "finished {}", "operation");
        });
        let records = exporter.get_emitted_logs().expect("logs");
        assert_eq!(records.len(), 1);
        let record = &records[0].record;
        assert_eq!(record.body(), Some(&AnyValue::from("finished operation")));
        assert_eq!(record.severity_number(), Some(Severity::Info));
        assert_eq!(record.severity_text(), Some("INFO"));
    }

    #[test]
    fn local_controls_do_not_filter_otlp_debug_trace_spans_or_events() {
        for (verbose, log, logged) in [
            (false, Some("warn"), false),
            (true, Some("warn"), false),
            (false, Some("off"), false),
            (true, None, true),
            (false, Some("trace"), true),
            (false, Some("host=warn,[context{enabled=true}]=off"), false),
        ] {
            let config = configured_local_policy(verbose, log, None);
            let output = Arc::new(Mutex::new(Vec::new()));
            let exporter = InMemorySpanExporter::default();
            let provider = tracer_provider_builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let subscriber = compose_subscriber(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                Some(provider.tracer("test")),
                Some(Shared(output.clone())),
            );
            tracing::subscriber::with_default(subscriber, || {
                let context = tracing::debug_span!("context", enabled = true);
                context.in_scope(|| {
                    let request = tracing::info_span!("request");
                    request.in_scope(|| {
                        let debug = tracing::debug_span!("debug_operation");
                        debug.in_scope(|| tracing::trace!("trace operation detail"));
                        let trace = tracing::trace_span!("trace_operation");
                        trace.in_scope(|| tracing::debug!("debug operation detail"));
                        tracing::info!("ordinary info");
                    });
                });
            });
            provider.force_flush().expect("flush");
            let spans = exporter.get_finished_spans().expect("spans");
            for name in ["context", "request", "debug_operation", "trace_operation"] {
                assert!(
                    spans.iter().any(|span| span.name == name),
                    "{verbose}/{log:?}: missing {name}"
                );
            }
            let events: Vec<_> = spans
                .iter()
                .flat_map(|span| span.events.events.iter())
                .collect();
            assert!(
                events
                    .iter()
                    .any(|event| event.name == "trace operation detail"),
                "{verbose}/{log:?}"
            );
            assert!(
                events
                    .iter()
                    .any(|event| event.name == "debug operation detail"),
                "{verbose}/{log:?}"
            );
            assert_eq!(
                String::from_utf8(output.lock().expect("output").clone())
                    .expect("utf8")
                    .contains("ordinary info"),
                logged,
                "{verbose}/{log:?}"
            );
        }
    }

    fn span_names(spans: &[SpanData]) -> Vec<&str> {
        spans.iter().map(|span| span.name.as_ref()).collect()
    }

    #[test]
    fn span_names_preserve_exported_operation_order() {
        use opentelemetry::trace::Tracer as _;
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("names");
        drop(tracer.start("first_operation"));
        drop(tracer.start("second_operation"));
        provider.force_flush().expect("flush");
        assert_eq!(
            span_names(&exporter.get_finished_spans().expect("spans")),
            ["first_operation", "second_operation"]
        );
    }

    #[test]
    fn sdk_suppressed_export_work_does_not_generate_otlp_spans() {
        use opentelemetry::trace::Tracer as _;
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("suppression-proof");
        let subscriber = compose_subscriber(
            &configured_local_policy(false, Some("off"), None),
            None::<tracing_subscriber::layer::Identity>,
            Some(tracer.clone()),
            None::<Shared>,
        );
        tracing::subscriber::with_default(subscriber, || {
            let _sdk_scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
            drop(tracer.start("direct_sdk_control"));
            let export_work = tracing::trace_span!(parent: None, "export_transport");
            drop(export_work);
        });
        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("spans");
        let names = span_names(&spans);
        assert!(
            names.is_empty(),
            "SDK suppression must also cover tracing's explicit-root transport work: {names:?}"
        );
    }

    #[test]
    fn sdk_boundary_span_and_event_admission_is_dynamic_and_birth_scoped() {
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        fn same_callsite() -> tracing::Span {
            tracing::trace_span!(parent: None, "same_callsite")
        }
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = compose_subscriber(
            &configured_local_policy(false, Some("trace"), None),
            None::<tracing_subscriber::layer::Identity>,
            Some(provider.tracer("boundary")),
            Some(Shared(Arc::new(Mutex::new(Vec::new())))),
        );
        tracing::subscriber::with_default(subscriber, || {
            let app = tracing::info_span!("application");
            let late = {
                let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
                let root = same_callsite();
                let explicit = tracing::trace_span!(parent: &app, "sdk_explicit");
                app.in_scope(|| {
                    let contextual = tracing::trace_span!("sdk_contextual");
                    tracing::error!("sdk_contextual_event");
                    drop(contextual);
                });
                tracing::error!(parent: &app, "sdk_explicit_event");
                drop(explicit);
                root
            };
            // A filtered birth cannot be resurrected by a late parent or close.
            // The public extension returns Ok even when no bridge data exists;
            // exported absence below, not its return value, proves no resurrection.
            let _result = late.set_parent(opentelemetry::Context::new());
            drop(late);
            drop(same_callsite());
            app.in_scope(|| tracing::info!("application_event"));
            // Closing an admitted application span inside SDK work must still export it.
            let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
            drop(app);
        });
        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("spans");
        let names: Vec<_> = spans.iter().map(|s| s.name.as_ref()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"application"));
        assert!(names.contains(&"same_callsite"));
        let app = spans.iter().find(|s| s.name == "application").expect("app");
        assert_eq!(app.events.events.len(), 1);
        assert_eq!(app.events.events[0].name, "application_event");
        assert_eq!(app.status, opentelemetry::trace::Status::Unset);
    }

    #[test]
    fn sdk_boundary_activation_preserves_suppression_and_direct_sdk_parent() {
        use opentelemetry::trace::{TraceContextExt as _, Tracer as _};
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let tracer = provider.tracer("activation");
        let subscriber = compose_subscriber(
            &configured_local_policy(false, Some("off"), None),
            None::<tracing_subscriber::layer::Identity>,
            Some(tracer.clone()),
            None::<Shared>,
        );
        let original = opentelemetry::Context::current()
            .span()
            .span_context()
            .clone();
        tracing::subscriber::with_default(subscriber, || {
            let app = tracing::info_span!("application");
            let first = app.enter();
            let parent = opentelemetry::Context::current()
                .span()
                .span_context()
                .clone();
            assert!(parent.is_valid());
            drop(tracer.start("direct_child"));
            let repeated = app.enter();
            // Equivalent contexts cannot distinguish repeated entry identities;
            // interleaved external-context behavior has separate characterization.
            drop(first);
            assert_eq!(
                opentelemetry::Context::current().span().span_context(),
                &parent
            );
            drop(repeated);
            assert_eq!(
                opentelemetry::Context::current().span().span_context(),
                &original
            );
            {
                let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
                app.in_scope(|| {
                    assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                    drop(tracer.start("suppressed_direct_child"));
                    drop(tracing::trace_span!(parent: None, "suppressed_root"));
                    let nested = tracing::trace_span!("suppressed_nested");
                    nested.in_scope(|| {
                        assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                    });
                });
                assert!(opentelemetry::Context::is_current_telemetry_suppressed());
            }
            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _entered = app.enter();
                let nested = tracing::info_span!("nested_application");
                let _nested = nested.enter();
                panic!("activation unwind proof");
            }));
            assert!(panic.is_err());
            assert!(!opentelemetry::Context::is_current_telemetry_suppressed());
            assert_eq!(
                opentelemetry::Context::current().span().span_context(),
                &original
            );
        });
        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("spans");
        let app = spans.iter().find(|s| s.name == "application").expect("app");
        let child = spans
            .iter()
            .find(|s| s.name == "direct_child")
            .expect("child");
        assert_eq!(child.parent_span_id, app.span_context.span_id());
        assert_eq!(child.span_context.trace_id(), app.span_context.trace_id());
        assert_eq!(spans.len(), 3, "{spans:?}");
    }

    #[test]
    fn sdk_boundary_repeated_entry_known_out_of_order_external_context_limitation() {
        assert_repeated_entry_interleaving(false, false, false);
    }

    #[test]
    fn sdk_boundary_repeated_entry_known_out_of_order_suppression_limitation() {
        assert_repeated_entry_interleaving(true, false, false);
    }

    #[test]
    fn sdk_boundary_repeated_entry_lifo_restores_distinguishable_external_context() {
        assert_repeated_entry_interleaving(false, true, false);
    }

    #[test]
    fn sdk_boundary_repeated_entry_lifo_restores_interleaved_suppression() {
        assert_repeated_entry_interleaving(true, true, false);
    }

    #[test]
    fn sdk_boundary_repeated_entry_pinned_bridge_has_same_ordinary_limitation() {
        assert_repeated_entry_interleaving(false, false, true);
    }

    fn assert_repeated_entry_interleaving(suppressed: bool, lifo: bool, pinned_bridge: bool) {
        use opentelemetry::trace::TraceContextExt as _;
        #[derive(Debug, PartialEq)]
        struct ExternalMarker;
        {
            let provider = tracer_provider_builder()
                .with_simple_exporter(InMemorySpanExporter::default())
                .build();
            let dispatch = if pinned_bridge {
                tracing::Dispatch::new(tracing_subscriber::registry().with(
                    tracing_opentelemetry::layer().with_tracer(provider.tracer("pinned-bridge")),
                ))
            } else {
                tracing::Dispatch::new(compose_subscriber(
                    &configured_local_policy(false, Some("off"), None),
                    None::<tracing_subscriber::layer::Identity>,
                    Some(provider.tracer("interleaved-entry")),
                    None::<Shared>,
                ))
            };
            tracing::dispatcher::with_default(&dispatch, || {
                let original = opentelemetry::Context::current();
                let app = tracing::info_span!("interleaved_application");
                let first = app.enter();
                let app_context = opentelemetry::Context::current()
                    .span()
                    .span_context()
                    .clone();
                let external = opentelemetry::Context::new().with_value(ExternalMarker);
                let external = if suppressed {
                    external.with_telemetry_suppressed()
                } else {
                    external
                };
                let external_scope = external.attach();
                let second = app.enter();
                let before_exit = opentelemetry::Context::current();
                let remaining = if lifo {
                    drop(second);
                    first
                } else {
                    // Known limitation: callback IDs cannot tell this exit from
                    // the second entry's exit. It pops the second activation.
                    drop(first);
                    second
                };
                let after_entry_exit = opentelemetry::Context::current();
                drop(external_scope);
                let after_external_exit = opentelemetry::Context::current();
                drop(remaining);
                let restored = opentelemetry::Context::current();
                assert_eq!(before_exit.span().span_context(), &app_context);
                assert_eq!(before_exit.is_telemetry_suppressed(), suppressed);
                assert!(!after_entry_exit.span().span_context().is_valid());
                assert_eq!(
                    after_entry_exit.get::<ExternalMarker>(),
                    Some(&ExternalMarker)
                );
                assert_eq!(after_entry_exit.is_telemetry_suppressed(), suppressed);
                assert_eq!(after_external_exit.span().span_context(), &app_context);
                assert!(after_external_exit.get::<ExternalMarker>().is_none());
                // Correct first-entry restoration for LIFO; known early exposure
                // of that unsuppressed activation when the second entry is alive.
                assert!(!after_external_exit.is_telemetry_suppressed());
                assert_eq!(
                    restored.span().span_context(),
                    original.span().span_context()
                );
                assert_eq!(
                    restored.is_telemetry_suppressed(),
                    original.is_telemetry_suppressed()
                );
            });
        }
    }

    #[test]
    fn sdk_boundary_runtime_worker_cannot_clear_suppression_by_entering_application_span() {
        let runtime = sdk_boundary::ExporterRuntime::new().expect("runtime");
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = compose_subscriber(
            &configured_local_policy(false, Some("off"), None),
            None::<tracing_subscriber::layer::Identity>,
            Some(provider.tracer("runtime")),
            None::<Shared>,
        );
        let (done, received) = std::sync::mpsc::channel();
        tracing::subscriber::with_default(subscriber, || {
            let app = tracing::info_span!("runtime_application");
            runtime.in_scope(|| {
                tokio::spawn(async move {
                    app.in_scope(|| {
                        assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                        drop(tracing::trace_span!(parent: None, "sdk_transport_root"));
                        tracing::error!(parent: &app, "sdk_transport_event");
                    });
                    tokio::task::yield_now().await;
                    assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                    drop(app);
                    done.send(()).expect("done");
                });
            });
            received
                .recv_timeout(Duration::from_secs(5))
                .expect("worker completed");
            assert!(!opentelemetry::Context::is_current_telemetry_suppressed());
        });
        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("spans");
        assert_eq!(spans.len(), 1, "{spans:?}");
        assert_eq!(spans[0].name, "runtime_application");
        assert!(spans[0].events.events.is_empty());
        drop(runtime);
    }

    #[derive(Clone, Debug)]
    struct Probe {
        handle: tokio::runtime::Handle,
        output: std::sync::mpsc::Sender<&'static str>,
    }
    impl Probe {
        async fn transport(&self, signal: &'static str) {
            let output = self.output.clone();
            self.handle
                .spawn(async move {
                    tokio::task::yield_now().await;
                    assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                    output.send(signal).expect("export signal");
                })
                .await
                .expect("export transport");
        }
    }
    impl SpanExporter for Probe {
        async fn export(&self, batch: Vec<SpanData>) -> opentelemetry_sdk::error::OTelSdkResult {
            assert!(batch.iter().any(|span| span.name == "shutdown_span"));
            self.transport("trace").await;
            Ok(())
        }
    }
    impl opentelemetry_sdk::metrics::exporter::PushMetricExporter for Probe {
        async fn export(
            &self,
            metrics: &opentelemetry_sdk::metrics::data::ResourceMetrics,
        ) -> opentelemetry_sdk::error::OTelSdkResult {
            assert!(
                metrics
                    .scope_metrics()
                    .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
                    .any(|metric| metric.name() == "shutdown.counter")
            );
            self.transport("metric").await;
            Ok(())
        }
        fn force_flush(&self) -> opentelemetry_sdk::error::OTelSdkResult {
            Ok(())
        }
        fn shutdown_with_timeout(
            &self,
            _timeout: Duration,
        ) -> opentelemetry_sdk::error::OTelSdkResult {
            Ok(())
        }
        fn temporality(&self) -> opentelemetry_sdk::metrics::Temporality {
            opentelemetry_sdk::metrics::Temporality::Cumulative
        }
    }
    impl opentelemetry_sdk::logs::LogExporter for Probe {
        async fn export(
            &self,
            batch: opentelemetry_sdk::logs::LogBatch<'_>,
        ) -> opentelemetry_sdk::error::OTelSdkResult {
            assert!(batch.iter().any(|(record, _)| record.body()
                == Some(&opentelemetry::logs::AnyValue::from("shutdown_log"))));
            self.transport("log").await;
            Ok(())
        }
    }
    #[tokio::test]
    async fn sdk_boundary_guard_keeps_transport_alive_through_all_provider_shutdowns() {
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry::trace::Tracer as _;
        use opentelemetry_sdk::metrics::exporter::PushMetricExporter;
        let log_policy = BatchPolicy::resolve();
        let runtime = sdk_boundary::ExporterRuntime::new().expect("runtime");
        let (output, received) = std::sync::mpsc::channel();
        let probe = runtime.in_scope(|| Probe {
            handle: tokio::runtime::Handle::current(),
            output,
        });
        PushMetricExporter::force_flush(&probe).expect("metric exporter flush");
        let tracer = tracer_provider_builder()
            .with_batch_exporter(probe.clone())
            .build();
        let logger = logger_provider_with(Resource::builder().build(), log_policy, probe.clone());
        {
            use opentelemetry::logs::{LogRecord as _, Logger as _, LoggerProvider as _};
            let log = logger.logger("shutdown");
            let mut record = log.create_log_record();
            record.set_body("shutdown_log".into());
            log.emit(record);
        }
        let meter = SdkMeterProvider::builder()
            .with_periodic_exporter(probe)
            .build();
        drop(tracer.tracer("shutdown").start("shutdown_span"));
        meter
            .meter("shutdown")
            .u64_counter("shutdown.counter")
            .build()
            .add(1, &[]);
        drop(TelemetryGuard::new(
            Some(runtime),
            Some(meter),
            Some(tracer),
            Some(logger),
            true,
        ));
        let signals: Vec<_> = received.try_iter().collect();
        assert_eq!(signals, ["metric", "trace", "log"]);
        assert!(!opentelemetry::Context::is_current_telemetry_suppressed());
    }

    #[test]
    fn sdk_boundary_activation_is_subscriber_qualified() {
        use opentelemetry::trace::TraceContextExt as _;
        let provider = tracer_provider_builder()
            .with_simple_exporter(InMemorySpanExporter::default())
            .build();
        let config = configured_local_policy(false, Some("off"), None);
        let dispatch = || {
            tracing::Dispatch::new(compose_subscriber(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                Some(provider.tracer("dispatch")),
                None::<Shared>,
            ))
        };
        let a = dispatch();
        let b = dispatch();
        let original = opentelemetry::Context::current()
            .span()
            .span_context()
            .clone();
        tracing::dispatcher::with_default(&a, || {
            let app_a = tracing::info_span!("app_a");
            let _entered = app_a.enter();
            let a_context = opentelemetry::Context::current()
                .span()
                .span_context()
                .clone();
            tracing::dispatcher::with_default(&b, || {
                let app_b = tracing::info_span!("app_b");
                app_b.in_scope(|| {
                    assert_ne!(
                        opentelemetry::Context::current()
                            .span()
                            .span_context()
                            .span_id(),
                        a_context.span_id()
                    );
                });
                assert_eq!(
                    opentelemetry::Context::current().span().span_context(),
                    &a_context
                );
            });
        });
        assert_eq!(
            opentelemetry::Context::current().span().span_context(),
            &original
        );
    }

    #[test]
    fn production_provider_records_unsampled_parent_with_unchanged_ids() {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
        };
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let config = configured_local_policy(false, Some("off"), None);
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            Some(provider.tracer("test")),
            None::<Shared>,
        );
        let trace_id = TraceId::from_hex("11111111111111111111111111111111").expect("trace ID");
        let parent_id = SpanId::from_hex("2222222222222222").expect("parent ID");
        let parent = opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            trace_id,
            parent_id,
            TraceFlags::NOT_SAMPLED,
            true,
            TraceState::default(),
        ));
        tracing::subscriber::with_default(subscriber, || {
            let request = tracing::info_span!("request");
            request.set_parent(parent).expect("parent before SDK start");
            request.in_scope(|| {
                let child = tracing::info_span!("operation");
                drop(child);
            });
        });
        provider.force_flush().expect("flush");
        let spans = exporter.get_finished_spans().expect("spans");
        let request = spans
            .iter()
            .find(|span| span.name == "request")
            .expect("record unsampled parent's request");
        let operation = spans
            .iter()
            .find(|span| span.name == "operation")
            .expect("record unsampled parent's operation");
        assert_eq!(request.span_context.trace_id(), trace_id);
        assert_eq!(request.parent_span_id, parent_id);
        assert!(request.span_context.is_sampled());
        assert_eq!(operation.span_context.trace_id(), trace_id);
        assert_eq!(operation.parent_span_id, request.span_context.span_id());
    }

    #[test]
    fn independent_sink_admission_exports_info_requests_with_quiet_logs() {
        let config = configured_local_policy(false, Some("warn"), None);
        let output = Arc::new(Mutex::new(Vec::new()));
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            Some(provider.tracer("test")),
            Some(Shared(output.clone())),
        );
        tracing::subscriber::with_default(subscriber, || {
            let request = tracing::info_span!(target: "jaunder::observability::http", "request");
            tracing_opentelemetry::OpenTelemetrySpanExt::set_parent(
                &request,
                opentelemetry::Context::new(),
            )
            .expect("assign parent before SDK context starts");
            request.in_scope(|| {
                tracing::info!("ordinary info");
                tracing::warn!("ordinary warning");
            });
        });
        provider.force_flush().expect("flush");
        assert!(
            exporter
                .get_finished_spans()
                .expect("spans")
                .iter()
                .any(|span| span.name == "request")
        );
        let output = String::from_utf8(output.lock().expect("output").clone()).expect("utf8");
        assert!(output.contains("ordinary warning"));
        assert!(!output.contains("ordinary info"));
    }

    fn evaluate_field(evaluated: &std::cell::Cell<u32>) -> u32 {
        evaluated.set(evaluated.get() + 1);
        evaluated.get()
    }

    #[test]
    fn absent_native_provider_evaluates_admitted_event_fields() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut config = configured_local_policy(false, Some("trace"), None);
        config.json_format = true;
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            Some(Shared(output.clone())),
        );
        let evaluated = std::cell::Cell::new(0_u32);
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(evaluated = evaluate_field(&evaluated), "admitted debug");
            tracing::trace!(evaluated = evaluate_field(&evaluated), "admitted trace");
        });
        assert_eq!(evaluated.get(), 2);
        let output = String::from_utf8(output.lock().expect("output").clone()).expect("utf8");
        let records: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).expect("JSON event"))
            .collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["fields"]["evaluated"], 1);
        assert_eq!(records[0]["fields"]["message"], "admitted debug");
        assert_eq!(records[1]["fields"]["evaluated"], 2);
        assert_eq!(records[1]["fields"]["message"], "admitted trace");
    }

    #[test]
    fn absent_native_provider_does_not_evaluate_filtered_event_fields_but_keeps_span_trace() {
        for policy in ["off", "warn"] {
            for output_enabled in [false, true] {
                let output = Arc::new(Mutex::new(Vec::new()));
                let config = configured_local_policy(false, Some(policy), None);
                let subscriber = compose_subscriber(
                    &config,
                    None::<tracing_subscriber::layer::Identity>,
                    None,
                    output_enabled.then(|| Shared(output.clone())),
                );
                let evaluated = std::cell::Cell::new(0_u32);
                tracing::subscriber::with_default(subscriber, || {
                    let decision = tracing::info_span!("decision", request_id = 7);
                    let _decision = decision.enter();
                    let warning = tracing::warn_span!("warning_context", retained = true);
                    let _warning = warning.enter();
                    tracing::trace!(evaluated = evaluate_field(&evaluated), "filtered trace");
                    tracing::debug!(evaluated = evaluate_field(&evaluated), "filtered debug");
                    tracing::warn!("ordinary warning");
                    let trace = tracing_error::SpanTrace::capture();
                    assert_eq!(trace.status(), tracing_error::SpanTraceStatus::CAPTURED);
                    let rendered = trace.to_string();
                    for context in [
                        "decision",
                        "request_id=7",
                        "warning_context",
                        "retained=true",
                    ] {
                        assert!(rendered.contains(context), "{rendered}");
                    }
                });
                assert_eq!(evaluated.get(), 0, "{policy}, output={output_enabled}");
                let output =
                    String::from_utf8(output.lock().expect("output").clone()).expect("utf8");
                assert_eq!(
                    output.contains("ordinary warning"),
                    output_enabled && policy == "warn"
                );
                assert!(!output.contains("filtered trace") && !output.contains("filtered debug"));
            }
        }
    }

    #[test]
    fn absent_native_provider_preserves_independent_warn_sink_without_enabling_debug() {
        let output = Arc::new(Mutex::new(Vec::new()));
        let extra = fmt::layer()
            .json()
            .with_writer(Shared(output.clone()))
            .with_filter(tracing::level_filters::LevelFilter::WARN);
        let config = configured_local_policy(false, Some("off"), None);
        let subscriber = compose_subscriber(&config, Some(extra), None, None::<Shared>);
        let evaluated = std::cell::Cell::new(0_u32);
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(evaluated = evaluate_field(&evaluated), "filtered debug");
            tracing::warn!("independent warning");
        });
        assert_eq!(evaluated.get(), 0);
        let output = String::from_utf8(output.lock().expect("output").clone()).expect("utf8");
        assert!(output.contains("independent warning"));
        assert!(!output.contains("filtered debug"));
    }

    #[test]
    fn span_trace_keeps_initial_and_late_fields_without_stdout() {
        let config = configured_local_policy(false, Some("off"), None);
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<Shared>,
        );
        tracing::subscriber::with_default(subscriber, || {
            let span =
                tracing::info_span!("decision", request_id = 7, allowed = tracing::field::Empty);
            span.record("allowed", true);
            span.in_scope(|| {
                let trace = tracing_error::SpanTrace::capture();
                assert_eq!(trace.status(), tracing_error::SpanTraceStatus::CAPTURED);
                let rendered = trace.to_string();
                assert!(rendered.contains("request_id=7"), "{rendered}");
                assert_eq!(rendered.matches("allowed=true").count(), 1, "{rendered}");
            });
        });
    }

    #[test]
    fn span_trace_records_late_fields_once_with_pretty_and_json_output() {
        for (log, format) in [
            ("warn", "pretty"),
            ("warn", "json"),
            ("debug", "pretty"),
            ("debug", "json"),
        ] {
            let config = TelemetryConfig::from_raw(
                false,
                TelemetryRawConfig {
                    log_filter: Ok(Some(log.to_owned())),
                    rust_log: Ok(None),
                    log_format: Ok(Some(format.to_owned())),
                    jaunder_otlp_endpoint: Ok(None),
                    otlp_endpoint: Ok(None),
                    slow_op_ms: Ok(None),
                    e2e_seed_process: Ok(None),
                },
            );
            let subscriber = compose_subscriber(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                None,
                Some(Shared(Arc::new(Mutex::new(Vec::new())))),
            );
            tracing::subscriber::with_default(subscriber, || {
                let span = tracing::info_span!(
                    "decision",
                    request_id = 7,
                    allowed = tracing::field::Empty
                );
                span.record("allowed", true);
                span.in_scope(|| {
                    tracing::warn!("console record");
                    let rendered = tracing_error::SpanTrace::capture().to_string();
                    assert!(rendered.contains("request_id=7"), "{format}: {rendered}");
                    assert_eq!(
                        rendered.matches("allowed=true").count(),
                        1,
                        "{format}: {rendered}"
                    );
                });
            });
        }
    }

    #[test]
    fn span_field_formatting_failure_is_local_nonrecursive_and_metric_free() {
        struct BrokenDebug;
        impl std::fmt::Debug for BrokenDebug {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        let ((), output) = fallback_capture::capture(|| {
            assert_zero_error_metrics(|| {
                let config = configured_local_policy(false, Some("off"), None);
                let subscriber = compose_subscriber(
                    &config,
                    None::<tracing_subscriber::layer::Identity>,
                    None,
                    None::<Shared>,
                );
                tracing::subscriber::with_default(subscriber, || {
                    let span = tracing::info_span!("decision", failed = tracing::field::Empty);
                    span.record("failed", tracing::field::debug(BrokenDebug));
                });
            });
        });
        assert_eq!(
            output,
            "host.telemetry.span_fields: diagnostic span field formatting failed\n"
        );
    }

    #[test]
    fn diagnostic_span_context_remains_available_without_an_exporter() {
        let config = configured_local_policy(false, Some("off"), None);
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<Shared>,
        );
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::trace_span!("no_exporter");
            assert!(!span.is_disabled());
        });
    }

    #[test]
    fn local_log_off_leaves_metric_collection_active() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(exporter.clone()).build())
            .build();
        opentelemetry::global::set_meter_provider(provider.clone());
        let config = configured_local_policy(false, Some("off"), None);
        let subscriber = compose_subscriber(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<Shared>,
        );
        tracing::subscriber::with_default(subscriber, || {
            opentelemetry::global::meter("filter proof")
                .u64_counter("independent.metric")
                .build()
                .add(1, &[]);
        });
        provider.force_flush().expect("flush");
        let metrics = exporter.get_finished_metrics().expect("metrics");
        let metric = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .find(|metric| metric.name() == "independent.metric")
            .expect("collected counter");
        let opentelemetry_sdk::metrics::data::AggregatedMetrics::U64(
            opentelemetry_sdk::metrics::data::MetricData::Sum(sum),
        ) = metric.data()
        else {
            unreachable!("a u64 counter exports u64 sum data")
        };
        assert_eq!(
            sum.data_points()
                .map(opentelemetry_sdk::metrics::data::SumDataPoint::value)
                .collect::<Vec<_>>(),
            vec![1]
        );
    }

    #[test]
    fn initial_field_formatting_failure_does_not_recurse_on_late_recording() {
        struct BrokenDebug;
        impl std::fmt::Debug for BrokenDebug {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        let ((), output) = fallback_capture::capture(|| {
            assert_zero_error_metrics(|| {
                let config = configured_local_policy(false, Some("off"), None);
                let subscriber = compose_subscriber(
                    &config,
                    None::<tracing_subscriber::layer::Identity>,
                    None,
                    None::<Shared>,
                );
                tracing::subscriber::with_default(subscriber, || {
                    // Explicit initial values isolate the formatter-failure contract
                    // from the span macro's generated source regions.
                    let callsite = tracing::callsite!(
                        name: "decision",
                        kind: tracing::metadata::Kind::SPAN,
                        level: tracing::Level::INFO,
                        fields: failed, allowed
                    );
                    let metadata = tracing::Callsite::metadata(callsite);
                    let field = metadata.fields().field("failed").expect("declared field");
                    let failed = tracing::field::debug(BrokenDebug);
                    let values = [(&field, Some(&failed as &dyn tracing::field::Value))];
                    let span = tracing::Span::new(metadata, &metadata.fields().value_set(&values));
                    span.record("allowed", true);
                });
            });
        });
        assert_eq!(
            output,
            "host.telemetry.span_fields: diagnostic span field formatting failed\n"
        );
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

    pub(super) fn assert_runtime_fallback_is_nonrecursive<R>(operation: impl FnOnce() -> R) -> R {
        let exporter = InMemorySpanExporter::default();
        let provider = tracer_provider_builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let output = Arc::new(Mutex::new(Vec::new()));
        let subscriber = compose_subscriber(
            &configured_local_policy(false, Some("trace"), None),
            None::<tracing_subscriber::layer::Identity>,
            Some(provider.tracer("runtime-fallback")),
            Some(Shared(output.clone())),
        );
        // Metric-observer setup/flush is not the operation under test; keep its
        // SDK instrumentation outside the application subscriber observation.
        let result = assert_zero_error_metrics(|| {
            tracing::subscriber::with_default(subscriber, || {
                let observer = tracing::info_span!("runtime_fallback_observer");
                observer.in_scope(operation)
            })
        });
        provider.force_flush().expect("flush fallback observer");
        let spans = exporter.get_finished_spans().expect("spans");
        assert_eq!(span_names(&spans), ["runtime_fallback_observer"]);
        assert!(spans[0].events.events.is_empty());
        assert!(output.lock().expect("local output").is_empty());
        result
    }

    #[test]
    fn exporter_runtime_setup_failure_disables_export_with_redacted_nonrecursive_fallback() {
        let (runtime, output) = assert_runtime_fallback_is_nonrecursive(|| {
            fallback_capture::capture(|| {
                setup_exporter_runtime_with(Some("http://127.0.0.1:4317"), || {
                    Err::<(), _>(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "synthetic-secret",
                    ))
                })
            })
        });
        assert!(runtime.is_none());
        assert_eq!(
            output,
            "host.telemetry.exporter_runtime_setup: telemetry export disabled\n"
        );
    }

    #[test]
    fn exporter_runtime_setup_requires_endpoint_and_preserves_success() {
        let calls = std::cell::Cell::new(0);
        let build = || -> std::io::Result<_> {
            calls.set(calls.get() + 1);
            Ok(42)
        };
        let absent = setup_exporter_runtime_with(None, build);
        assert!(absent.is_none());
        assert_eq!(calls.get(), 0, "no endpoint must not construct a runtime");
        let (runtime, output) = fallback_capture::capture(|| {
            setup_exporter_runtime_with(Some("http://127.0.0.1:4317"), build)
        });
        assert_eq!(runtime, Some(42));
        assert_eq!(
            calls.get(),
            1,
            "configured endpoint constructs exactly once"
        );
        assert!(output.is_empty());
    }

    #[test]
    fn process_telemetry_does_not_create_diag_file_when_capture_dir_is_configured() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let capture = crate::capture::CaptureDirectory::new(dir.path().to_path_buf())
            .expect("prepare capture directory");
        let diag_path = capture.path(crate::capture::Stream::Diag);
        let _guard = init_tracing(&default_telemetry_config());
        assert!(!diag_path.exists());
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
        assert_eq!(slow_op_threshold(Ok(None), || {}), Duration::from_secs(5));
    }

    #[test]
    fn slow_op_threshold_reads_typed_override() {
        assert_eq!(
            slow_op_threshold(Ok(Some("1234".to_owned())), || {}),
            Duration::from_millis(1234)
        );
    }

    #[test]
    fn nonnumeric_slow_threshold_uses_default_with_one_fixed_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let threshold = assert_zero_error_metrics(|| {
            slow_op_threshold(Ok(Some("not-a-number".to_owned())), || {
                push_fallback(&mut output, FallbackKind::SlowThreshold);
            })
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
            slow_op_threshold(Err(invalid_unicode_env()), || {
                push_fallback(&mut output, FallbackKind::SlowThreshold);
            })
        });
        assert_eq!(threshold, Duration::from_secs(5));
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::SlowThreshold,
        );
    }
    #[test]
    fn otlp_endpoint_prefers_jaunder_specific_setting() {
        let preferred_fallback_must_not_run = || {
            unreachable!("a valid preferred endpoint must not emit a fallback");
        };

        assert_eq!(
            otel_exporter_otlp_endpoint(
                Ok(Some("http://preferred:4317".to_owned())),
                Ok(Some("http://fallback:4317".to_owned())),
                preferred_fallback_must_not_run,
            )
            .as_deref(),
            Some("http://preferred:4317")
        );
    }

    #[test]
    fn otlp_endpoint_falls_back_to_standard_value() {
        let standard_fallback_must_not_run = || {
            unreachable!("a valid standard endpoint must not emit a fallback");
        };

        assert_eq!(
            otel_exporter_otlp_endpoint(
                Ok(None),
                Ok(Some("http://fallback:4317".to_owned())),
                standard_fallback_must_not_run,
            )
            .as_deref(),
            Some("http://fallback:4317")
        );
    }

    #[test]
    fn blank_primary_endpoint_disables_export_with_one_fixed_fallback() {
        let mut output = Vec::new();
        let endpoint = otel_exporter_otlp_endpoint(
            Ok(Some("   ".to_owned())),
            Ok(Some("http://ignored:4317".to_owned())),
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
        let endpoint = otel_exporter_otlp_endpoint(Ok(None), Ok(Some("   ".to_owned())), || {
            push_fallback(&mut output, FallbackKind::OtlpEndpoint);
        });
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
            otel_exporter_otlp_endpoint(
                Err(invalid_unicode_env()),
                Ok(Some("http://ignored:4317".to_owned())),
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
            otel_exporter_otlp_endpoint(Ok(None), Err(invalid_unicode_env()), || {
                push_fallback(&mut output, FallbackKind::OtlpEndpoint);
            })
        });
        assert!(endpoint.is_none(), "invalid secondary disables export");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::OtlpEndpoint,
        );
    }

    #[test]
    fn use_json_format_defaults_to_pretty() {
        assert!(!use_json_format(Ok(None), || {}));
    }

    #[test]
    fn use_json_format_accepts_json() {
        assert!(use_json_format(Ok(Some("json".to_owned())), || {}));
    }

    #[test]
    fn use_json_format_accepts_pretty() {
        assert!(!use_json_format(Ok(Some("pretty".to_owned())), || {}));
    }

    #[test]
    fn invalid_unicode_log_format_uses_pretty() {
        assert!(!use_json_format(Err(invalid_unicode_env()), || {}));
    }

    #[test]
    fn invalid_unicode_log_format_uses_pretty_with_one_redacted_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let use_json = assert_zero_error_metrics(|| {
            use_json_format(Err(invalid_unicode_env()), || {
                push_fallback(&mut output, FallbackKind::LogFormat);
            })
        });
        assert!(!use_json, "pretty format remains the invalid-value default");
        assert_fixed_fallback(
            &String::from_utf8(output).expect("fallback utf8"),
            FallbackKind::LogFormat,
        );
    }

    #[test]
    fn resolved_filter_accepts_valid_jaunder_directive() {
        assert_eq!(
            format!(
                "{:?}",
                resolved_filter(false, Ok(Some("jaunder=info".to_owned())), Ok(None), || {},)
            ),
            format!("{:?}", EnvFilter::new("jaunder=info"))
        );
    }

    #[test]
    fn invalid_log_filter_directive_uses_default_with_one_fixed_fallback_and_zero_metrics() {
        let mut output = Vec::new();
        let filter = assert_zero_error_metrics(|| {
            resolved_filter(
                false,
                Ok(Some("[not-a-directive".to_owned())),
                Ok(None),
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
            resolved_filter(false, Err(invalid_unicode_env()), Ok(None), || {
                push_fallback(&mut output, FallbackKind::LogFilter);
            })
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
            resolved_filter(false, Ok(None), Err(invalid_unicode_env()), || {
                push_fallback(&mut output, FallbackKind::LogFilter);
            })
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

    fn configured_local_policy(
        verbose: bool,
        log_filter: Option<&str>,
        rust_log: Option<&str>,
    ) -> TelemetryConfig {
        TelemetryConfig::from_raw(
            verbose,
            TelemetryRawConfig {
                log_filter: Ok(log_filter.map(str::to_owned)),
                rust_log: Ok(rust_log.map(str::to_owned)),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(None),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        )
    }

    #[test]
    fn local_filter_accessor_preserves_precedence_and_verbose_defaults() {
        assert_eq!(
            configured_local_policy(true, Some("warn"), Some("debug"))
                .local_log_filter()
                .to_string(),
            "warn"
        );
        assert_eq!(
            configured_local_policy(true, None, None)
                .local_log_filter()
                .to_string(),
            default_filter(true).to_string()
        );
        assert_eq!(
            configured_local_policy(false, None, Some("off"))
                .local_log_filter()
                .to_string(),
            "off"
        );
        assert_eq!(
            default_telemetry_config().local_log_filter().to_string(),
            default_filter(false).to_string()
        );
    }

    #[test]
    fn raw_config_applies_fallbacks_and_preserves_the_recognised_seed_process() {
        let config = assert_zero_error_metrics(|| {
            TelemetryConfig::from_raw(
                false,
                TelemetryRawConfig {
                    log_filter: Err(invalid_unicode_env()),
                    rust_log: Ok(None),
                    log_format: Err(invalid_unicode_env()),
                    jaunder_otlp_endpoint: Err(invalid_unicode_env()),
                    otlp_endpoint: Ok(Some("http://ignored:4317".to_owned())),
                    slow_op_ms: Err(invalid_unicode_env()),
                    e2e_seed_process: Ok(Some(E2E_SEED_PROCESS_JAUNDER.to_owned())),
                },
            )
        });
        assert_eq!(
            format!("{:?}", config.filter),
            format!("{:?}", default_filter(false))
        );
        assert!(!config.json_format);
        assert!(!config.otlp_endpoint_configured());
        assert_eq!(config.slow_op_threshold, Duration::from_secs(5));
        assert_eq!(
            config.e2e_seed_process.as_deref(),
            Some(E2E_SEED_PROCESS_JAUNDER)
        );
    }

    #[test]
    fn telemetry_resource_records_recognised_e2e_seed_process_marker() {
        let resource = telemetry_resource(e2e_seed_process(Ok(Some(
            E2E_SEED_PROCESS_TEST_SUPPORT.to_owned(),
        ))));
        assert_eq!(
            resource.get(&opentelemetry::Key::from_static_str(E2E_SEED_PROCESS_ATTR)),
            Some(Value::from(E2E_SEED_PROCESS_TEST_SUPPORT))
        );
    }

    #[test]
    fn telemetry_resource_ignores_unrecognised_e2e_seed_process_marker() {
        let resource = telemetry_resource(e2e_seed_process(Ok(Some("user-controlled".to_owned()))));
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
            setup_provider_with(
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
            setup_provider_with(
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
        let config = TelemetryConfig::from_raw(
            false,
            TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(Some("http://127.0.0.1:4317".to_owned())),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        );
        init_tracing_impl(&config);
    }

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint() {
        let config = TelemetryConfig::from_raw(
            false,
            TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(Some("not a valid endpoint".to_owned())),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        );
        let guard = init_tracing_impl(&config);
        assert!(!guard.otel_tracing_enabled());
    }

    #[test]
    fn init_tracing_impl_handles_json_output() {
        let config = TelemetryConfig::from_raw(
            false,
            TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(Some("json".to_owned())),
                jaunder_otlp_endpoint: Ok(None),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        );
        init_tracing_impl(&config);
    }

    #[test]
    fn subscriber_install_failure_is_nonfatal_in_subprocess() {
        const CHILD: &str = "JAUNDER_TEST_SUBSCRIBER_INSTALL_CHILD";
        if std::env::var_os(CHILD).is_some() {
            tracing::subscriber::set_global_default(tracing_subscriber::registry())
                .expect("install test subscriber");
            assert_zero_error_metrics(|| {
                let guard = init_tracing_impl(&default_telemetry_config());
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
        drop(TelemetryGuard::new(
            Some(sdk_boundary::ExporterRuntime::new().expect("runtime")),
            Some(provider),
            None,
            None,
            true,
        ));

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
        let exporter = RetainingSpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter.clone())
            .build();

        provider.tracer("test").in_span("test-span", |_cx| {});
        drop(TelemetryGuard::new(
            Some(sdk_boundary::ExporterRuntime::new().expect("runtime")),
            None,
            Some(provider),
            None,
            true,
        ));

        let spans = exporter.0.get_finished_spans().expect("spans");
        assert!(
            spans.iter().any(|span| span.name == "test-span"),
            "span not exported on guard drop"
        );
    }

    #[test]
    fn guard_requires_provider_and_subscriber_for_otel_tracing() {
        let installed = TelemetryGuard::new(
            None,
            None,
            Some(SdkTracerProvider::builder().build()),
            None,
            true,
        );
        assert!(installed.otel_tracing_enabled());
        drop(installed);

        let rejected = TelemetryGuard::new(
            None,
            None,
            Some(SdkTracerProvider::builder().build()),
            None,
            false,
        );
        assert!(!rejected.otel_tracing_enabled());
    }

    #[test]
    fn guard_drop_is_noop_when_inert() {
        // No OTLP endpoint configured -> both providers None -> Drop does nothing
        // and must not panic.
        drop(TelemetryGuard::new(None, None, None, None, true));
    }

    #[tokio::test]
    async fn meter_shutdown_failure_preserves_primary_result_with_one_fallback_and_zero_metrics() {
        let meter = SdkMeterProvider::builder()
            .with_reader(PeriodicReader::builder(InMemoryMetricExporter::default()).build())
            .build();
        let (primary, output) = assert_zero_error_metrics(|| {
            fallback_capture::capture(|| {
                let primary: Result<&str, &str> = Ok("preserved");
                let mut guard = TelemetryGuard::new(
                    Some(sdk_boundary::ExporterRuntime::new().expect("runtime")),
                    Some(meter),
                    None,
                    None,
                    true,
                );
                guard.meter_shutdown = |_| {
                    Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                        "injected meter shutdown failure".to_owned(),
                    ))
                };
                drop(guard);
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
                let mut guard = TelemetryGuard::new(
                    Some(sdk_boundary::ExporterRuntime::new().expect("runtime")),
                    None,
                    Some(tracer),
                    None,
                    true,
                );
                guard.tracer_shutdown = |_| {
                    Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
                        "injected tracer shutdown failure".to_owned(),
                    ))
                };
                drop(guard);
                primary
            })
        });
        assert_eq!(primary, Ok("preserved"));
        assert_fixed_fallback(&output, FallbackKind::TracerShutdown);
    }

    #[derive(Clone)]
    struct Reply {
        rejected: Option<i64>,
        calls: Arc<Mutex<usize>>,
    }
    #[tonic::async_trait]
    impl LogsService for Reply {
        async fn export(
            &self,
            _: tonic::Request<ExportLogsServiceRequest>,
        ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
            *self.calls.lock().unwrap() += 1;
            match self.rejected {
                Some(n) => Ok(tonic::Response::new(ExportLogsServiceResponse {
                    partial_success: Some(ExportLogsPartialSuccess {
                        rejected_log_records: n,
                        error_message: "WITHHELD_REMOTE_DETAIL".into(),
                    }),
                })),
                None => Err(tonic::Status::unavailable("WITHHELD_REMOTE_DETAIL")),
            }
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stock_partial_rejection_and_warning_keep_success_and_transport_failure_stays_failed() {
        for rejected in [Some(1), Some(0), None] {
            let incoming =
                tonic::transport::server::TcpIncoming::bind("127.0.0.1:0".parse().unwrap())
                    .unwrap();
            let endpoint = format!("http://{}", incoming.local_addr().unwrap());
            let calls = Arc::new(Mutex::new(0));
            let reply = Reply {
                rejected,
                calls: calls.clone(),
            };
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(
                tonic::transport::Server::builder()
                    .add_service(LogsServiceServer::new(reply))
                    .serve_with_incoming_shutdown(incoming, async {
                        let _ = stopped.await;
                    }),
            );
            let runtime = sdk_boundary::ExporterRuntime::new().unwrap();
            let provider = runtime
                .in_scope(|| {
                    build_otel_logger(
                        &endpoint,
                        Resource::builder_empty().build(),
                        BatchPolicy::resolve(),
                    )
                })
                .unwrap();
            let log = provider.logger("stock fixture");
            let mut record = log.create_log_record();
            record.set_body("safe application message".into());
            log.emit(record);
            assert_eq!(provider.force_flush().is_ok(), rejected.is_some());
            shutdown_logger(&provider);
            drop(log);
            drop(provider);
            drop(runtime);
            assert_eq!(*calls.lock().unwrap(), 1, "no retries or recursive export");
            stop.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }

    #[derive(Debug)]
    struct GatedExport {
        started: std::sync::mpsc::Sender<usize>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        counts: Arc<Mutex<Vec<usize>>>,
    }
    impl opentelemetry_sdk::logs::LogExporter for GatedExport {
        async fn export(
            &self,
            batch: opentelemetry_sdk::logs::LogBatch<'_>,
        ) -> opentelemetry_sdk::error::OTelSdkResult {
            let count = batch.iter().count();
            self.counts.lock().unwrap().push(count);
            self.started.send(count).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(())
        }
    }
    #[test]
    fn stock_default_queue_and_batch_are_bounded_without_changing_primary_result() {
        const CHILD: &str = "JAUNDER_TEST_STOCK_QUEUE";
        if std::env::var_os(CHILD).is_none() {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "telemetry::tests::stock_default_queue_and_batch_are_bounded_without_changing_primary_result",
                    "--nocapture",
                ])
                .env(CHILD, "1");
            for key in [
                "OTEL_BLRP_MAX_QUEUE_SIZE",
                "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
                "OTEL_BLRP_SCHEDULE_DELAY",
                "OTEL_BLRP_EXPORT_TIMEOUT",
            ] {
                child.env_remove(key);
            }
            let output = child.output().unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stderr}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
            return;
        }
        let (started, received) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let counts = Arc::new(Mutex::new(Vec::new()));
        let logs = logger_provider_with(
            Resource::builder_empty().build(),
            BatchPolicy::resolve(),
            GatedExport {
                started,
                release: Mutex::new(blocked),
                counts: counts.clone(),
            },
        );
        let logger = logs.logger("saturation");
        let offer = || {
            let mut record = logger.create_log_record();
            record.set_body("safe bounded fixture".into());
            logger.emit(record);
        };
        for _ in 0..512 {
            offer();
        }
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), 512);
        for _ in 0..(2048 + 50) {
            offer();
        }
        for _ in 0..5 {
            release.send(()).unwrap();
        }
        let primary = 17;
        shutdown_logger(&logs);
        assert_eq!(primary, 17);
        assert_eq!(counts.lock().unwrap().iter().sum::<usize>(), 512 + 2048);
    }

    #[derive(Debug)]
    struct BatchCounts(Arc<Mutex<Vec<usize>>>);
    impl opentelemetry_sdk::logs::LogExporter for BatchCounts {
        async fn export(
            &self,
            batch: opentelemetry_sdk::logs::LogBatch<'_>,
        ) -> opentelemetry_sdk::error::OTelSdkResult {
            self.0.lock().unwrap().push(batch.iter().count());
            Ok(())
        }
    }
    #[test]
    fn inherited_batch_policy_is_resolved_at_construction_and_snapshot_overrides_builder_defaults()
    {
        const CHILD: &str = "JAUNDER_TEST_STOCK_BATCH";
        if let Ok(mode) = std::env::var(CHILD) {
            let counts = Arc::new(Mutex::new(Vec::new()));
            let policy = if mode == "override" {
                BatchPolicy(
                    opentelemetry_sdk::logs::BatchConfigBuilder::default()
                        .with_max_queue_size(2)
                        .with_max_export_batch_size(2)
                        .with_scheduled_delay(Duration::from_mins(1))
                        .build(),
                )
            } else {
                BatchPolicy::resolve()
            };
            let logs = logger_provider_with(
                Resource::builder_empty().build(),
                policy,
                BatchCounts(counts.clone()),
            );
            let logger = logs.logger("batch fixture");
            for _ in 0..2 {
                let mut record = logger.create_log_record();
                record.set_body("safe batch fixture".into());
                logger.emit(record);
            }
            logs.force_flush().unwrap();
            shutdown_logger(&logs);
            assert_eq!(
                *counts.lock().unwrap(),
                if mode == "zero" { vec![] } else { vec![2] }
            );
            return;
        }
        for mode in ["zero", "override", "clamp"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "telemetry::tests::inherited_batch_policy_is_resolved_at_construction_and_snapshot_overrides_builder_defaults",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .env("OTEL_BLRP_MAX_QUEUE_SIZE", if mode == "clamp" { "2" } else { "0" })
                .env("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", if mode == "clamp" { "99" } else { "0" })
                .env("OTEL_BLRP_SCHEDULE_DELAY", "60000")
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stderr}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"));
        }
    }

    #[test]
    fn repeated_shutdown_failure_is_nonfatal_and_visible_without_coalescing_contract() {
        let provider = SdkLoggerProvider::builder().build();
        provider.shutdown().unwrap();
        let (result, output) = fallback_capture::capture(|| {
            shutdown_logger(&provider);
            shutdown_logger(&provider);
            17
        });
        assert_eq!(result, 17);
        assert_eq!(output.matches("native log shutdown failed").count(), 2);
    }

    #[derive(Clone, Debug, Default)]
    struct Records(Arc<Mutex<Vec<SdkLogRecord>>>);
    impl LogExporter for Records {
        async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
            self.0
                .lock()
                .unwrap()
                .extend(batch.iter().map(|(r, _)| r.clone()));
            Ok(())
        }
    }
    #[test]
    fn stock_records_follow_actual_event_parent_not_ambient_context() {
        let records = Records::default();
        let logs = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let traces = SdkTracerProvider::builder()
            .with_sampler(opentelemetry_sdk::trace::Sampler::ParentBased(Box::new(
                opentelemetry_sdk::trace::Sampler::AlwaysOn,
            )))
            .build();
        let config = default_telemetry_config();
        let dispatch = tracing::Dispatch::new(compose_subscriber_with_logger(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            Some(traces.tracer("context")),
            None::<fn() -> std::io::Stdout>,
            Some(&logs),
        ));
        let (outer, explicit) = tracing::dispatcher::with_default(&dispatch, || {
            let outer = tracing::info_span!(parent: None, "outer");
            let explicit = tracing::info_span!(parent: None, "explicit");
            explicit
                .set_parent(opentelemetry::Context::new().with_remote_span_context(
                    SpanContext::new(
                        TraceId::from(31),
                        SpanId::from(32),
                        TraceFlags::NOT_SAMPLED,
                        true,
                        TraceState::default(),
                    ),
                ))
                .unwrap();
            let expected = (
                outer.context().span().span_context().clone(),
                explicit.context().span().span_context().clone(),
            );
            outer.in_scope(|| {
                tracing::info!("contextual");
                tracing::info!(parent: &explicit, "explicit");
                tracing::info!(parent: None, "root");
                {
                    let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
                    tracing::info!(parent: None, "suppressed");
                }
                assert_eq!(
                    opentelemetry::Context::current().span().span_context(),
                    &expected.0
                );
            });
            expected
        });
        logs.force_flush().unwrap();
        let rows = records.0.lock().unwrap();
        assert_eq!(rows.len(), 3);
        for (body, expected) in [("contextual", outer), ("explicit", explicit)] {
            let row = rows
                .iter()
                .find(|r| r.body() == Some(&AnyValue::from(body)))
                .unwrap();
            let actual = row.trace_context().unwrap();
            assert_eq!(actual.trace_id, expected.trace_id());
            assert_eq!(actual.span_id, expected.span_id());
            assert_eq!(actual.trace_flags, Some(expected.trace_flags()));
        }
        assert!(
            rows.iter()
                .find(|r| r.body() == Some(&AnyValue::from("root")))
                .unwrap()
                .trace_context()
                .is_none()
        );
    }

    #[test]
    fn actual_composition_retains_native_facade_and_parser_levels_under_every_local_policy() {
        tracing_log::LogTracer::init().unwrap();
        log::set_max_level(log::LevelFilter::Trace);
        for (verbose, filter) in [
            (false, Some("warn")),
            (false, Some("off")),
            (false, Some("host=warn,[context{enabled=true}]=off")),
            (false, None),
            (true, None),
        ] {
            let records = Records::default();
            let logs = SdkLoggerProvider::builder()
                .with_simple_exporter(records.clone())
                .build();
            let mut config = default_telemetry_config();
            config.filter = filter.map_or_else(
                || default_filter(verbose),
                tracing_subscriber::EnvFilter::new,
            );
            config.otlp_endpoint = Some("configured".into());
            let subscriber = compose_subscriber_with_logger(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                None,
                None::<fn() -> std::io::Stdout>,
                Some(&logs),
            );
            tracing::subscriber::with_default(subscriber, || {
                tracing::trace!(
                    target: "html5ever::tokenizer",
                    raw = "raw-before-Collector",
                    "parser detail",
                );
                tracing::debug!(
                    target: "html5ever::tokenizer",
                    "parser detail",
                );
                tracing::info!(
                    target: "html5ever::tokenizer",
                    "parser detail",
                );
                tracing::warn!(
                    target: "html5ever::tokenizer",
                    "parser detail",
                );
                tracing::error!(
                    target: "html5ever::tokenizer",
                    "parser detail",
                );
                log::trace!(
                    target: "facade::application",
                    "facade detail",
                );
                log::debug!(
                    target: "facade::application",
                    "facade detail",
                );
                log::info!(
                    target: "facade::application",
                    "facade detail",
                );
                log::warn!(
                    target: "facade::application",
                    "facade detail",
                );
                log::error!(
                    target: "facade::application",
                    "facade detail",
                );
            });
            logs.force_flush().unwrap();
            let rows = records.0.lock().unwrap();
            assert_eq!(rows.len(), 10);
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.severity_number(),
                    Some(
                        [
                            opentelemetry::logs::Severity::Trace,
                            opentelemetry::logs::Severity::Debug,
                            opentelemetry::logs::Severity::Info,
                            opentelemetry::logs::Severity::Warn,
                            opentelemetry::logs::Severity::Error
                        ][index % 5]
                    )
                );
                if index < 5 {
                    assert_eq!(
                        row.target().map(AsRef::as_ref),
                        Some("html5ever::tokenizer")
                    );
                } else {
                    assert_eq!(row.target().map(AsRef::as_ref), Some("log"));
                    assert!(
                        row.attributes_iter()
                            .any(|(k, v)| k.as_str() == "log.target"
                                && v == &AnyValue::from("facade::application"))
                    );
                }
            }
        }
    }
}
