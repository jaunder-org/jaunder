//! Closed native log projection through the pinned SDK logger.
//!
//! Explicit messages become string bodies, with the actual tracing severity.
//! Only the host error carrier's named scalar/Display/Debug fields are admitted;
//! its reviewed `error.context` carrier remains a string, not an exported map.
//! `error.span_trace`, request aggregates, and arbitrary Debug fields are absent.
//! The production credential-consumption events' `user.id` Display projection
//! is a numeric attribute; oversized unsigned scalars retain exact decimal text.
//! Tower's response status is admitted only from its reviewed response target.
//!
//! Request context comes only from the HTTP request span's path-only `uri`,
//! including late updates, never from formatted span fields. Correlation uses
//! the event's real parent and the activation layer's owning weak dispatch.
//! A span-free SDK emission scope prevents ambient fallback without admitting
//! suppressed work. Optional wiring does not select a production sink or own
//! provider shutdown; those responsibilities stay at the composition root.

use opentelemetry::logs::{AnyValue, LogRecord as _, Logger as _, Severity};
use opentelemetry::trace::TraceContextExt as _;
use opentelemetry_sdk::logs::{SdkLogRecord, SdkLogger};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tracing::dispatcher::WeakDispatch;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

pub(super) struct NativeLogs {
    logger: SdkLogger,
    dispatch: Arc<OnceLock<WeakDispatch>>,
}

impl NativeLogs {
    pub(super) fn new(logger: SdkLogger, dispatch: Arc<OnceLock<WeakDispatch>>) -> Self {
        Self { logger, dispatch }
    }
}

impl<S: tracing::Subscriber + for<'a> LookupSpan<'a>> Layer<S> for NativeLogs {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: Context<'_, S>,
    ) {
        if attrs.metadata().name() == "request"
            && attrs.metadata().target() == "jaunder::observability::http"
        {
            let mut path = RequestPath::default();
            attrs.record(&mut path);
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(path);
            }
        }
    }

    fn on_record(
        &self,
        id: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        ctx: Context<'_, S>,
    ) {
        if let Some(span) = ctx.span(id)
            && let Some(path) = span.extensions_mut().get_mut::<RequestPath>()
        {
            values.record(path);
        }
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if opentelemetry::Context::is_current_telemetry_suppressed()
            || excluded_target(event.metadata().target())
        {
            return;
        }
        let parent = ctx.event_span(event).map(|span| span.id());
        let context = parent.and_then(|id| {
            self.dispatch
                .get()
                .and_then(WeakDispatch::upgrade)
                .and_then(|dispatch| tracing_opentelemetry::get_otel_context(&id, &dispatch))
        });
        let mut record = self.logger.create_log_record();
        record.set_timestamp(std::time::SystemTime::now());
        record.set_target(event.metadata().target());
        record.add_attribute("jaunder.target", event.metadata().target());
        if let Some(span) = ctx.event_span(event) {
            for parent in span.scope() {
                if let Some(path) = parent
                    .extensions()
                    .get::<RequestPath>()
                    .and_then(|path| path.0.clone())
                {
                    record.add_attribute("jaunder.request.uri", path);
                    break;
                }
            }
        }
        let severity = match *event.metadata().level() {
            tracing::Level::TRACE => Severity::Trace,
            tracing::Level::DEBUG => Severity::Debug,
            tracing::Level::INFO => Severity::Info,
            tracing::Level::WARN => Severity::Warn,
            tracing::Level::ERROR => Severity::Error,
        };
        record.set_severity_number(severity);
        record.set_severity_text(severity.name());
        event.record(&mut Fields {
            record: &mut record,
            target: event.metadata().target(),
        });
        if let Some(context) = context {
            let span = context.span();
            let parent = span.span_context();
            if parent.is_valid() {
                record.set_trace_context(
                    parent.trace_id(),
                    parent.span_id(),
                    Some(parent.trace_flags()),
                );
            }
        }
        // The SDK fills missing IDs from ambient context. A span-free emission
        // scope keeps explicit roots and unavailable tracers genuinely absent;
        // SDK suppression is checked above and never cleared to admit a record.
        let _scope = opentelemetry::Context::new().attach();
        self.logger.emit(record);
    }
}

fn excluded_target(target: &str) -> bool {
    [
        "opentelemetry",
        "tracing_opentelemetry",
        "host::telemetry",
        "host.telemetry",
        "jaunder::observability::diagnostics",
        "server::observability::diagnostics",
        "server.observability",
    ]
    .iter()
    .any(|prefix| target.starts_with(prefix))
}

