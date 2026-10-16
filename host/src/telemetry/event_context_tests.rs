//! Behavioral checks through the stock SDK record and public tracing context.
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
    trace::{Sampler, SdkTracerProvider},
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
#[test]
fn stock_records_follow_actual_event_parent_not_ambient_context() {
    let records = Records::default();
    let logs = SdkLoggerProvider::builder()
        .with_simple_exporter(records.clone())
        .build();
    let traces = SdkTracerProvider::builder()
        .with_sampler(Sampler::ParentBased(Box::new(Sampler::AlwaysOn)))
        .build();
    let config = super::super::default_telemetry_config();
    let dispatch = Dispatch::new(super::super::compose_subscriber_with_logger(
        &config,
        None::<tracing_subscriber::layer::Identity>,
        Some(traces.tracer("context")),
        None::<fn() -> std::io::Stdout>,
        Some(&logs),
    ));
    let (outer, explicit) = tracing::dispatcher::with_default(&dispatch, || {
        let outer = tracing::info_span!(parent: None, "outer");
        let explicit = tracing::info_span!(parent: None, "explicit");
        explicit
            .set_parent(
                OtelContext::new().with_remote_span_context(SpanContext::new(
                    TraceId::from(31),
                    SpanId::from(32),
                    TraceFlags::NOT_SAMPLED,
                    true,
                    TraceState::default(),
                )),
            )
            .unwrap();
        let expected = (
            outer.context().span().span_context().clone(),
            explicit.context().span().span_context().clone(),
        );
        outer.in_scope(|| {
            tracing::info!("contextual");
            tracing::info!(parent: &explicit, "explicit");
            tracing::info!(parent: None, "root");
            {
                let _scope = OtelContext::enter_telemetry_suppressed_scope();
                tracing::info!(parent: None, "suppressed");
            }
            assert_eq!(OtelContext::current().span().span_context(), &expected.0);
        });
        expected
    });
    logs.force_flush().unwrap();
    let rows = records.0.lock().unwrap();
    assert_eq!(rows.len(), 3);
    for (body, expected) in [("contextual", outer), ("explicit", explicit)] {
        let row = rows
            .iter()
            .find(|r| r.body() == Some(&AnyValue::from(body)))
            .unwrap();
        let actual = row.trace_context().unwrap();
        assert_eq!(actual.trace_id, expected.trace_id());
        assert_eq!(actual.span_id, expected.span_id());
        assert_eq!(actual.trace_flags, Some(expected.trace_flags()));
    }
    assert!(
        rows.iter()
            .find(|r| r.body() == Some(&AnyValue::from("root")))
            .unwrap()
            .trace_context()
            .is_none()
    );
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
    let span = tracing::dispatcher::with_default(&owning, || tracing::info_span!("owning-span"));
    let expected = span.context().span().span_context().clone();
    let other = Dispatch::new(tracing_subscriber::registry());
    tracing::dispatcher::with_default(&other, || {
        let field = META.fields().field("message").unwrap();
        let value: &dyn tracing::field::Value = &"owning event";
        let entries = [(&field, Some(value))];
        let fields = META.fields().value_set(&entries);
        owning.event(&Event::new_child_of(span.id(), &META, &fields));
    });
    logs.force_flush().unwrap();
    let rows = records.0.lock().unwrap();
    assert_eq!(rows.len(), 1);
    let context = rows[0].trace_context().unwrap();
    assert_eq!(context.trace_id, expected.trace_id());
    assert_eq!(context.span_id, expected.span_id());
}

#[test]
fn actual_composition_retains_native_facade_and_parser_levels_under_every_local_policy() {
    tracing_log::LogTracer::init().unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    for (verbose, filter) in [
        (false, Some("warn")),
        (false, Some("off")),
        (false, Some("host=warn,[context{enabled=true}]=off")),
        (false, None),
        (true, None),
    ] {
        let records = Records::default();
        let logs = SdkLoggerProvider::builder()
            .with_simple_exporter(records.clone())
            .build();
        let mut config = super::super::default_telemetry_config();
        config.filter = filter.map_or_else(
            || super::super::default_filter(verbose),
            tracing_subscriber::EnvFilter::new,
        );
        config.otlp_endpoint = Some("configured".into());
        let subscriber = super::super::compose_subscriber_with_logger(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<fn() -> std::io::Stdout>,
            Some(&logs),
        );
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target:"html5ever::tokenizer",raw="raw-before-Collector","parser detail");
            tracing::debug!(target:"html5ever::tokenizer","parser detail");
            tracing::info!(target:"html5ever::tokenizer","parser detail");
            tracing::warn!(target:"html5ever::tokenizer","parser detail");
            tracing::error!(target:"html5ever::tokenizer","parser detail");
            log::trace!(target:"facade::application","facade detail");
            log::debug!(target:"facade::application","facade detail");
            log::info!(target:"facade::application","facade detail");
            log::warn!(target:"facade::application","facade detail");
            log::error!(target:"facade::application","facade detail");
        });
        logs.force_flush().unwrap();
        let rows = records.0.lock().unwrap();
        assert_eq!(rows.len(), 10);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                row.severity_number(),
                Some(
                    [
                        opentelemetry::logs::Severity::Trace,
                        opentelemetry::logs::Severity::Debug,
                        opentelemetry::logs::Severity::Info,
                        opentelemetry::logs::Severity::Warn,
                        opentelemetry::logs::Severity::Error
                    ][index % 5]
                )
            );
            if index < 5 {
                assert_eq!(
                    row.target().map(AsRef::as_ref),
                    Some("html5ever::tokenizer")
                );
            } else {
                assert_eq!(row.target().map(AsRef::as_ref), Some("log"));
                assert!(
                    row.attributes_iter()
                        .any(|(k, v)| k.as_str() == "log.target"
                            && v == &AnyValue::from("facade::application"))
                );
            }
        }
    }
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
        tracing::trace!(target: "unapproved::producer", signed=-17_i64, huge=u128::MAX, user.id=%17, raw="raw-before-collector", "stock raw");
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
