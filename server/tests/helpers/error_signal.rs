/// `opentelemetry::global` has one process-wide meter provider. Error-signal
/// assertions serialize provider installation and collection so parallel tests
/// cannot observe one another's metrics.
pub(crate) static ERROR_SIGNAL_METRICS_LOCK: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

/// Captures one request-boundary error event and its `jaunder.errors` point.
///
/// Request-boundary tests reuse the real tracing subscriber and in-memory `OTel`
/// exporter rather than mocking either reporting path.
#[macro_export]
macro_rules! assert_error_signal {
    (
        $future:expr,
        event = $event_marker:literal,
        event_kind = $event_kind:literal,
        event_class = $event_class:literal,
        metric_kind = $metric_kind:literal,
        metric_class = $metric_class:literal,
        disposition = $disposition:literal,
        context = $context:literal
    ) => {{
        #[derive(Clone)]
        struct CapturedWriter {
            output: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
            terminal: std::sync::Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
            marker: &'static str,
        }
        impl std::io::Write for CapturedWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.output
                    .lock()
                    .expect("event capture lock")
                    .extend_from_slice(bytes);
                if String::from_utf8_lossy(bytes).contains(self.marker) {
                    if let Some(terminal) =
                        self.terminal.lock().expect("event terminal lock").take()
                    {
                        terminal
                            .send(())
                            .expect("test waits for captured error event");
                    }
                }
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedWriter {
            type Writer = Self;

            fn make_writer(&'writer self) -> Self::Writer {
                self.clone()
            }
        }

        use opentelemetry_sdk::metrics::{
            InMemoryMetricExporter, PeriodicReader, SdkMeterProvider,
            data::{AggregatedMetrics, MetricData},
        };
        let _metrics_guard = $crate::helpers::error_signal::ERROR_SIGNAL_METRICS_LOCK
            .lock()
            .await;

        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        opentelemetry::global::set_meter_provider(provider.clone());

        let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (terminal_tx, terminal_rx) = tokio::sync::oneshot::channel();
        let terminal = std::sync::Arc::new(std::sync::Mutex::new(Some(terminal_tx)));
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(CapturedWriter {
                output: output.clone(),
                terminal,
                marker: $event_marker,
            })
            .finish();
        let value = {
            let _guard = tracing::subscriber::set_default(subscriber);
            $future.await
        };
        terminal_rx
            .await
            .expect("expected error event reports before assertions");
        provider.force_flush().expect("flush error metrics");

        let text = String::from_utf8(output.lock().expect("event capture lock").clone())
            .expect("captured events are UTF-8");
        let events: Vec<_> = text
            .lines()
            .filter(|line| line.contains($event_marker))
            .collect();
        assert_eq!(events.len(), 1, "exactly one error event: {text}");
        let event = events[0].to_owned();
        assert!(
            event.contains(&format!(r#""error.kind":"{}""#, $event_kind)),
            "event kind: {event}"
        );
        assert!(
            event.contains(&format!(r#""error.class":"{}""#, $event_class)),
            "event class: {event}"
        );
        if !$context.is_empty() {
            assert!(event.contains($context), "event context: {event}");
        }

        let metrics = exporter.get_finished_metrics().expect("finished metrics");
        let points: Vec<_> = metrics
            .iter()
            .flat_map(opentelemetry_sdk::metrics::data::ResourceMetrics::scope_metrics)
            .flat_map(opentelemetry_sdk::metrics::data::ScopeMetrics::metrics)
            .filter(|metric| metric.name() == "jaunder.errors")
            .filter_map(|metric| match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => Some(sum),
                _ => None,
            })
            .flat_map(opentelemetry_sdk::metrics::data::Sum::data_points)
            .map(|point| {
                (
                    point.value(),
                    point
                        .attributes()
                        .map(|kv| (kv.key.as_str().to_owned(), kv.value.to_string()))
                        .collect::<std::collections::BTreeMap<_, _>>(),
                )
            })
            .filter(|(_, attributes)| {
                attributes.get("error.kind").map(String::as_str) == Some($metric_kind)
                    && attributes.get("error.class").map(String::as_str) == Some($metric_class)
                    && attributes.get("error.disposition").map(String::as_str) == Some($disposition)
                    && attributes.get("telemetry.origin").map(String::as_str) == Some("server")
            })
            .collect();
        assert_eq!(points.len(), 1, "one matching jaunder.errors point");
        assert_eq!(points[0].0, 1, "error metric increments exactly once");

        (value, event)
    }};
}
