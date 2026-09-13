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
    let span = tracing::span!(
        Level::INFO,
        "request",
        method = %request.method(),
        uri = %request.uri(),
        version = ?request.version(),
        headers = ?request.headers(),
    );
    if let Some(parent) = request.extensions().get::<ExtractedTraceContext>()
        && span.set_parent(parent.0.clone()).is_err()
    {
        super::diagnostics::report_trace_parent_failure();
    }
    span
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
    use tower::ServiceExt;

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
        assert_eq!(
            output,
            "server.observability.trace_parent: request trace parent assignment failed\n"
        );
    }
}
