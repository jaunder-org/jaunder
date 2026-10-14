//! Native logger ownership and pinned SDK batch policy. The composition boundary
//! resolves inherited policy before exporter work; installed processors receive
//! that complete owned snapshot, not an application environment reader.
//!
//! The pinned public processor builder constructs an unused default (including
//! dependency environment reads) before `with_batch_config` replaces it. That
//! constructor-only limitation cannot change effective policy. Callbacks and
//! teardown do not resolve application policy again.
//!
//! Export futures run under SDK suppression and a private diagnostic dispatch.
//! Only pinned metadata and a numeric rejection count classify failures; raw
//! transport/partial-success text never reaches an application sink. Failed SDK
//! results retain their error variant with redacted implementation-specific text.
//! Warning-only responses remain successful rather than fabricating rejection.
//! Fixed local stderr lines are coalesced per kind/process and never re-exported.
//!
//! Synchronous emission must not switch tracing dispatch from inside an event
//! callback. SDK suppression protects that path; shutdown also observes queue
//! drops. Provider shutdown uses the SDK's five-second budget, without an extra
//! force-flush. A returned acknowledgment is not delivery or worker-containment
//! proof; residual blocking export work can finish after a timeout.

use opentelemetry::trace::FutureExt as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::logs::{
    BatchConfig, BatchLogProcessor, LogBatch, LogExporter, LogProcessor, SdkLogRecord,
    SdkLoggerProvider,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::prelude::*;

pub(super) struct BatchPolicy(BatchConfig);

impl BatchPolicy {
    pub(super) fn resolve() -> Self {
        Self(BatchConfig::default())
    }
}

pub(super) fn provider<E: LogExporter + 'static>(
    resource: Resource,
    policy: BatchPolicy,
    exporter: E,
) -> SdkLoggerProvider {
    in_diagnostic_scope(|| {
        let processor = BatchLogProcessor::builder(SafeExporter(exporter))
            .with_batch_config(policy.0)
            .build();
        SdkLoggerProvider::builder()
            .with_resource(resource)
            .with_log_processor(SafeProcessor(processor))
            .build()
    })
}

pub(super) fn build(
    endpoint: &str,
    resource: Resource,
    policy: BatchPolicy,
) -> anyhow::Result<SdkLoggerProvider> {
    in_diagnostic_scope(|| {
        let exporter = anyhow::Context::context(
            opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build(),
            "failed to build OTLP log exporter",
        )?;
        Ok(provider(resource, policy, exporter))
    })
}

pub(super) fn setup_failure(_error: &anyhow::Error) {
    report(Failure::Setup);
}

#[derive(Clone, Copy)]
enum Failure {
    Setup,
    Export,
    Partial,
    QueueFull,
    Flush,
    Shutdown,
    PartialWarning,
}

static REPORTED: AtomicU8 = AtomicU8::new(0);

fn report(kind: Failure) {
    let bit = 1 << kind as u8;
    if REPORTED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        super::fallback(match kind {
            Failure::Setup => super::FallbackKind::LoggerSetup,
            Failure::Export => super::FallbackKind::LoggerExport,
            Failure::Partial => super::FallbackKind::LoggerPartial,
            Failure::QueueFull => super::FallbackKind::LoggerQueueFull,
            Failure::Flush => super::FallbackKind::LoggerFlush,
            Failure::Shutdown => super::FallbackKind::LoggerShutdown,
            Failure::PartialWarning => super::FallbackKind::LoggerPartialWarning,
        });
    }
}

struct SdkDiagnostics(Arc<AtomicBool>);

#[derive(Default)]
struct RejectionCount(i64);

impl tracing::field::Visit for RejectionCount {
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if field.name() == "rejected_log_records" {
            self.0 = value;
        }
    }
    fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {}
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SdkDiagnostics {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // Pinned diagnostic metadata identifies partial success without rendering
        // exporter error text or payload fields into any application sink.
        match event.metadata().name() {
            "TonicLogsClient.PartialSuccess" => {
                let mut rejected = RejectionCount::default();
                event.record(&mut rejected);
                if rejected.0 > 0 {
                    self.0.store(true, Ordering::Relaxed);
                    report(Failure::Partial);
                } else {
                    // OTLP can also return a warning with zero rejected records.
                    // Preserve that successful result without treating its raw
                    // explanatory text as safe local diagnostic content.
                    report(Failure::PartialWarning);
                }
            }
            "BatchLogProcessor.LogDroppingStarted" | "BatchLogProcessor.LogsDropped" => {
                report(Failure::QueueFull);
            }
            "BatchLogProcessor.Emit.AfterShutdown" => report(Failure::Shutdown),
            _ => {}
        }
    }
}

