//! HTTP observability middleware extracts inbound W3C trace context, assigns and
//! propagates request IDs, and creates the request span that adopts that context.

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName};
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::Context;
use opentelemetry::propagation::Extractor;
use tower::ServiceBuilder;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Trace context extracted from inbound request headers (W3C `traceparent`),
/// stashed in request extensions so the request span can adopt it as parent.
#[derive(Clone)]
struct ExtractedTraceContext(Context);

type TraceContextExtractor = fn(&HeaderMap) -> Context;

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

fn extract_from_headers(headers: &HeaderMap) -> Context {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(headers))
    })
}

async fn extract_trace_context(
    State(extract): State<TraceContextExtractor>,
    mut request: Request,
    next: Next,
) -> Response {
    let context = extract(request.headers());
    request
        .extensions_mut()
        .insert(ExtractedTraceContext(context));
    next.run(request).await
}

/// Builds the per-request tracing span, adopting any extracted upstream trace
/// context as its parent.
fn make_request_span(request: &Request) -> Span {
    let trusted_proxy_outcome = request
        .extensions()
        .get::<crate::trusted_proxy::RequestAddress>()
        .map(|address| address.outcome.as_str());
    let span = tracing::span!(
        Level::INFO,
        "request",
        method = %request.method(),
        uri = request.uri().path(),
        version = ?request.version(),
        trusted_proxy.outcome = ?trusted_proxy_outcome,
    );
    if let Some(parent) = request.extensions().get::<ExtractedTraceContext>() {
        adopt_trace_parent(&span, &parent.0);
    }
    span
}

fn adopt_trace_parent(span: &Span, parent: &Context) {
    // Overall span activity includes diagnostics/ordinary logs, not just tracing.
    // Querying admission must not start an SDK context before parent assignment.
    if host::telemetry::span_is_trace_admitted(span)
        && let Err(error) = span.set_parent(parent.clone())
    {
        super::diagnostics::report_trace_parent_failure(&error);
    }
}

/// Applies request IDs and tracing to `router`. Inbound W3C trace-context
/// extraction and parent adoption are enabled only with an installed OTLP tracer.
pub fn with_http_observability<S>(router: Router<S>, trace_parent_enabled: bool) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    with_http_observability_using(router, trace_parent_enabled.then_some(extract_from_headers))
}

fn with_http_observability_using<S>(
    router: Router<S>,
    trace_context_extractor: Option<TraceContextExtractor>,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let request_id_header = HeaderName::from_static("x-request-id");
    let layer = ServiceBuilder::new()
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
    with_trace_context_extraction(router.layer(layer), trace_context_extractor)
}

