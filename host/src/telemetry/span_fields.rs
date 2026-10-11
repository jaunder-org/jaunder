//! `SpanTrace` field storage independent of ordinary-log formatters.
//!
//! A distinct formatter type gives `ErrorLayer` its own extension cache. The
//! console's pretty/JSON caches cannot double-append these late field updates.

use tracing::{Subscriber, span};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::FormattedFields;
use tracing_subscriber::fmt::format::{DefaultFields, FormatFields, Writer};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

pub(super) struct SpanTraceFields;

impl<'writer> FormatFields<'writer> for SpanTraceFields {
    fn format_fields<R: RecordFields>(
        &self,
        writer: Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        let result = DefaultFields::default().format_fields(writer, fields);
        if let Err(error) = &result {
            field_format_fallback(*error);
        }
        result
    }
}

fn field_format_fallback(_error: std::fmt::Error) {
    // Formatting failure belongs to the diagnostic transport, so reporting it
    // through tracing would recurse into the same field formatter.
    super::fallback(super::FallbackKind::SpanFieldFormat);
}

pub(super) struct SpanFieldUpdates;

impl<S> Layer<S> for SpanFieldUpdates
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_record(&self, id: &span::Id, record: &span::Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            if let Some(fields) = extensions.get_mut::<FormattedFields<SpanTraceFields>>() {
                // The formatter reports failures locally; retaining the primary
                // operation must not turn a diagnostic fault into an app fault.
                let _ = SpanTraceFields.add_fields(fields, record);
            }
        }
    }
}