#[derive(Default)]
struct RequestPath(Option<String>);

impl Visit for RequestPath {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "uri" {
            self.0 = (value.starts_with('/')
                && !value.contains(['?', '#'])
                && !value.chars().any(char::is_control))
            .then(|| value.to_owned());
        }
    }
    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

struct Fields<'a> {
    record: &'a mut SdkLogRecord,
    target: &'a str,
}

impl Fields<'_> {
    fn admitted(&self, name: &str) -> bool {
        match name {
            "error.kind" | "error.class" | "error.public" | "error.source" | "error.context"
            | "error.disposition" | "telemetry.origin" => self.target == "host::error",
            "db.system" => [
                "host::",
                "storage::",
                "web::",
                "jaunder::",
                "test_support::",
            ]
            .iter()
            .any(|prefix| self.target.starts_with(prefix)),
            "user.id" => matches!(
                self.target,
                "web::registration::api" | "web::password_reset::api" | "web::email::api"
            ),
            "status" => self.target == "tower_http::trace::on_response",
            _ => false,
        }
    }
    fn scalar(&mut self, field: &Field, value: AnyValue) {
        if self.admitted(field.name()) {
            self.record.add_attribute(field.name().to_owned(), value);
        }
    }
}

static FORMAT_FAILURE_REPORTED: AtomicBool = AtomicBool::new(false);

