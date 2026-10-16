//! Process-global integration: actual host ownership, server capture, and stock
//! three-signal OTLP callbacks through the production shutdown order.
use super::*;
use opentelemetry_proto::tonic::collector::{logs::v1 as l, metrics::v1 as m, trace::v1 as t};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
const MARKER: &str = "WITHHELD_REMOTE_1743";
// The fake Collector is not application work. Keep its transport out of the
// application providers while retaining all of its local tracing callbacks.
thread_local! { static RESPONDER_SCOPE: std::cell::RefCell<Option<opentelemetry::ContextGuard>> = const { std::cell::RefCell::new(None) }; }
#[derive(Clone)]
struct Reply {
    mode: String,
    counts: Arc<[AtomicUsize; 3]>,
}
impl Reply {
    fn hit(&self, signal: usize) -> Result<(), tonic::Status> {
        assert!(
            self.counts[signal].fetch_add(1, Ordering::Relaxed) < 16,
            "recursive export"
        );
        if matches!(self.mode.as_str(), "reject" | "local-reject") {
            Err(tonic::Status::invalid_argument(MARKER))
        } else {
            Ok(())
        }
    }
    fn partial(&self) -> bool {
        matches!(self.mode.as_str(), "partial" | "warning" | "local-partial")
    }
    fn count(&self) -> i64 {
        i64::from(matches!(self.mode.as_str(), "partial" | "local-partial"))
    }
}
#[tonic::async_trait]
impl l::logs_service_server::LogsService for Reply {
    async fn export(
        &self,
        request: tonic::Request<l::ExportLogsServiceRequest>,
    ) -> Result<tonic::Response<l::ExportLogsServiceResponse>, tonic::Status> {
        // Transport callbacks must not feed back into native application logs.
        for resource in &request.get_ref().resource_logs {
            for scope in &resource.scope_logs {
                for record in &scope.log_records {
                    assert!(
                        !record.event_name.ends_with(".ExportFailed")
                            && !record.event_name.ends_with(".PartialSuccess"),
                        "remote SDK diagnostic fed back into native logs"
                    );
                    assert!(
                        !format!("{record:?}").contains(MARKER),
                        "remote detail in native output"
                    );
                }
            }
        }
        self.hit(0)?;
        Ok(tonic::Response::new(l::ExportLogsServiceResponse {
            partial_success: self.partial().then(|| l::ExportLogsPartialSuccess {
                rejected_log_records: self.count(),
                error_message: MARKER.into(),
            }),
        }))
    }
}
#[tonic::async_trait]
impl t::trace_service_server::TraceService for Reply {
    async fn export(
        &self,
        _: tonic::Request<t::ExportTraceServiceRequest>,
    ) -> Result<tonic::Response<t::ExportTraceServiceResponse>, tonic::Status> {
        self.hit(1)?;
        Ok(tonic::Response::new(t::ExportTraceServiceResponse {
            partial_success: self.partial().then(|| t::ExportTracePartialSuccess {
                rejected_spans: self.count(),
                error_message: MARKER.into(),
            }),
        }))
    }
}
#[tonic::async_trait]
impl m::metrics_service_server::MetricsService for Reply {
    async fn export(
        &self,
        request: tonic::Request<m::ExportMetricsServiceRequest>,
    ) -> Result<tonic::Response<m::ExportMetricsServiceResponse>, tonic::Status> {
        assert!(
            request
                .get_ref()
                .resource_metrics
                .iter()
                .flat_map(|r| &r.scope_metrics)
                .flat_map(|s| &s.metrics)
                .any(|metric| metric.name == "root.shutdown.counter"),
            "actual metric measurement required"
        );
        self.hit(2)?;
        Ok(tonic::Response::new(m::ExportMetricsServiceResponse {
            partial_success: self.partial().then(|| m::ExportMetricsPartialSuccess {
                rejected_data_points: self.count(),
                error_message: MARKER.into(),
            }),
        }))
    }
}
fn config(endpoint: Option<String>) -> TelemetryConfig {
    TelemetryConfig::from_raw(
        false,
        host::telemetry::TelemetryRawConfig {
            log_filter: Ok(Some("trace".into())),
            rust_log: Ok(None),
            log_format: Ok(Some("json".into())),
            jaunder_otlp_endpoint: Ok(endpoint),
            otlp_endpoint: Ok(None),
            slow_op_ms: Ok(None),
            e2e_seed_process: Ok(None),
        },
    )
}
fn run_child(mode: &str) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .on_thread_start(|| {
            RESPONDER_SCOPE.with(|scope| {
                *scope.borrow_mut() =
                    Some(opentelemetry::Context::enter_telemetry_suppressed_scope());
            });
        })
        .on_thread_stop(|| RESPONDER_SCOPE.with(|scope| drop(scope.borrow_mut().take())))
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let incoming =
            tonic::transport::server::TcpIncoming::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = incoming.local_addr().unwrap();
        let endpoint = format!("http://{address}");
        let counts = Arc::new(std::array::from_fn(|_| AtomicUsize::new(0)));
        let reply = Reply {
            mode: mode.to_owned(),
            counts: counts.clone(),
        };
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(
            tonic::transport::Server::builder()
                .add_service(l::logs_service_server::LogsServiceServer::new(
                    reply.clone(),
                ))
                .add_service(t::trace_service_server::TraceServiceServer::new(
                    reply.clone(),
                ))
                .add_service(m::metrics_service_server::MetricsServiceServer::new(reply))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                }),
        );
        let endpoint = if mode == "endpoint" {
            format!("http://user:{MARKER}@{address}/{MARKER}")
        } else {
            endpoint
        };
        let path = PathBuf::from(std::env::var_os("JAUNDER_TEST_ROOT_DIAG").unwrap());
        let previous = std::panic::take_hook();
        let local = mode.starts_with("local-");
        let guard = init_server_tracing(&config((!local).then(|| endpoint.clone())), Some(path));
        if !local {
            assert!(
                guard.otel_tracing_enabled(),
                "normal lifecycle rather than early setup fallback"
            );
        }
        {
            let span =
                tracing::info_span!(target:"host::root_test","root_shutdown",operation="safe");
            let _span = span.enter();
            tracing::info!(target:"host::root_test","safe primary operation");
            tracing::warn!(target:"host::root_test","safe local warning");
        }
        opentelemetry::global::meter("root-test")
            .u64_counter("root.shutdown.counter")
            .build()
            .add(1, &[]);
        if local {
            exercise_local_fmt(&endpoint);
        }
        drop(guard); // Endpoint modes use the actual meter -> tracer -> logger -> runtime path.
        std::panic::set_hook(previous);
        if mode != "endpoint" {
            assert!(
                counts.iter().all(|n| n.load(Ordering::Relaxed) > 0),
                "each signal must reach the responder"
            );
        }
        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        println!("PRIMARY_RESULT=17");
    });
}