fn diagnostic_dispatch(partial: Arc<AtomicBool>) -> tracing::Dispatch {
    tracing::Dispatch::new(tracing_subscriber::registry().with(SdkDiagnostics(partial)))
}

fn in_diagnostic_scope<T>(operation: impl FnOnce() -> T) -> T {
    let dispatch = diagnostic_dispatch(Arc::new(AtomicBool::new(false)));
    tracing::dispatcher::with_default(&dispatch, || {
        let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
        operation()
    })
}

fn redact(result: OTelSdkResult) -> OTelSdkResult {
    result.map_err(|error| match error {
        // The SDK declares this text implementation-specific. Preserve failure
        // and its variant, but never give SDK diagnostics the transport payload.
        OTelSdkError::InternalFailure(_) => {
            OTelSdkError::InternalFailure("native log operation failed".to_owned())
        }
        error => error,
    })
}

#[derive(Debug)]
struct SafeExporter<E>(E);

impl<E: LogExporter> LogExporter for SafeExporter<E> {
    async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
        let partial = Arc::new(AtomicBool::new(false));
        let dispatch = diagnostic_dispatch(partial.clone());
        let context = opentelemetry::Context::current().with_telemetry_suppressed();
        let result = self
            .0
            .export(batch)
            .with_subscriber(dispatch)
            .with_context(context)
            .await;
        if result.is_err() {
            report(Failure::Export);
        }
        if result.is_ok() && partial.load(Ordering::Relaxed) {
            return Err(OTelSdkError::InternalFailure(
                "native log export partially rejected".to_owned(),
            ));
        }
        redact(result)
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        let result = in_diagnostic_scope(|| self.0.shutdown_with_timeout(timeout));
        if result.is_err() {
            report(Failure::Shutdown);
        }
        redact(result)
    }
    fn set_resource(&mut self, resource: &Resource) {
        in_diagnostic_scope(|| self.0.set_resource(resource));
    }
}

#[derive(Debug)]
struct SafeProcessor(BatchLogProcessor);

impl LogProcessor for SafeProcessor {
    fn emit(&self, record: &mut SdkLogRecord, scope: &opentelemetry::InstrumentationScope) {
        // Emission runs inside tracing's event callback: switching its dispatch
        // there would reborrow tracing TLS and panic. Suppress SDK observation
        // without changing dispatch; queue-drop diagnostics are also observed
        // at the ordinary shutdown boundary, outside the callback.
        let _scope = opentelemetry::Context::enter_telemetry_suppressed_scope();
        self.0.emit(record, scope);
    }
    fn force_flush(&self) -> OTelSdkResult {
        let result = in_diagnostic_scope(|| self.0.force_flush());
        if result.is_err() {
            report(Failure::Flush);
        }
        redact(result)
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        let result = in_diagnostic_scope(|| self.0.shutdown_with_timeout(timeout));
        if result.is_err() {
            report(Failure::Shutdown);
        }
        redact(result)
    }
    fn set_resource(&mut self, resource: &Resource) {
        in_diagnostic_scope(|| self.0.set_resource(resource));
    }
}