fn render(value: &dyn std::fmt::Debug) -> Option<String> {
    let mut output = String::new();
    if write!(&mut output, "{value:?}").is_err() {
        if !FORMAT_FAILURE_REPORTED.swap(true, Ordering::Relaxed) {
            super::fallback(super::FallbackKind::NativeLogProjection);
        }
        None
    } else {
        Some(output)
    }
}

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.record.set_body(AnyValue::from(value.to_owned()));
        } else {
            self.scalar(field, AnyValue::from(value.to_owned()));
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.scalar(field, AnyValue::Int(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        // OTLP integers are signed. Exact decimal text preserves oversized
        // values without turning a large identifier into a negative one.
        self.scalar(
            field,
            i64::try_from(value).map_or_else(|_| AnyValue::from(value.to_string()), AnyValue::Int),
        );
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.scalar(field, AnyValue::Boolean(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.scalar(field, AnyValue::Double(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        // Only reviewed host error carriers and numeric ID representations have
        // a Debug door. Other fields are not even rendered for diagnostics.
        let carrier =
            self.target == "host::error" && name.starts_with("error.") && self.admitted(name);
        let numeric = matches!(name, "user.id" | "status") && self.admitted(name);
        if (name == "message" || carrier || numeric)
            && let Some(value) = render(value)
        {
            if name == "message" {
                self.record.set_body(AnyValue::from(value));
            } else if numeric {
                let number = if name == "status" {
                    value
                        .strip_prefix("Some(")
                        .and_then(|value| value.strip_suffix(')'))
                        .unwrap_or(&value)
                } else {
                    &value
                };
                if let Ok(value) = number.parse::<i64>() {
                    self.scalar(field, AnyValue::Int(value));
                }
            } else {
                self.scalar(field, AnyValue::from(value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::LoggerProvider as _;
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceFlags, TraceId, TraceState, TracerProvider as _,
    };
    use opentelemetry_sdk::logs::{InMemoryLogExporter, SdkLoggerProvider};
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    fn capture(
        tracer: Option<opentelemetry_sdk::trace::SdkTracer>,
        operation: impl FnOnce(),
    ) -> Vec<SdkLogRecord> {
        let exporter = InMemoryLogExporter::default();
        let provider = SdkLoggerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = super::super::compose_subscriber_with_logger(
            &super::super::default_telemetry_config(),
            None::<tracing_subscriber::layer::Identity>,
            tracer,
            None::<fn() -> std::io::Stdout>,
            Some(provider.logger("projection")),
        );
        tracing::subscriber::with_default(subscriber, operation);
        exporter
            .get_emitted_logs()
            .expect("logs")
            .into_iter()
            .map(|log| log.record)
            .collect()
    }

    fn remote(trace: &str, span: &str) -> opentelemetry::Context {
        opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex(trace).expect("trace ID"),
            SpanId::from_hex(span).expect("span ID"),
            TraceFlags::NOT_SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    #[test]
    fn native_projection_all_severities_ignore_local_policy_and_sdk_work_stays_excluded() {
        for (verbose, policy) in [
            (false, Some("warn")),
            (false, Some("off")),
            (false, Some("host=warn,[context{enabled=true}]=off")),
            (false, None),
            (true, None),
        ] {
            let exporter = InMemoryLogExporter::default();
            let logger = SdkLoggerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            let tracer = super::super::tracer_provider_builder().build();
            let config = super::super::TelemetryConfig::from_raw(
                verbose,
                super::super::TelemetryRawConfig {
                    log_filter: Ok(policy.map(str::to_owned)),
                    rust_log: Ok(None),
                    log_format: Ok(None),
                    jaunder_otlp_endpoint: Ok(None),
                    otlp_endpoint: Ok(None),
                    slow_op_ms: Ok(None),
                    e2e_seed_process: Ok(None),
                },
            );
            let subscriber = super::super::compose_subscriber_with_logger(
                &config,
                None::<tracing_subscriber::layer::Identity>,
                Some(tracer.tracer("severity")),
                None::<fn() -> std::io::Stdout>,
                Some(logger.logger("severity")),
            );
            tracing::subscriber::with_default(subscriber, || {
                let context =
                    tracing::info_span!(target: "host::application", "context", enabled = true);
                context.in_scope(|| {
                    tracing::trace!(target: "host::application", "trace message");
                    tracing::debug!(target: "host::application", "debug message");
                    tracing::info!(target: "host::application", "info message");
                    tracing::warn!(target: "host::application", "warn message");
                    tracing::error!(target: "host::application", "error message");
                });
                let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
                context.in_scope(|| {
                    tracing::error!(target: "host::application", parent: None, "SDK root must stay excluded");
                    tracing::error!(target: "host::application", parent: &context, "SDK explicit must stay excluded");
                    assert!(opentelemetry::Context::is_current_telemetry_suppressed());
                });
                assert!(opentelemetry::Context::is_current_telemetry_suppressed());
            });
            let records = exporter.get_emitted_logs().expect("logs");
            assert_eq!(records.len(), 5, "{verbose}/{policy:?}");
            assert_eq!(
                records
                    .iter()
                    .map(|log| log.record.severity_number().expect("severity"))
                    .collect::<Vec<_>>(),
                [
                    Severity::Trace,
                    Severity::Debug,
                    Severity::Info,
                    Severity::Warn,
                    Severity::Error
                ]
            );
            assert_eq!(
                records
                    .iter()
                    .map(|log| log.record.body().expect("body").clone())
                    .collect::<Vec<_>>(),
                [
                    "trace message",
                    "debug message",
                    "info message",
                    "warn message",
                    "error message"
                ]
                .map(AnyValue::from)
            );
            assert!(
                records
                    .iter()
                    .all(|log| log.record.trace_context().is_some())
            );
            assert!(!opentelemetry::Context::is_current_telemetry_suppressed());
        }
    }

    #[test]
    fn native_projection_has_no_ambient_fallback_without_a_tracer_and_preserves_suppression() {
        let ambient = remote("33333333333333333333333333333333", "3333333333333333");
        let _ambient = ambient.clone().attach();
        let records = capture(None, || {
            let parent = tracing::info_span!("unavailable_tracer");
            parent.in_scope(|| {
                tracing::info!(target: "host::application", "contextual without tracer");
                tracing::info!(target: "host::application", parent: &parent, "explicit without tracer");
                tracing::info!(target: "host::application", parent: None, "root without tracer");
                let _suppressed = opentelemetry::Context::enter_telemetry_suppressed_scope();
                tracing::error!(target: "host::application", parent: None, "suppressed root");
                tracing::error!(target: "host::application", parent: &parent, "suppressed explicit");
                assert!(opentelemetry::Context::is_current_telemetry_suppressed());
            });
        });
        assert_eq!(records.len(), 3);
        assert!(
            records
                .iter()
                .all(|record| record.trace_context().is_none())
        );
        assert_eq!(
            opentelemetry::Context::current().span().span_context(),
            ambient.span().span_context()
        );
        assert!(!opentelemetry::Context::is_current_telemetry_suppressed());
    }

    #[test]
    fn native_projection_context_is_dispatch_qualified() {
        let provider = super::super::tracer_provider_builder().build();
        let mut inner = Vec::new();
        let outer = capture(Some(provider.tracer("outer")), || {
            let parent = tracing::info_span!(parent: None, "outer_parent");
            parent
                .set_parent(remote(
                    "44444444444444444444444444444444",
                    "4444444444444444",
                ))
                .expect("parent");
            parent.in_scope(|| {
                inner = capture(Some(provider.tracer("inner")), || {
                    let inner_parent = tracing::info_span!(parent: None, "inner_parent");
                    inner_parent.set_parent(remote("55555555555555555555555555555555", "5555555555555555")).expect("parent");
                    tracing::info!(target: "host::application", parent: &inner_parent, "inner explicit");
                    tracing::info!(target: "host::application", parent: None, "inner root");
                });
                tracing::info!(target: "host::application", "outer contextual");
            });
        });
        assert_eq!(outer.len(), 1);
        assert_eq!(inner.len(), 2);
        assert_eq!(
            outer[0].trace_context().expect("outer").trace_id,
            TraceId::from_hex("44444444444444444444444444444444").expect("trace")
        );
        assert_eq!(
            inner[0].trace_context().expect("inner").trace_id,
            TraceId::from_hex("55555555555555555555555555555555").expect("trace")
        );
        assert!(inner[1].trace_context().is_none());
    }

    #[test]
    fn native_projection_ignores_sdk_and_local_fallback_targets_even_without_suppression() {
        let records = capture(None, || {
            tracing::error!(target: "opentelemetry_sdk", "synthetic export payload");
            tracing::error!(target: "opentelemetry_otlp", "synthetic partial success payload");
            tracing::error!(target: "tracing_opentelemetry", "synthetic bridge failure");
            tracing::error!(target: "host::telemetry::native_logs", "synthetic self failure");
            tracing::error!(target: "host.telemetry.tracer_shutdown", "synthetic shutdown failure");
            tracing::error!(target: "jaunder::observability::diagnostics", "synthetic parent failure");
            tracing::error!(target: "server.observability.trace_parent", "synthetic parent failure");
            tracing::info!(target: "host::application", "application control");
        });
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].body(),
            Some(&AnyValue::from("application control"))
        );
    }

    #[test]
    fn native_projection_never_renders_unreviewed_debug_fields_or_inherits_request_aggregates() {
        struct MustNotRender;
        impl std::fmt::Debug for MustNotRender {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                panic!("unreviewed Debug field was rendered");
            }
        }
        let records = capture(None, || {
            tracing::info!(target: "external::library", { error.source = ?MustNotRender, error.span_trace = ?MustNotRender, request = ?MustNotRender, db.system = ?MustNotRender }, "external control");
            tracing::info!(target: "host::application", { error.span_trace = ?MustNotRender, request = ?MustNotRender, db.system = ?MustNotRender }, "application control");
            let wrong =
                tracing::info_span!(target: "external::library", "request", uri = "/unreviewed");
            wrong.in_scope(|| tracing::info!(target: "host::application", "wrong producer"));
            let request = tracing::info_span!(target: "jaunder::observability::http", "request", uri = tracing::field::Empty);
            request.record("uri", "/safe");
            request.in_scope(|| tracing::info!(target: "host::application", "late path"));
            request.record("uri", "/unsafe?synthetic-secret");
            request.in_scope(|| tracing::info!(target: "host::application", "query rejected"));
            tracing::info!(target: "host::application", parent: None, "root has no request path");
        });
        assert_eq!(records.len(), 6);
        for (index, record) in records.iter().enumerate() {
            let path = record
                .attributes_iter()
                .find(|(key, _)| key.as_str() == "jaunder.request.uri");
            assert_eq!(
                path.map(|(_, value)| value.clone()),
                (index == 3).then(|| AnyValue::from("/safe"))
            );
        }
    }

    #[test]
    fn native_projection_formatting_failure_is_bounded_redacted_once_and_nonrecursive() {
        struct BrokenDebug;
        impl std::fmt::Debug for BrokenDebug {
            fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        let (records, diagnostic) = super::super::fallback_capture::capture(|| {
            capture(None, || {
                for _ in 0..20 {
                    tracing::info!(target: "host::application", message = ?BrokenDebug);
                }
                tracing::info!(target: "host::application", "still operational");
            })
        });
        assert_eq!(records.len(), 21);
        assert!(records[..20].iter().all(|record| record.body().is_none()));
        assert_eq!(
            records[20].body(),
            Some(&AnyValue::from("still operational"))
        );
        assert_eq!(
            diagnostic,
            "host.telemetry.native_log_projection: native log field formatting failed\n"
        );
        assert!(diagnostic.len() <= 256);
    }

    #[test]
    fn native_projection_preserves_actual_host_error_carrier_representations() {
        let records = capture(None, || {
            crate::error::InternalError::masked(
                crate::error::ErrorKind::Storage,
                crate::error::ErrorClass::Bug,
                "storage operation failed",
                anyhow::Error::new(std::io::Error::other("storage unavailable")),
            )
            .with_context("operation", "cleanup")
            .emit_boundary_failure();
            crate::error::report_swallowed(
                crate::error::ErrorKind::Storage,
                crate::error::ErrorClass::Transient,
                "server.cleanup",
                crate::error::SwallowedSource::Redacted,
            );
        });
        assert_eq!(records.len(), 2);
        let attrs = |index: usize, key: &str| {
            records[index]
                .attributes_iter()
                .find(|(name, _)| name.as_str() == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(attrs(0, "error.kind"), Some(AnyValue::from("Storage")));
        assert_eq!(attrs(0, "error.class"), Some(AnyValue::from("Bug")));
        assert_eq!(
            attrs(0, "error.public"),
            Some(AnyValue::from("storage operation failed"))
        );
        assert_eq!(
            attrs(0, "error.source"),
            Some(AnyValue::from("storage unavailable"))
        );
        assert_eq!(
            attrs(0, "error.context"),
            Some(AnyValue::from("[(\"operation\", \"cleanup\")]"))
        );
        assert_eq!(attrs(1, "error.kind"), Some(AnyValue::from("storage")));
        assert_eq!(
            attrs(1, "error.context"),
            Some(AnyValue::from("server.cleanup"))
        );
        assert_eq!(attrs(1, "error.source"), Some(AnyValue::from("redacted")));
        assert!(records.iter().all(|record| {
            record
                .attributes_iter()
                .all(|(key, _)| key.as_str() != "error.span_trace")
        }));
    }

    #[test]
    fn native_projection_preserves_scalar_and_reviewed_identifier_types_without_unsigned_wrap() {
        let records = capture(None, || {
            let user_id = common::ids::UserId::from(42);
            tracing::info!(target: "web::registration::api", { user.id = %user_id }, "credential consumed");
            tracing::info!(target: "web::email::api", { user.id = u64::MAX }, "unsigned identifier");
            tracing::info!(target: "host::error", { error.public = true, error.source = 1.5_f64 }, "scalars");
            tracing::info!(target: "tower_http::trace::on_response", status = Some(401_i32), response_headers = ?["synthetic-secret"], "finished processing request");
        });
        assert_eq!(records.len(), 4);
        let attribute = |index: usize, name: &str| {
            records[index]
                .attributes_iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(attribute(0, "user.id"), Some(AnyValue::Int(42)));
        assert_eq!(
            attribute(1, "user.id"),
            Some(AnyValue::from("18446744073709551615"))
        );
        assert_eq!(attribute(2, "error.public"), Some(AnyValue::Boolean(true)));
        assert_eq!(attribute(2, "error.source"), Some(AnyValue::Double(1.5)));
        assert_eq!(attribute(3, "status"), Some(AnyValue::Int(401)));
        assert!(attribute(3, "response_headers").is_none());
    }

    #[test]
    fn native_projection_admits_reviewed_fields_and_only_request_path_context() {
        let records = capture(None, || {
            let request = tracing::info_span!(target: "jaunder::observability::http", "request", uri = "/atompub/nonexistent/posts", headers = ?["synthetic-credential"], body = "synthetic-content", error.span_trace = "synthetic-stack");
            request.in_scope(|| {
                let kind = crate::error::ErrorKind::Storage;
                let class = crate::error::ErrorClass::Bug;
                let post_id = common::ids::PostId::from(7);
                tracing::info!(target: "host::error", { error.kind = ?kind, error.class = ?class,
                    error.public = %"storage operation failed", error.source = %"pool unavailable", error.context = "server.cleanup",
                    error.disposition = "swallowed", telemetry.origin = "server", db.system = "postgres", user_id = 42_i64,
                    post_id = %post_id, error.span_trace = "synthetic-stack", credentials = "synthetic-credential", request = ?["synthetic-content"] },
                    "safe operation failed");
            });
        });
        assert_eq!(records.len(), 1);
        let attributes: std::collections::BTreeMap<_, _> = records[0]
            .attributes_iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        assert_eq!(
            attributes,
            std::collections::BTreeMap::from([
                ("jaunder.target", AnyValue::from("host::error")),
                (
                    "jaunder.request.uri",
                    AnyValue::from("/atompub/nonexistent/posts")
                ),
                ("error.kind", AnyValue::from("Storage")),
                ("error.class", AnyValue::from("Bug")),
                ("error.public", AnyValue::from("storage operation failed")),
                ("error.source", AnyValue::from("pool unavailable")),
                ("error.context", AnyValue::from("server.cleanup")),
                ("error.disposition", AnyValue::from("swallowed")),
                ("telemetry.origin", AnyValue::from("server")),
                ("db.system", AnyValue::from("postgres")),
            ])
        );
        let encoded = format!("{:?}", records[0]);
        for rejected in [
            "synthetic-credential",
            "synthetic-content",
            "synthetic-stack",
        ] {
            assert!(!encoded.contains(rejected), "{encoded}");
        }
    }

    #[test]
    fn native_projection_missing_bridge_parent_does_not_borrow_the_current_context() {
        let provider = super::super::tracer_provider_builder().build();
        let records = capture(Some(provider.tracer("missing-parent")), || {
            let missing = {
                let _suppressed = opentelemetry::Context::enter_telemetry_suppressed_scope();
                tracing::info_span!(target: "host::application", parent: None, "suppressed_birth")
            };
            let current =
                tracing::info_span!(target: "host::application", parent: None, "available_parent");
            current.in_scope(|| {
                tracing::info!(target: "host::application", parent: &missing, "missing bridge parent");
                tracing::info!(target: "host::application", parent: None, message = "literal root body");
                tracing::info!(target: "host::application", "contextual control");
            });
        });
        assert_eq!(records.len(), 3);
        assert!(records[0].trace_context().is_none());
        assert!(records[1].trace_context().is_none());
        assert_eq!(
            records[1].body(),
            Some(&AnyValue::from("literal root body"))
        );
        assert!(records[2].trace_context().is_some());
    }

    #[test]
    fn native_projection_resolves_explicit_contextual_and_root_parents() {
        let provider = super::super::tracer_provider_builder().build();
        let mut contexts = Vec::new();
        let records = capture(Some(provider.tracer("parents")), || {
            let explicit = tracing::info_span!(target: "jaunder::observability::http", parent: None, "request", uri = "/explicit");
            explicit
                .set_parent(remote(
                    "11111111111111111111111111111111",
                    "1111111111111111",
                ))
                .expect("parent");
            let current = tracing::info_span!(target: "jaunder::observability::http", parent: None, "request", uri = "/current");
            current
                .set_parent(remote(
                    "22222222222222222222222222222222",
                    "2222222222222222",
                ))
                .expect("parent");
            current.in_scope(|| {
                contexts.push(current.context().span().span_context().clone());
                contexts.push(explicit.context().span().span_context().clone());
                tracing::info!(target: "host::application", "contextual");
                tracing::info!(target: "host::application", parent: &explicit, "explicit");
                tracing::info!(target: "host::application", parent: None, "root");
            });
        });
        assert_eq!(records.len(), 3);
        for (record, context) in records.iter().zip(&contexts) {
            let actual = record.trace_context().expect("correlated");
            assert_eq!(actual.trace_id, context.trace_id());
            assert_eq!(actual.span_id, context.span_id());
            assert_eq!(actual.trace_flags, Some(TraceFlags::SAMPLED));
        }
        assert_ne!(contexts[0].trace_id(), contexts[1].trace_id());
        assert_eq!(
            contexts[0].trace_id(),
            TraceId::from_hex("22222222222222222222222222222222").expect("trace")
        );
        assert_eq!(
            contexts[1].trace_id(),
            TraceId::from_hex("11111111111111111111111111111111").expect("trace")
        );
        assert!(
            records[2].trace_context().is_none(),
            "explicit root must not inherit current SDK context"
        );
        let path = |index: usize| {
            records[index]
                .attributes_iter()
                .find(|(key, _)| key.as_str() == "jaunder.request.uri")
                .map(|(_, value)| value.clone())
        };
        assert_eq!(path(0), Some(AnyValue::from("/current")));
        assert_eq!(path(1), Some(AnyValue::from("/explicit")));
        assert!(path(2).is_none());
    }
}
