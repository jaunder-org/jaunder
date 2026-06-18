use std::sync::Once;
use std::time::{Duration, Instant};

use axum::http::HeaderName;
use axum::Router;
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

static INIT_TRACING: Once = Once::new();

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

fn otel_exporter_otlp_endpoint() -> Option<String> {
    std::env::var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
        .ok()
        .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn build_otel_tracer(endpoint: &str) -> Result<opentelemetry_sdk::trace::Tracer, String> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP span exporter: {error}"))?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    let tracer = provider.tracer("jaunder");
    opentelemetry::global::set_tracer_provider(provider);
    Ok(tracer)
}

fn build_otel_meter(endpoint: &str) -> Result<(), String> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| format!("failed to build OTLP metric exporter: {error}"))?;
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(exporter)
        .build();
    opentelemetry::global::set_meter_provider(provider);
    Ok(())
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
            return;
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
        #[allow(clippy::cast_possible_truncation)]
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

fn init_tracing_impl(verbose: bool) {
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

    let otel_layer =
        otel_exporter_otlp_endpoint().and_then(|endpoint| match build_otel_tracer(&endpoint) {
            Ok(tracer) => Some(tracing_opentelemetry::layer().with_tracer(tracer)),
            Err(error) => {
                eprintln!(
                    "OTel disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                None
            }
        });

    // Metrics share the OTLP endpoint with traces; setup failure is non-fatal.
    if let Some(endpoint) = otel_exporter_otlp_endpoint() {
        if let Err(error) = build_otel_meter(&endpoint) {
            eprintln!(
                "OTel metrics disabled because exporter setup failed (endpoint {endpoint}): {error}"
            );
        }
    }

    // `try_init` fails only if a global subscriber is already installed. That
    // leaves the process running without our configured layers, which is worth
    // knowing about; emit to stderr since tracing itself is what failed to come
    // up.
    if let Err(error) = tracing_subscriber::registry()
        .with(env_filter)
        .with(slow_span_layer)
        .with(fmt_layer)
        .with(otel_layer)
        .try_init()
    {
        eprintln!("tracing subscriber init failed (continuing without it): {error}");
    }
}

pub fn init_tracing(verbose: bool) {
    INIT_TRACING.call_once(|| init_tracing_impl(verbose));
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
    use std::sync::{Mutex, MutexGuard};
    use tower::ServiceExt;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn lock_env() -> MutexGuard<'static, ()> {
        match ENV_LOCK.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
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
                        StatusCode::INTERNAL_SERVER_ERROR
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
