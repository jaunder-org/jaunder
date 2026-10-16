//! Event-parent correlation around the stock log bridge. Retention belongs to
//! the shared Collector policy, not this context-only adapter.
//!
//! The owning dispatch resolves IDs even when another dispatch is current.
//! Roots and unavailable parents cannot borrow ambient IDs; attaching context
//! must preserve SDK suppression. No span extension lock crosses public lookup.
use opentelemetry::Context as OtelContext;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_sdk::logs::{SdkLogger, SdkLoggerProvider};
use std::sync::OnceLock;
use tracing::{Dispatch, Event, Subscriber, dispatcher::WeakDispatch};
use tracing_subscriber::{
    layer::{Context, Layer},
    registry::LookupSpan,
};

pub(super) struct EventContextBridge {
    bridge: Option<OpenTelemetryTracingBridge<SdkLoggerProvider, SdkLogger>>,
    dispatch: OnceLock<WeakDispatch>,
}
impl EventContextBridge {
    pub(super) fn new(provider: Option<&SdkLoggerProvider>) -> Self {
        Self {
            bridge: provider.map(OpenTelemetryTracingBridge::new),
            dispatch: OnceLock::new(),
        }
    }
}
impl<S> Layer<S> for EventContextBridge
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_register_dispatch(&self, dispatch: &Dispatch) {
        let _ = self.dispatch.set(dispatch.downgrade());
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let suppressed = OtelContext::is_current_telemetry_suppressed();
        let Some(bridge) = &self.bridge else { return };
        let parent = ctx.event_span(event).map(|span| span.id());
        let context = parent
            .and_then(|id| {
                self.dispatch
                    .get()
                    .and_then(WeakDispatch::upgrade)
                    .and_then(|dispatch| tracing_opentelemetry::get_otel_context(&id, &dispatch))
            })
            .unwrap_or_default();
        let context = if suppressed {
            context.with_telemetry_suppressed()
        } else {
            context
        };
        let _guard = context.attach();
        bridge.on_event(event, ctx);
    }
}
#[cfg(test)]
#[path = "event_context_tests.rs"]
mod tests;