pub(super) fn shutdown(provider: &SdkLoggerProvider) {
    // Provider shutdown supplies the pinned five-second budget and drains its
    // queue. An extra unbounded pre-shutdown force_flush is not part of this path.
    if in_diagnostic_scope(|| provider.shutdown()).is_err() {
        report(Failure::Shutdown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::logs::LoggerProvider as _;
    use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
    use opentelemetry_sdk::logs::LogBatch;

    use opentelemetry_proto::tonic::collector::logs::v1::{
        ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
        logs_service_server::{LogsService, LogsServiceServer},
    };
    use opentelemetry_proto::tonic::common::v1::any_value::Value as ProtoValue;
    use std::sync::Mutex;

    #[derive(Clone, Copy)]
    enum Reply {
        Success,
        Partial,
        PartialWarning,
        Unavailable,
    }

    #[derive(Clone)]
    struct WireService {
        reply: Reply,
        received: Arc<Mutex<Vec<ExportLogsServiceRequest>>>,
    }

    #[tonic::async_trait]
    impl LogsService for WireService {
        async fn export(
            &self,
            request: tonic::Request<ExportLogsServiceRequest>,
        ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
            self.received
                .lock()
                .expect("wire records")
                .push(request.into_inner());
            match self.reply {
                Reply::Success => Ok(tonic::Response::new(ExportLogsServiceResponse {
                    partial_success: None,
                })),
                Reply::Partial => Ok(tonic::Response::new(ExportLogsServiceResponse {
                    partial_success: Some(ExportLogsPartialSuccess {
                        rejected_log_records: 1,
                        error_message: "synthetic-secret-payload".to_owned(),
                    }),
                })),
                Reply::PartialWarning => Ok(tonic::Response::new(ExportLogsServiceResponse {
                    partial_success: Some(ExportLogsPartialSuccess {
                        rejected_log_records: 0,
                        error_message: "synthetic-secret-payload".to_owned(),
                    }),
                })),
                Reply::Unavailable => Err(tonic::Status::unavailable("synthetic-secret-payload")),
            }
        }
    }

    struct WireServer {
        endpoint: String,
        received: Arc<Mutex<Vec<ExportLogsServiceRequest>>>,
        stop: tokio::sync::oneshot::Sender<()>,
        task: tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    }

    impl WireServer {
        fn start(reply: Reply) -> Self {
            let incoming = tonic::transport::server::TcpIncoming::bind(
                "127.0.0.1:0".parse().expect("loopback"),
            )
            .expect("owned listener");
            let endpoint = format!("http://{}", incoming.local_addr().expect("address"));
            let received = Arc::new(Mutex::new(Vec::new()));
            let service = WireService {
                reply,
                received: received.clone(),
            };
            let (stop, stopped) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(
                tonic::transport::Server::builder()
                    .add_service(LogsServiceServer::new(service))
                    .serve_with_incoming_shutdown(incoming, async {
                        stopped.await.expect("normal server stop");
                    }),
            );
            Self {
                endpoint,
                received,
                stop,
                task,
            }
        }
        async fn stop(self) {
            self.stop.send(()).expect("request normal shutdown");
            tokio::time::timeout(Duration::from_secs(5), self.task)
                .await
                .expect("server teardown deadline")
                .expect("server task")
                .expect("server shutdown");
        }
    }

    #[derive(Debug)]
    struct BatchCounts(Arc<Mutex<Vec<usize>>>);
    impl LogExporter for BatchCounts {
        async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
            self.0.lock().expect("counts").push(batch.iter().count());
            Ok(())
        }
    }

    #[test]
    fn native_logger_owned_snapshot_overrides_constructor_reads_and_preserves_sdk_input_policy() {
        const CHILD: &str = "JAUNDER_TEST_NATIVE_BATCH_CHILD";
        if let Some(mode) = std::env::var_os(CHILD) {
            use opentelemetry::logs::{LogRecord as _, Logger as _};
            let policy = if mode == "override" {
                // This complete SDK snapshot intentionally differs from the
                // child's inherited constructor defaults; no parent mutation.
                BatchPolicy(
                    opentelemetry_sdk::logs::BatchConfigBuilder::default()
                        .with_max_queue_size(2)
                        .with_max_export_batch_size(2)
                        .with_scheduled_delay(Duration::from_mins(1))
                        .build(),
                )
            } else {
                BatchPolicy::resolve()
            };
            let counts = Arc::new(Mutex::new(Vec::new()));
            let logger = provider(
                Resource::builder().build(),
                policy,
                BatchCounts(counts.clone()),
            );
            let log = logger.logger("batch snapshot");
            let offered = if mode == "defaults" || mode == "invalid" {
                513
            } else {
                2
            };
            for _ in 0..offered {
                let mut record = log.create_log_record();
                record.set_body("batch policy control".into());
                log.emit(record);
            }
            logger.force_flush().expect("snapshot flush");
            shutdown(&logger);
            let counts = counts.lock().expect("counts");
            if mode == "zero" {
                assert!(counts.is_empty());
            } else if mode == "defaults" || mode == "invalid" {
                assert_eq!(counts.iter().sum::<usize>(), 513);
                assert_eq!(counts[0], 512, "pinned default batch size");
            } else {
                assert_eq!(
                    &*counts,
                    &[2],
                    "full snapshot/clamping must own effective policy"
                );
            }
            return;
        }
        for mode in ["override", "zero", "clamp", "defaults", "invalid"] {
            let mut child =
                std::process::Command::new(std::env::current_exe().expect("executable"));
            child.args(["--exact", "telemetry::native_export::tests::native_logger_owned_snapshot_overrides_constructor_reads_and_preserves_sdk_input_policy", "--nocapture"]).env(CHILD, mode);
            for name in [
                "OTEL_BLRP_MAX_QUEUE_SIZE",
                "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
                "OTEL_BLRP_SCHEDULE_DELAY",
                "OTEL_BLRP_EXPORT_TIMEOUT",
            ] {
                child.env_remove(name);
            }
            match mode {
                "override" | "zero" => {
                    child
                        .env("OTEL_BLRP_MAX_QUEUE_SIZE", "0")
                        .env("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "0")
                        .env("OTEL_BLRP_SCHEDULE_DELAY", "0");
                }
                "clamp" => {
                    child
                        .env("OTEL_BLRP_MAX_QUEUE_SIZE", "2")
                        .env("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE", "99")
                        .env("OTEL_BLRP_SCHEDULE_DELAY", "60000");
                }
                "invalid" => {
                    child
                        .env("OTEL_BLRP_MAX_QUEUE_SIZE", "invalid")
                        .env(
                            "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
                            "999999999999999999999999999999999999999",
                        )
                        .env("OTEL_BLRP_SCHEDULE_DELAY", "invalid");
                }
                _ => {}
            }
            let output = child.output().expect("child");
            assert!(
                output.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[derive(Debug)]
    struct GatedExport {
        started: std::sync::mpsc::Sender<usize>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        counts: Arc<Mutex<Vec<usize>>>,
    }
    impl LogExporter for GatedExport {
        async fn export(&self, batch: LogBatch<'_>) -> OTelSdkResult {
            assert!(opentelemetry::Context::is_current_telemetry_suppressed());
            let count = batch.iter().count();
            self.counts.lock().expect("counts").push(count);
            self.started.send(count).expect("export started");
            self.release
                .lock()
                .expect("release")
                .recv_timeout(Duration::from_secs(5))
                .expect("controlled export release");
            Ok(())
        }
    }

    #[test]
    fn native_logger_default_queue_is_bounded_and_saturation_is_nonfatal_redacted_once() {
        const CHILD: &str = "JAUNDER_TEST_NATIVE_SATURATION_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let mut child =
                std::process::Command::new(std::env::current_exe().expect("executable"));
            child.args(["--exact", "telemetry::native_export::tests::native_logger_default_queue_is_bounded_and_saturation_is_nonfatal_redacted_once", "--nocapture"]).env(CHILD, "1");
            for name in [
                "OTEL_BLRP_MAX_QUEUE_SIZE",
                "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
                "OTEL_BLRP_SCHEDULE_DELAY",
                "OTEL_BLRP_EXPORT_TIMEOUT",
            ] {
                child.env_remove(name);
            }
            let output = child.output().expect("saturation child");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let (started, started_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let counts = Arc::new(Mutex::new(Vec::new()));
        let logger = provider(
            Resource::builder().build(),
            BatchPolicy::resolve(),
            GatedExport {
                started,
                release: Mutex::new(release_rx),
                counts: counts.clone(),
            },
        );
        let mut config = super::super::default_telemetry_config();
        config.otlp_endpoint = Some("native fixture".to_owned());
        let subscriber = super::super::compose_subscriber_with_logger(
            &config,
            None::<tracing_subscriber::layer::Identity>,
            None,
            None::<fn() -> std::io::Stdout>,
            Some(logger.logger("saturation")),
        );
        let (primary, diagnostic) = super::super::fallback_capture::capture(|| {
            tracing::subscriber::with_default(subscriber, || {
                for _ in 0..512 {
                    tracing::info!(target: "host::application", "bounded queue fixture");
                }
                assert_eq!(
                    started_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("first batch"),
                    512
                );
                for _ in 0..(2048 + 50) {
                    tracing::info!(target: "host::application", "bounded queue fixture");
                }
            });
            for _ in 0..5 {
                release.send(()).expect("release bounded batches");
            }
            shutdown(&logger);
            Ok::<_, &'static str>("application result preserved")
        });
        assert_eq!(primary, Ok("application result preserved"));
        assert_eq!(
            counts.lock().expect("counts").iter().sum::<usize>(),
            512 + 2048,
            "queue must not grow to retain overflowing records"
        );
        assert_eq!(
            diagnostic,
            "host.telemetry.logger_queue_full: native log queue full; records dropped\n"
        );
        assert!(diagnostic.len() <= 256);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_logger_real_unavailable_transport_is_failed_without_raw_payloads_or_recursion()
    {
        use opentelemetry::logs::{LogRecord as _, Logger as _};
        let server = WireServer::start(Reply::Unavailable);
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().expect("runtime");
        let logger = runtime
            .in_scope(|| {
                build(
                    &server.endpoint,
                    Resource::builder().build(),
                    BatchPolicy::resolve(),
                )
            })
            .expect("logger");
        let log = logger.logger("unavailable");
        for _ in 0..3 {
            let mut record = log.create_log_record();
            record.set_body("unavailable transport fixture".into());
            log.emit(record);
            assert!(
                !logger
                    .force_flush()
                    .expect_err("unavailable export must remain failed")
                    .to_string()
                    .contains("synthetic-secret")
            );
        }
        shutdown(&logger);
        assert_eq!(server.received.lock().expect("wire requests").len(), 3);
        drop(log);
        drop(logger);
        drop(runtime);
        server.stop().await;
    }

    #[derive(Debug)]
    struct ShutdownDeadlineProbe {
        started: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        stopped: std::sync::mpsc::Sender<()>,
    }
    impl LogExporter for ShutdownDeadlineProbe {
        async fn export(&self, _batch: LogBatch<'_>) -> OTelSdkResult {
            self.started.send(()).expect("started");
            self.release
                .lock()
                .expect("release")
                .recv_timeout(Duration::from_secs(10))
                .expect("release after bounded shutdown");
            Ok(())
        }
        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            self.stopped.send(()).expect("exporter stopped");
            Ok(())
        }
    }

    #[test]
    fn native_logger_shutdown_uses_sdk_five_second_budget_and_preserves_primary_result() {
        use opentelemetry::logs::{LogRecord as _, Logger as _};
        let (started, started_rx) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let (stopped, stopped_rx) = std::sync::mpsc::channel();
        let policy = BatchPolicy(
            opentelemetry_sdk::logs::BatchConfigBuilder::default()
                .with_max_queue_size(2)
                .with_max_export_batch_size(1)
                .with_scheduled_delay(Duration::from_mins(1))
                .build(),
        );
        let logger = provider(
            Resource::builder().build(),
            policy,
            ShutdownDeadlineProbe {
                started,
                release: Mutex::new(release_rx),
                stopped,
            },
        );
        let log = logger.logger("deadline");
        let mut record = log.create_log_record();
        record.set_body("shutdown deadline control".into());
        log.emit(record);
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("export in progress");
        let ((primary, failed), diagnostic) = super::super::fallback_capture::capture(|| {
            let primary = Ok::<_, &'static str>("application completion");
            let failed = logger.shutdown().expect_err("blocked export must time out");
            (primary, failed)
        });
        assert_eq!(primary, Ok("application completion"));
        assert!(
            failed.to_string().contains("5s"),
            "SDK default shutdown budget: {failed}"
        );
        assert_eq!(
            diagnostic,
            "host.telemetry.logger_shutdown: native log shutdown failed\n"
        );
        assert!(
            matches!(
                stopped_rx.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ),
            "shutdown return is not worker containment or delivery proof"
        );
        release.send(()).expect("release residual export");
        stopped_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("normal eventual exporter cleanup");
    }

    #[test]
    fn native_logger_setup_failure_preserves_error_type_and_does_not_restore_stdout() {
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().expect("runtime");
        let ((), diagnostic) = super::super::fallback_capture::capture(|| {
            for _ in 0..3 {
                let error = runtime
                    .in_scope(|| {
                        build(
                            "not a valid endpoint with synthetic-secret",
                            Resource::builder().build(),
                            BatchPolicy::resolve(),
                        )
                    })
                    .expect_err("invalid endpoint");
                assert!(
                    error
                        .downcast_ref::<opentelemetry_otlp::ExporterBuildError>()
                        .is_some()
                );
                setup_failure(&error);
            }
        });
        assert_eq!(
            diagnostic,
            "host.telemetry.logger_setup: native log export disabled\n"
        );
        assert!(diagnostic.len() <= 256);
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("capture").extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }

    fn emit_wire_fixture(sampled: bool) {
        use opentelemetry::trace::{
            SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
        };
        use tracing_opentelemetry::OpenTelemetrySpanExt as _;
        let request = tracing::info_span!(target: "jaunder::observability::http", "request", uri = "/safe", initial = 7, late = tracing::field::Empty);
        request
            .set_parent(
                opentelemetry::Context::new().with_remote_span_context(SpanContext::new(
                    TraceId::from_hex("0123456789abcdef0123456789abcdef").expect("trace"),
                    SpanId::from_hex("0123456789abcdef").expect("parent"),
                    if sampled {
                        TraceFlags::SAMPLED
                    } else {
                        TraceFlags::NOT_SAMPLED
                    },
                    true,
                    TraceState::default(),
                )),
            )
            .expect("parent");
        request.record("late", true);
        request.in_scope(|| {
            let span_trace = tracing_error::SpanTrace::capture().to_string();
            assert!(span_trace.contains("initial=7"), "{span_trace}");
            assert!(span_trace.contains("late=true"), "{span_trace}");
            tracing::trace!(target: "host::application", "trace/detail");
            tracing::debug!(target: "host::application", "debug/detail");
            tracing::info!(target: "host::application", "info/detail");
            tracing::warn!(target: "host::application", "warn/detail");
            tracing::error!(target: "host::application", { error.span_trace = "synthetic-raw-stack" }, "error/detail");
            tracing::info!(target: "host::application", parent: None, "root/detail");
        });
    }

    fn proto_value<'a>(
        attributes: &'a [opentelemetry_proto::tonic::common::v1::KeyValue],
        key: &str,
    ) -> Option<&'a ProtoValue> {
        attributes
            .iter()
            .find(|attribute| attribute.key == key)?
            .value
            .as_ref()?
            .value
            .as_ref()
    }

    fn assert_wire_records(requests: &[ExportLogsServiceRequest]) {
        use opentelemetry::trace::TraceId;
        let records: Vec<_> = requests
            .iter()
            .flat_map(|request| &request.resource_logs)
            .flat_map(|resource| &resource.scope_logs)
            .flat_map(|scope| &scope.log_records)
            .collect();
        assert_eq!(records.len(), 6);
        assert_eq!(
            records
                .iter()
                .map(|record| record.severity_number)
                .collect::<Vec<_>>(),
            [1, 5, 9, 13, 17, 9]
        );
        let bodies: Vec<_> = records
            .iter()
            .map(|record| {
                record
                    .body
                    .as_ref()
                    .and_then(|body| body.value.as_ref())
                    .expect("body")
                    .clone()
            })
            .collect();
        assert_eq!(
            bodies,
            [
                "trace/detail",
                "debug/detail",
                "info/detail",
                "warn/detail",
                "error/detail",
                "root/detail"
            ]
            .map(|value| ProtoValue::StringValue(value.to_owned()))
        );
        for record in &records[..5] {
            assert_eq!(
                record.trace_id,
                TraceId::from_hex("0123456789abcdef0123456789abcdef")
                    .expect("trace")
                    .to_bytes()
            );
            assert_eq!(record.span_id.len(), 8);
            assert_eq!(
                record.flags & 1,
                1,
                "AlwaysOn records clear sampled hints too"
            );
            assert_eq!(
                proto_value(&record.attributes, "jaunder.request.uri"),
                Some(&ProtoValue::StringValue("/safe".to_owned()))
            );
            assert!(proto_value(&record.attributes, "error.span_trace").is_none());
        }
        assert!(records[5].trace_id.is_empty() && records[5].span_id.is_empty());
        for resource in requests.iter().flat_map(|request| &request.resource_logs) {
            let attributes = &resource.resource.as_ref().expect("resource").attributes;
            assert_eq!(
                proto_value(attributes, "service.name"),
                Some(&ProtoValue::StringValue(
                    "shared-process-fixture".to_owned()
                ))
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_logger_real_subscriber_wire_records_ignore_local_policy_and_keep_context_diagnostics()
     {
        use opentelemetry::trace::TracerProvider as _;
        let server = WireServer::start(Reply::Success);
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().expect("runtime");
        for (verbose, policy) in [
            (false, Some("warn")),
            (false, Some("off")),
            (false, Some("host=warn,[context{enabled=true}]=off")),
            (false, None),
            (true, None),
        ] {
            for sampled in [false, true] {
                let resource = Resource::builder()
                    .with_attribute(opentelemetry::KeyValue::new(
                        "service.name",
                        "shared-process-fixture",
                    ))
                    .build();
                let logger = runtime
                    .in_scope(|| build(&server.endpoint, resource, BatchPolicy::resolve()))
                    .expect("logger");
                let tracer = super::super::tracer_provider_builder().build();
                let stdout = Captured::default();
                let diagnostic = Captured::default();
                let config = super::super::TelemetryConfig::from_raw(
                    verbose,
                    super::super::TelemetryRawConfig {
                        log_filter: Ok(policy.map(str::to_owned)),
                        rust_log: Ok(None),
                        log_format: Ok(None),
                        jaunder_otlp_endpoint: Ok(Some(server.endpoint.clone())),
                        otlp_endpoint: Ok(None),
                        slow_op_ms: Ok(None),
                        e2e_seed_process: Ok(None),
                    },
                );
                let extra = tracing_subscriber::fmt::layer()
                    .json()
                    .with_writer(diagnostic.clone())
                    .with_filter(tracing_subscriber::filter::LevelFilter::WARN);
                let subscriber = super::super::compose_subscriber_with_logger(
                    &config,
                    Some(extra),
                    Some(tracer.tracer("wire")),
                    Some(stdout.clone()),
                    Some(logger.logger("jaunder")),
                );
                tracing::subscriber::with_default(subscriber, || emit_wire_fixture(sampled));
                logger.force_flush().expect("delivered batch");
                shutdown(&logger);
                let requests: Vec<_> = server
                    .received
                    .lock()
                    .expect("wire requests")
                    .drain(..)
                    .collect();
                assert_wire_records(&requests);
                assert!(stdout.0.lock().expect("stdout").is_empty());
                let diag =
                    String::from_utf8(diagnostic.0.lock().expect("diag").clone()).expect("utf8");
                assert!(diag.contains("warn/detail") && diag.contains("error/detail"));
                assert!(!diag.contains("debug/detail") && !diag.contains("trace/detail"));
                drop(logger);
                drop(tracer);
            }
        }
        drop(runtime);
        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_logger_real_tonic_partial_success_remains_failed_and_redacted() {
        use opentelemetry::logs::{LogRecord as _, Logger as _};
        let server = WireServer::start(Reply::Partial);
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().expect("exporter runtime");
        let logger = runtime
            .in_scope(|| {
                build(
                    &server.endpoint,
                    Resource::builder().build(),
                    BatchPolicy::resolve(),
                )
            })
            .expect("real logger");
        let log = logger.logger("wire");
        for _ in 0..3 {
            let mut record = log.create_log_record();
            record.set_body("safe wire message".into());
            log.emit(record);
            let failure = logger
                .force_flush()
                .expect_err("partial rejection is not success");
            assert!(!failure.to_string().contains("synthetic-secret"));
        }
        shutdown(&logger);
        assert_eq!(
            server.received.lock().expect("requests").len(),
            3,
            "partial rejection must not be retried or recursively exported"
        );
        drop(log);
        drop(logger);
        drop(runtime);
        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_logger_real_tonic_warning_without_rejection_preserves_success() {
        use opentelemetry::logs::{LogRecord as _, Logger as _};
        let server = WireServer::start(Reply::PartialWarning);
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().expect("runtime");
        let policy = BatchPolicy::resolve();
        let logger = runtime
            .in_scope(|| build(&server.endpoint, Resource::builder().build(), policy))
            .expect("logger");
        let log = logger.logger("warning");
        let mut record = log.create_log_record();
        record.set_body("accepted with warning".into());
        log.emit(record);
        logger
            .force_flush()
            .expect("zero rejected records is not a fabricated export failure");
        shutdown(&logger);
        assert_eq!(server.received.lock().expect("requests").len(), 1);
        drop(log);
        drop(logger);
        drop(runtime);
        server.stop().await;
    }

    #[derive(Debug)]
    struct FailingTransport;
    impl LogExporter for FailingTransport {
        async fn export(&self, _batch: LogBatch<'_>) -> OTelSdkResult {
            tracing::warn!(name: "TonicLogsClient.PartialSuccess", target: "opentelemetry_otlp", error_message = "synthetic-secret-payload");
            Err(OTelSdkError::InternalFailure(
                "synthetic-secret-payload".to_owned(),
            ))
        }
        fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
            Err(OTelSdkError::InternalFailure(
                "synthetic-secret-shutdown".to_owned(),
            ))
        }
    }

    #[test]
    fn native_logger_repeated_export_failures_are_redacted_nonrecursive_and_nonfatal() {
        const CHILD: &str = "JAUNDER_TEST_NATIVE_FAILURE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let capture = std::env::var("JAUNDER_TEST_NATIVE_DIAG_PATH").expect("diagnostic path");
            let diag = std::fs::File::create(capture).expect("diagnostic file");
            let logger = provider(
                Resource::builder().build(),
                BatchPolicy::resolve(),
                FailingTransport,
            );
            let config = super::super::TelemetryConfig::from_raw(
                false,
                super::super::TelemetryRawConfig {
                    log_filter: Ok(Some("trace".to_owned())),
                    rust_log: Ok(None),
                    log_format: Ok(None),
                    jaunder_otlp_endpoint: Ok(Some("configured-native".to_owned())),
                    otlp_endpoint: Ok(None),
                    slow_op_ms: Ok(None),
                    e2e_seed_process: Ok(None),
                },
            );
            let extra = tracing_subscriber::fmt::layer()
                .json()
                .with_writer(diag)
                .with_filter(tracing_subscriber::filter::LevelFilter::WARN);
            let subscriber = super::super::compose_subscriber_with_logger(
                &config,
                Some(extra),
                None,
                Some(std::io::stdout),
                Some(logger.logger("failure")),
            );
            tracing::subscriber::set_global_default(subscriber).expect("subscriber");
            for _ in 0..3 {
                tracing::info!(target: "host::application", "ordinary native message");
                let failed = logger
                    .force_flush()
                    .expect_err("real export failure must remain failed");
                assert!(
                    !failed.to_string().contains("synthetic-secret"),
                    "raw exporter failure escaped redaction"
                );
            }
            shutdown(&logger);
            println!("cli result preserved");
            return;
        }
        let directory = tempfile::tempdir().expect("directory");
        let diag = directory.path().join("diag.log");
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "telemetry::native_export::tests::native_logger_repeated_export_failures_are_redacted_nonrecursive_and_nonfatal", "--nocapture"])
            .env(CHILD, "1").env("JAUNDER_TEST_NATIVE_DIAG_PATH", &diag).output().expect("child");
        assert!(
            output.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("stdout");
        let stderr = String::from_utf8(output.stderr).expect("stderr");
        assert!(stdout.contains("cli result preserved"));
        assert!(!stdout.contains("ordinary native message"));
        assert!(!stderr.contains("synthetic-secret"));
        assert_eq!(
            stderr
                .matches("host.telemetry.logger_export: native log export failed\n")
                .count(),
            1
        );
        assert_eq!(
            stderr
                .matches("host.telemetry.logger_shutdown: native log shutdown failed\n")
                .count(),
            1
        );
        assert!(stderr.lines().all(|line| line.len() < 256));
        assert!(
            !std::fs::read_to_string(diag)
                .expect("diagnostic capture")
                .contains("synthetic-secret")
        );
    }
}