// The actual host fmt route is absent with an endpoint. Public SDK providers
// are test stimulus for that local-only root, not an endpoint-mirroring claim.
fn exercise_local_fmt(endpoint: &str) {
    use opentelemetry::{
        logs::{LogRecord as _, Logger as _, LoggerProvider as _},
        metrics::MeterProvider as _,
        trace::{Tracer as _, TracerProvider as _},
    };
    use opentelemetry_otlp::WithExportConfig as _;
    let sdk = opentelemetry::Context::enter_telemetry_suppressed_scope();
    let logs = opentelemetry_sdk::logs::SdkLoggerProvider::builder()
        .with_batch_exporter(
            opentelemetry_otlp::LogExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .unwrap(),
        )
        .build();
    let traces = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(
            opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .unwrap(),
        )
        .build();
    let metrics = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_periodic_exporter(
            opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .unwrap(),
        )
        .build();
    drop(sdk);
    let logger = logs.logger("host::root_test");
    let mut record = logger.create_log_record();
    record.set_body("safe direct SDK fixture".into());
    logger.emit(record);
    drop(traces.tracer("local-test").start("safe direct span"));
    metrics
        .meter("local-test")
        .u64_counter("root.shutdown.counter")
        .build()
        .add(1, &[]);
    let _sdk = opentelemetry::Context::enter_telemetry_suppressed_scope();
    let _ = metrics.shutdown();
    let _ = traces.shutdown();
    let _ = logs.shutdown();
}

#[test]
fn actual_root_shutdown_and_sdk_remote_diagnostics_preserve_primary_result_and_capture_privacy() {
    const CHILD: &str = "JAUNDER_TEST_ROOT_SDK_MODE";
    if let Ok(mode) = std::env::var(CHILD) {
        run_child(&mode);
        return;
    }
    for mode in [
        "normal",
        "reject",
        "partial",
        "warning",
        "endpoint",
        "local-partial",
        "local-reject",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diag.log");
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","observability::initialization::sdk_diagnostics_tests::actual_root_shutdown_and_sdk_remote_diagnostics_preserve_primary_result_and_capture_privacy","--nocapture"]).env(CHILD,mode).env("JAUNDER_TEST_ROOT_DIAG",&path).output().unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let diag = std::fs::read_to_string(path).unwrap();
        assert!(output.status.success(), "{mode}: {stdout}\n{stderr}");
        assert!(stdout.contains("PRIMARY_RESULT=17"));
        if !mode.starts_with("local-") {
            assert!(
                !stdout.contains("safe primary operation"),
                "no stdout mirroring"
            );
        }
        if mode.starts_with("local-") {
            for client in ["Logs", "Traces", "Metrics"] {
                let name = format!(
                    "Tonic{client}Client.{}",
                    if mode == "local-partial" {
                        "PartialSuccess"
                    } else {
                        "ExportFailed"
                    }
                );
                assert!(
                    stdout.contains(&name),
                    "{mode}: genuine {name} missing from actual host fmt"
                );
            }
        }
        for text in [&stderr, &stdout, &diag] {
            assert!(!text.contains(MARKER), "{mode}: disclosed remote detail");
        }
        let rows: Vec<serde_json::Value> = diag
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            rows.iter()
                .any(|r| r["fields"]["message"] == "safe local warning")
        );
        if matches!(mode, "partial" | "warning" | "local-partial") {
            for (client, field) in [
                ("Logs", "rejected_log_records"),
                ("Traces", "rejected_spans"),
                ("Metrics", "rejected_data_points"),
            ] {
                let name = format!("Tonic{client}Client.PartialSuccess");
                let selected: Vec<_> = rows
                    .iter()
                    .filter(|r| r["fields"]["name"] == name)
                    .collect();
                assert!(
                    !selected.is_empty(),
                    "{mode}: actual {name} missing from owning diag reader"
                );
                for record in selected {
                    assert_eq!(record["level"], "WARN");
                    assert_eq!(record["target"], "opentelemetry-otlp");
                    assert_eq!(
                        record["fields"][field],
                        i64::from(matches!(mode, "partial" | "local-partial"))
                    );
                    assert_eq!(record["fields"]["error_message"], "remote detail withheld");
                    assert!(record.get("span").is_none());
                }
            }
        }
    }
}
