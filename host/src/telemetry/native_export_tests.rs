//! Actual stock OTLP log responses, not forged SDK diagnostics.
use super::*;
use opentelemetry::logs::{LogRecord as _, Logger as _, LoggerProvider as _};
use opentelemetry_proto::tonic::collector::logs::v1::{
    ExportLogsPartialSuccess, ExportLogsServiceRequest, ExportLogsServiceResponse,
    logs_service_server::{LogsService, LogsServiceServer},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
struct Reply {
    rejected: Option<i64>,
    calls: Arc<Mutex<usize>>,
}
#[tonic::async_trait]
impl LogsService for Reply {
    async fn export(
        &self,
        _: tonic::Request<ExportLogsServiceRequest>,
    ) -> Result<tonic::Response<ExportLogsServiceResponse>, tonic::Status> {
        *self.calls.lock().unwrap() += 1;
        match self.rejected {
            Some(n) => Ok(tonic::Response::new(ExportLogsServiceResponse {
                partial_success: Some(ExportLogsPartialSuccess {
                    rejected_log_records: n,
                    error_message: "WITHHELD_REMOTE_DETAIL".into(),
                }),
            })),
            None => Err(tonic::Status::unavailable("WITHHELD_REMOTE_DETAIL")),
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stock_partial_rejection_and_warning_keep_success_and_transport_failure_stays_failed() {
    for rejected in [Some(1), Some(0), None] {
        let incoming =
            tonic::transport::server::TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let endpoint = format!("http://{}", incoming.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(0));
        let reply = Reply {
            rejected,
            calls: calls.clone(),
        };
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(LogsServiceServer::new(reply))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                }),
        );
        let runtime = super::super::sdk_boundary::ExporterRuntime::new().unwrap();
        let provider = runtime
            .in_scope(|| {
                build(
                    &endpoint,
                    Resource::builder_empty().build(),
                    BatchPolicy::resolve(),
                )
            })
            .unwrap();
        let log = provider.logger("stock fixture");
        let mut record = log.create_log_record();
        record.set_body("safe application message".into());
        log.emit(record);
        assert_eq!(provider.force_flush().is_ok(), rejected.is_some());
        shutdown(&provider);
        drop(log);
        drop(provider);
        drop(runtime);
        assert_eq!(*calls.lock().unwrap(), 1, "no retries or recursive export");
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

#[derive(Debug)]
struct GatedExport {
    started: std::sync::mpsc::Sender<usize>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    counts: Arc<Mutex<Vec<usize>>>,
}
impl opentelemetry_sdk::logs::LogExporter for GatedExport {
    async fn export(
        &self,
        batch: opentelemetry_sdk::logs::LogBatch<'_>,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        let count = batch.iter().count();
        self.counts.lock().unwrap().push(count);
        self.started.send(count).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        Ok(())
    }
}
#[test]
fn stock_default_queue_and_batch_are_bounded_without_changing_primary_result() {
    const CHILD: &str = "JAUNDER_TEST_STOCK_QUEUE";
    if std::env::var_os(CHILD).is_none() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child.args(["--exact","telemetry::native_export::tests::stock_default_queue_and_batch_are_bounded_without_changing_primary_result","--nocapture"]).env(CHILD,"1");
        for key in [
            "OTEL_BLRP_MAX_QUEUE_SIZE",
            "OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",
            "OTEL_BLRP_SCHEDULE_DELAY",
            "OTEL_BLRP_EXPORT_TIMEOUT",
        ] {
            child.env_remove(key);
        }
        let output = child.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (started, received) = std::sync::mpsc::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let counts = Arc::new(Mutex::new(Vec::new()));
    let logs = provider(
        Resource::builder_empty().build(),
        BatchPolicy::resolve(),
        GatedExport {
            started,
            release: Mutex::new(blocked),
            counts: counts.clone(),
        },
    );
    let logger = logs.logger("saturation");
    let offer = || {
        let mut record = logger.create_log_record();
        record.set_body("safe bounded fixture".into());
        logger.emit(record);
    };
    for _ in 0..512 {
        offer();
    }
    assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), 512);
    for _ in 0..(2048 + 50) {
        offer();
    }
    for _ in 0..5 {
        release.send(()).unwrap();
    }
    let primary = 17;
    shutdown(&logs);
    assert_eq!(primary, 17);
    assert_eq!(counts.lock().unwrap().iter().sum::<usize>(), 512 + 2048);
}

#[derive(Debug)]
struct BatchCounts(Arc<Mutex<Vec<usize>>>);
impl opentelemetry_sdk::logs::LogExporter for BatchCounts {
    async fn export(
        &self,
        batch: opentelemetry_sdk::logs::LogBatch<'_>,
    ) -> opentelemetry_sdk::error::OTelSdkResult {
        self.0.lock().unwrap().push(batch.iter().count());
        Ok(())
    }
}
#[test]
fn inherited_batch_policy_is_resolved_at_construction_and_snapshot_overrides_builder_defaults() {
    const CHILD: &str = "JAUNDER_TEST_STOCK_BATCH";
    if let Ok(mode) = std::env::var(CHILD) {
        let counts = Arc::new(Mutex::new(Vec::new()));
        let policy = if mode == "override" {
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
        let logs = provider(
            Resource::builder_empty().build(),
            policy,
            BatchCounts(counts.clone()),
        );
        let logger = logs.logger("batch fixture");
        for _ in 0..2 {
            let mut record = logger.create_log_record();
            record.set_body("safe batch fixture".into());
            logger.emit(record);
        }
        logs.force_flush().unwrap();
        shutdown(&logs);
        assert_eq!(
            *counts.lock().unwrap(),
            if mode == "zero" { vec![] } else { vec![2] }
        );
        return;
    }
    for mode in ["zero", "override", "clamp"] {
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","telemetry::native_export::tests::inherited_batch_policy_is_resolved_at_construction_and_snapshot_overrides_builder_defaults","--nocapture"])
            .env(CHILD,mode).env("OTEL_BLRP_MAX_QUEUE_SIZE",if mode=="clamp" {"2"}else{"0"}).env("OTEL_BLRP_MAX_EXPORT_BATCH_SIZE",if mode=="clamp" {"99"}else{"0"}).env("OTEL_BLRP_SCHEDULE_DELAY","60000").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn repeated_shutdown_failure_is_nonfatal_and_visible_without_coalescing_contract() {
    let provider = SdkLoggerProvider::builder().build();
    provider.shutdown().unwrap();
    let (result, output) = super::super::fallback_capture::capture(|| {
        shutdown(&provider);
        shutdown(&provider);
        17
    });
    assert_eq!(result, 17);
    assert_eq!(output.matches("native log shutdown failed").count(), 2);
}
