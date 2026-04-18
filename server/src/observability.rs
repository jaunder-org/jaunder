use std::sync::Once;
use std::time::{Duration, Instant};

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

static INIT_TRACING: Once = Once::new();

fn default_filter() -> EnvFilter {
    EnvFilter::new("jaunder=debug,web=debug,common=debug,tower_http=debug,sqlx=info")
}

fn resolved_filter() -> EnvFilter {
    EnvFilter::try_from_env("JAUNDER_LOG_FILTER")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| default_filter())
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

pub fn slow_op_threshold() -> Duration {
    std::env::var("JAUNDER_SLOW_OP_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
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
        let Some(started_at) = started_at else {
            return;
        };

        let elapsed = started_at.0.elapsed();
        if let Some((elapsed_ms, threshold_ms)) = slow_span_values(elapsed, self.threshold) {
            let metadata = span.metadata();
            tracing::warn!(
                span_name = metadata.name(),
                span_target = metadata.target(),
                elapsed_ms,
                threshold_ms,
                "slow span detected"
            );
        }
    }
}

fn slow_span_values(elapsed: Duration, threshold: Duration) -> Option<(u64, u64)> {
    if elapsed >= threshold {
        Some((elapsed.as_millis() as u64, threshold.as_millis() as u64))
    } else {
        None
    }
}

fn init_tracing_impl() {
    // Forward any existing `log` macros to tracing so we can migrate in
    // phases without duplicate logging calls.
    let _ = tracing_log::LogTracer::init();
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let env_filter = resolved_filter();
    let slow_span_layer = SlowSpanLayer::new(slow_op_threshold());
    let use_json = use_json_format();

    if let Some(endpoint) = otel_exporter_otlp_endpoint() {
        match build_otel_tracer(&endpoint) {
            Ok(tracer) => {
                if use_json {
                    let _ = tracing_subscriber::registry()
                        .with(env_filter)
                        .with(slow_span_layer)
                        .with(fmt::layer().json())
                        .with(tracing_opentelemetry::layer().with_tracer(tracer))
                        .try_init();
                } else {
                    let _ = tracing_subscriber::registry()
                        .with(env_filter)
                        .with(slow_span_layer)
                        .with(fmt::layer())
                        .with(tracing_opentelemetry::layer().with_tracer(tracer))
                        .try_init();
                }
            }
            Err(error) => {
                eprintln!(
                    "OTel disabled because exporter setup failed (endpoint {endpoint}): {error}"
                );
                if use_json {
                    let _ = tracing_subscriber::registry()
                        .with(env_filter)
                        .with(slow_span_layer)
                        .with(fmt::layer().json())
                        .try_init();
                } else {
                    let _ = tracing_subscriber::registry()
                        .with(env_filter)
                        .with(slow_span_layer)
                        .with(fmt::layer())
                        .try_init();
                }
            }
        }
    } else if use_json {
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(slow_span_layer)
            .with(fmt::layer().json())
            .try_init();
    } else {
        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(slow_span_layer)
            .with(fmt::layer())
            .try_init();
    }
}

pub fn init_tracing() {
    INIT_TRACING.call_once(init_tracing_impl);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};

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

    #[test]
    fn init_tracing_impl_handles_invalid_otel_endpoint() {
        let _guard = lock_env();
        std::env::set_var(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            "not a valid endpoint",
        );
        init_tracing_impl();
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
        init_tracing_impl();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }

    #[test]
    fn init_tracing_impl_handles_no_otel_endpoint_with_json_output() {
        let _guard = lock_env();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::set_var("JAUNDER_LOG_FORMAT", "json");
        init_tracing_impl();
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
        init_tracing_impl();
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
        init_tracing_impl();
        std::env::remove_var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT");
        std::env::remove_var("JAUNDER_LOG_FORMAT");
    }
}