fn with_trace_context_extraction<S>(
    router: Router<S>,
    trace_context_extractor: Option<TraceContextExtractor>,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    match trace_context_extractor {
        Some(extractor) => router.layer(axum::middleware::from_fn_with_state(
            extractor,
            extract_trace_context,
        )),
        None => router,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{HeaderMap, Request, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context as LayerContext, Layer};
    use tracing_subscriber::prelude::*;

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

    #[test]
    fn production_extractor_parses_valid_trace_parent() {
        use opentelemetry::trace::TraceContextExt as _;

        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .expect("valid traceparent header"),
        );

        let context = extract_from_headers(&headers);
        let span_context = context.span().span_context().clone();
        assert!(span_context.is_valid());
        assert!(span_context.is_remote());
        assert_eq!(
            span_context.trace_id().to_string(),
            "4bf92f3577b34da6a3ce929d0e0e4736"
        );
        assert_eq!(span_context.span_id().to_string(), "00f067aa0ba902b7");
    }

    static EXTRACTION_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn counting_extract(_headers: &HeaderMap) -> Context {
        EXTRACTION_CALLS.fetch_add(1, Ordering::Relaxed);
        Context::new()
    }

    async fn extracted_context_status(request: axum::extract::Request) -> StatusCode {
        if request
            .extensions()
            .get::<ExtractedTraceContext>()
            .is_some()
        {
            StatusCode::OK
        } else {
            StatusCode::NO_CONTENT
        }
    }

    async fn extraction_response(extractor: Option<TraceContextExtractor>) -> Response {
        with_http_observability_using(
            Router::new().route("/", axum::routing::get(extracted_context_status)),
            extractor,
        )
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
        .expect("failed to get response")
    }

    #[derive(Clone, Default)]
    struct SpanFields(Arc<Mutex<Vec<(String, String)>>>);

    struct SpanFieldVisitor<'a>(&'a mut Vec<(String, String)>);

    impl Visit for SpanFieldVisitor<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    impl<S> Layer<S> for SpanFields
    where
        S: tracing::Subscriber,
    {
        fn on_new_span(
            &self,
            attributes: &tracing::span::Attributes<'_>,
            _: &tracing::span::Id,
            _: LayerContext<'_, S>,
        ) {
            let mut fields = self.0.lock().expect("span fields lock");
            attributes.record(&mut SpanFieldVisitor(&mut fields));
        }
    }

    #[test]
    fn request_span_records_only_safe_http_fields() {
        let fields = SpanFields::default();
        let subscriber = tracing_subscriber::registry().with(fields.clone());
        let mut request = Request::builder()
            .method("GET")
            .uri("/atompub/nonexistent/posts?access_token=secret-query-value")
            .header("authorization", "Bearer secret-authorization-value")
            .header("cookie", "session=secret-cookie-value")
            .header("x-forwarded-for", "203.0.113.10")
            .body(Body::empty())
            .expect("failed to build request");
        request
            .extensions_mut()
            .insert(crate::trusted_proxy::RequestAddress {
                transport_peer: Some("10.0.0.2:443".parse().expect("peer")),
                effective_client_ip: Some("203.0.113.10".parse().expect("client IP")),
                outcome: crate::trusted_proxy::ResolutionOutcome::Forwarded,
            });

        tracing::subscriber::with_default(subscriber, || {
            drop(make_request_span(&request));
        });

        let fields = fields.0.lock().expect("span fields lock");
        assert_eq!(
            fields
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["method", "uri", "version", "trusted_proxy.outcome"]
        );
        let serialized = format!("{fields:?}");
        assert!(serialized.contains("/atompub/nonexistent/posts"));
        for secret in [
            "secret-query-value",
            "secret-authorization-value",
            "secret-cookie-value",
            "203.0.113.10",
        ] {
            assert!(
                !serialized.contains(secret),
                "span fields contained {secret}"
            );
        }
    }

    fn w3c_extract(headers: &HeaderMap) -> Context {
        use opentelemetry::propagation::TextMapPropagator as _;
        opentelemetry_sdk::propagation::TraceContextPropagator::new()
            .extract(&HeaderExtractor(headers))
    }

    async fn traced_operation() -> StatusCode {
        assert!(
            !Span::current().is_disabled(),
            "diagnostic request context remains active"
        );
        let child = tracing::info_span!("operation");
        child.in_scope(|| StatusCode::OK)
    }

    fn telemetry_config(log: &str, trace: &str) -> host::telemetry::TelemetryConfig {
        host::telemetry::TelemetryConfig::from_raw(
            false,
            host::telemetry::TelemetryRawConfig {
                log_filter: Ok(Some(log.to_owned())),
                trace_filter: Ok(Some(trace.to_owned())),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(None),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        )
    }

    #[test]
    fn admitted_already_started_parent_failure_is_bounded_and_nonrecursive() {
        use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
        use opentelemetry_sdk::trace::SdkTracerProvider;
        let config = telemetry_config("off", "info");
        let provider = SdkTracerProvider::builder().build();
        let subscriber =
            host::test_support::telemetry_subscriber(&config, Some(provider.tracer("http-test")));
        let ((), output) = super::super::diagnostics::capture_fallbacks(|| {
            super::super::diagnostics::assert_zero_error_metrics(|| {
                tracing::subscriber::with_default(subscriber, || {
                    let span = tracing::info_span!("request");
                    assert!(host::telemetry::span_is_trace_admitted(&span));
                    assert!(span.context().span().span_context().is_valid());
                    adopt_trace_parent(&span, &Context::new());
                });
            });
        });
        assert_eq!(
            output,
            "server.observability.trace_parent: request trace parent assignment failed: reason=span-already-started\n"
        );
    }

    #[test]
    fn missing_sdk_layer_has_a_bounded_diagnostic_but_expected_absence_skips_assignment() {
        let config = telemetry_config("debug", "info");
        let subscriber = host::test_support::telemetry_subscriber(&config, None);
        let ((), output) = super::super::diagnostics::capture_fallbacks(|| {
            super::super::diagnostics::assert_zero_error_metrics(|| {
                tracing::subscriber::with_default(subscriber, || {
                    let span = tracing::info_span!("request");
                    assert!(!span.is_disabled());
                    assert!(!host::telemetry::span_is_trace_admitted(&span));
                    adopt_trace_parent(&span, &Context::new());
                    // A genuine SDK failure, independently of the normal-absence guard.
                    let error = span
                        .set_parent(Context::new())
                        .expect_err("SDK layer missing");
                    assert!(matches!(
                        error,
                        tracing_opentelemetry::SetParentError::LayerNotFound
                    ));
                    super::super::diagnostics::report_trace_parent_failure(&error);
                });
            });
        });
        assert_eq!(
            output,
            "server.observability.trace_parent: request trace parent assignment failed: reason=otel-layer-missing\n"
        );
    }

    #[test]
    fn actual_host_subscriber_preserves_http_parent_correlation_and_quiet_filtering() {
        use opentelemetry::trace::TracerProvider as _;
        use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        for log in ["warn", "debug"] {
            for trace in ["info", "warn", "off", "jaunder::observability::http=warn"] {
                for header in [
                    Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"),
                    None,
                    Some("secret-invalid-parent"),
                ] {
                    let config = telemetry_config(log, trace);
                    let exporter = InMemorySpanExporter::default();
                    let provider = SdkTracerProvider::builder()
                        .with_simple_exporter(exporter.clone())
                        .build();
                    let subscriber = host::test_support::telemetry_subscriber(
                        &config,
                        Some(provider.tracer("http-test")),
                    );
                    let (response, diagnostics) =
                        super::super::diagnostics::capture_fallbacks(|| {
                            tracing::subscriber::with_default(subscriber, || {
                                runtime.block_on(async {
                                    let mut request = Request::builder()
                                        .uri("/")
                                        .header("x-request-id", "known-request");
                                    if let Some(header) = header {
                                        request = request.header("traceparent", header);
                                    }
                                    with_http_observability_using(
                                        Router::new()
                                            .route("/", axum::routing::get(traced_operation)),
                                        Some(w3c_extract),
                                    )
                                    .oneshot(request.body(Body::empty()).expect("request"))
                                    .await
                                    .expect("response")
                                })
                            })
                        });
                    assert_eq!(response.status(), StatusCode::OK);
                    assert_eq!(response.headers()["x-request-id"], "known-request");
                    assert!(diagnostics.is_empty(), "{trace}/{header:?}: {diagnostics}");
                    // TraceLayer retains the request span in the response body until it drops.
                    drop(response);
                    provider.force_flush().expect("flush");
                    let spans = exporter.get_finished_spans().expect("spans");
                    if trace == "info" {
                        let request = spans
                            .iter()
                            .find(|span| span.name == "request")
                            .expect("request span");
                        let operation = spans
                            .iter()
                            .find(|span| span.name == "operation")
                            .expect("operation span");
                        assert_eq!(
                            operation.span_context.trace_id(),
                            request.span_context.trace_id()
                        );
                        assert_eq!(operation.parent_span_id, request.span_context.span_id());
                        if header.is_some_and(|header| header.starts_with("00-")) {
                            assert_eq!(
                                request.span_context.trace_id().to_string(),
                                "4bf92f3577b34da6a3ce929d0e0e4736"
                            );
                            assert_eq!(request.parent_span_id.to_string(), "00f067aa0ba902b7");
                        } else {
                            assert!(request.span_context.is_valid());
                            assert_eq!(
                                request.parent_span_id,
                                opentelemetry::trace::SpanId::INVALID
                            );
                        }
                    } else {
                        assert!(!spans.iter().any(|span| span.name == "request"), "{trace}");
                    }
                }
            }
        }
    }

    #[test]
    fn trace_parent_policy_controls_full_request_path() {
        EXTRACTION_CALLS.store(0, Ordering::Relaxed);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let ((disabled, disabled_calls, enabled), output) =
            super::super::diagnostics::capture_fallbacks(|| {
                runtime.block_on(async {
                    let disabled = extraction_response(None).await;
                    let disabled_calls = EXTRACTION_CALLS.load(Ordering::Relaxed);
                    let enabled = extraction_response(Some(counting_extract)).await;
                    (disabled, disabled_calls, enabled)
                })
            });

        assert_eq!(disabled.status(), StatusCode::NO_CONTENT);
        assert_eq!(disabled_calls, 0);
        assert_eq!(enabled.status(), StatusCode::OK);
        assert_eq!(EXTRACTION_CALLS.load(Ordering::Relaxed), 1);
        assert!(output.is_empty(), "expected trace absence: {output}");
    }
}
