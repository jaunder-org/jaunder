//! Admission witness for the OTLP trace layer, independent of other sinks.
//!
//! The witness layer must share a single filter with the OpenTelemetry layer.
//! Reading it neither evaluates a second filter nor starts the SDK span.

use tracing::{Span, Subscriber, span};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::{LookupSpan, Registry};

struct TraceAdmitted;

pub(super) struct TraceAdmissionLayer;

impl<S> Layer<S> for TraceAdmissionLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(TraceAdmitted);
        }
    }
}

/// Whether this span was admitted to the installed OTLP trace layer.
/// This is not a sampling/export guarantee and does not start its SDK context.
#[must_use]
pub fn span_is_trace_admitted(span: &Span) -> bool {
    span.with_subscriber(|(id, dispatch)| {
        dispatch
            .downcast_ref::<Registry>()
            .and_then(|registry| registry.span(id))
            .is_some_and(|span| span.extensions().get::<TraceAdmitted>().is_some())
    })
    .unwrap_or(false)
}
