use std::sync::Once;
use std::time::{Duration, Instant};

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

pub fn init_tracing() {
    INIT_TRACING.call_once(|| {
        // Forward any existing `log` macros to tracing so we can migrate in
        // phases without duplicate logging calls.
        let _ = tracing_log::LogTracer::init();

        let env_filter = resolved_filter();
        let slow_span_layer = SlowSpanLayer::new(slow_op_threshold());
        let registry = tracing_subscriber::registry()
            .with(env_filter)
            .with(slow_span_layer);

        if use_json_format() {
            let _ = registry.with(fmt::layer().json()).try_init();
        } else {
            let _ = registry.with(fmt::layer()).try_init();
        }
    });
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
}
