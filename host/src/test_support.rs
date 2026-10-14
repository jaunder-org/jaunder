//! Host-owned domain-value and executable-boundary test fixtures.

pub mod telemetry;

use super::feed::{FeedMinDays, FeedMinItems};
use super::password::Password;
use super::smtp_password::SmtpPassword;

/// Composes the actual host subscriber with an injected SDK tracer and discarded
/// ordinary output, without installing a global subscriber or provider.
pub fn telemetry_subscriber(
    config: &crate::telemetry::TelemetryConfig,
    tracer: Option<opentelemetry_sdk::trace::SdkTracer>,
) -> impl tracing::Subscriber + Send + Sync + 'static {
    crate::telemetry::compose_subscriber(
        config,
        None::<tracing_subscriber::layer::Identity>,
        tracer,
        Some(std::io::sink),
    )
}

/// Builds the actual host tracer policy with an injected in-memory/export seam.
#[must_use]
pub fn telemetry_tracer_provider(
    exporter: impl opentelemetry_sdk::trace::SpanExporter + 'static,
) -> opentelemetry_sdk::trace::SdkTracerProvider {
    crate::telemetry::tracer_provider_builder()
        .with_simple_exporter(exporter)
        .build()
}

/// Parses `s` into a valid [`Password`] for tests.
///
/// # Panics
///
/// Panics if `s` does not meet the shared password shape invariant.
#[must_use]
pub fn parse_password(s: &str) -> Password {
    match s.parse() {
        Ok(password) => password,
        Err(error) => panic!("valid test password: {error}"),
    }
}

/// Parses `s` into a valid [`SmtpPassword`] for tests.
///
/// # Panics
///
/// Panics if `s` is empty.
#[must_use]
pub fn parse_smtp_password(s: &str) -> SmtpPassword {
    match s.parse() {
        Ok(password) => password,
        Err(error) => panic!("valid test SMTP password: {error}"),
    }
}

/// Parses `s` into a valid [`FeedMinItems`] for tests.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_items(s: &str) -> FeedMinItems {
    match s.parse() {
        Ok(value) => value,
        Err(error) => panic!("valid test feeds.min_items: {error}"),
    }
}

/// Parses `s` into a valid [`FeedMinDays`] for tests.
///
/// # Panics
///
/// Panics if `s` is not a whole number of at least 1.
#[must_use]
pub fn parse_feed_min_days(s: &str) -> FeedMinDays {
    match s.parse() {
        Ok(value) => value,
        Err(error) => panic!("valid test feeds.min_days: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "valid test password")]
    fn parse_password_rejects_an_invalid_fixture() {
        let _ = parse_password("short");
    }

    #[test]
    #[should_panic(expected = "valid test SMTP password")]
    fn parse_smtp_password_rejects_an_invalid_fixture() {
        let _ = parse_smtp_password("");
    }

    #[test]
    #[should_panic(expected = "valid test feeds.min_items")]
    fn parse_feed_min_items_rejects_an_invalid_fixture() {
        let _ = parse_feed_min_items("0");
    }

    #[test]
    #[should_panic(expected = "valid test feeds.min_days")]
    fn parse_feed_min_days_rejects_an_invalid_fixture() {
        let _ = parse_feed_min_days("0");
    }
}
