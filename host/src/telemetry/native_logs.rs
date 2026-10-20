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
mod tests {
    use super::*;
    use opentelemetry::{
        logs::AnyValue,
        trace::{
            SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
            TracerProvider as _,
        },
    };
    use opentelemetry_sdk::{
        error::OTelSdkResult,
        logs::{LogBatch, LogExporter, SdkLogRecord},
        trace::SdkTracerProvider,
    };
    use std::sync::{Arc, Mutex};
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;
    use tracing_subscriber::prelude::*;

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

    struct TestCallsite;
    static CALLSITE: TestCallsite = TestCallsite;
    static META: tracing::Metadata<'static> = tracing::Metadata::new(
        "cross-dispatch",
        "context-proof",
        tracing::Level::INFO,
        Some(file!()),
        Some(line!()),
        Some(module_path!()),
        tracing::field::FieldSet::new(&["message"], tracing::callsite::Identifier(&CALLSITE)),
        tracing::metadata::Kind::EVENT,
    );
    impl tracing::callsite::Callsite for TestCallsite {
        fn set_interest(&self, _: tracing::subscriber::Interest) {}
        fn metadata(&self) -> &tracing::Metadata<'_> {
            &META
        }
    }
    #[test]
    fn event_uses_owning_dispatch_when_another_dispatch_is_current() {
        let metadata = tracing::callsite::Callsite::metadata(&CALLSITE);
        assert!(std::ptr::eq(metadata, &raw const META));
        tracing::callsite::Callsite::set_interest(
            &CALLSITE,
            tracing::subscriber::Interest::always(),
        );

        let records = Records::default();
        let logs = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let traces = SdkTracerProvider::builder().build();
        let owning = Dispatch::new(
            tracing_subscriber::registry()
                .with(tracing_opentelemetry::layer().with_tracer(traces.tracer("owning")))
                .with(EventContextBridge::new(Some(&logs))),
        );
        let span =
            tracing::dispatcher::with_default(&owning, || tracing::info_span!("owning-span"));
        let expected = span.context().span().span_context().clone();
        let other = Dispatch::new(tracing_subscriber::registry());
        tracing::dispatcher::with_default(&other, || {
            let field = metadata.fields().field("message").unwrap();
            let value: &dyn tracing::field::Value = &"owning event";
            let entries = [(&field, Some(value))];
            let fields = metadata.fields().value_set(&entries);
            owning.event(&Event::new_child_of(span.id(), metadata, &fields));
        });
        logs.force_flush().unwrap();
        let rows = records.0.lock().unwrap();
        assert_eq!(rows.len(), 1);
        let context = rows[0].trace_context().unwrap();
        assert_eq!(context.trace_id, expected.trace_id());
        assert_eq!(context.span_id, expected.span_id());
    }

    #[test]
    fn no_tracer_does_not_borrow_ambient_ids_and_preserves_stock_fields() {
        let records = Records::default();
        let logs = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let subscriber = tracing_subscriber::registry().with(EventContextBridge::new(Some(&logs)));
        let ambient = OtelContext::new().with_remote_span_context(SpanContext::new(
            TraceId::from(11),
            SpanId::from(12),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ));
        let _ambient = ambient.attach();
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(
                target: "unapproved::producer",
                signed = -17_i64,
                huge = u128::MAX,
                user.id = %17,
                raw = "raw-before-collector",
                "stock raw"
            );
            tracing::debug!("debug");
            tracing::info!("info");
            tracing::warn!("warn");
            tracing::error!("error");
        });
        logs.force_flush().unwrap();
        let rows = records.0.lock().unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.trace_context().is_none()));
        let attrs = rows[0]
            .attributes_iter()
            .map(|(k, v)| (k.as_str(), v.clone()))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(attrs["raw"], AnyValue::from("raw-before-collector"));
        assert_eq!(attrs["signed"], AnyValue::Int(-17));
        assert_eq!(attrs["huge"], AnyValue::from(u128::MAX.to_string()));
        assert_eq!(attrs["user.id"], AnyValue::from("17"));
    }
}
