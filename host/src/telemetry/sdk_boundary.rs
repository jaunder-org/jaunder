//! Public-API SDK self-observation isolation. This is not application retention.
//!
//! The bridge's dynamic per-layer guard rejects instrumentation born during SDK
//! work, independently of local diagnostics. Activation preserves that boundary
//! when SDK work enters an already-admitted application span. Context guards are
//! thread-local and qualified by the layer instance, not registry span IDs alone.
//!
//! Known limitation: callbacks identify spans, not individual entries. Repeated
//! entry of the same span with out-of-order exits can expose an interleaved
//! external context early, including losing suppression after that external
//! scope exits. The pinned bridge uses the same latest-ID exit behavior. This
//! is documented behavior, not a new API restriction or enforced ordering rule.

use std::cell::RefCell;
use std::sync::{Arc, OnceLock};

use opentelemetry::{Context as OtelContext, ContextGuard};
use tracing::{Dispatch, Subscriber, dispatcher::WeakDispatch, span::Id};
use tracing_subscriber::layer::{Context, Layer};

struct Entry {
    owner: Arc<()>,
    id: Id,
    guard: Option<ContextGuard>,
}

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static WORKER_SCOPE: RefCell<Option<ContextGuard>> = const { RefCell::new(None) };
}

/// Admitted tracing spans activate their `OTel` parent without clearing SDK scope.
pub(super) struct Activation {
    owner: Arc<()>,
    dispatch: OnceLock<WeakDispatch>,
}

impl Activation {
    pub(super) fn new() -> Self {
        Self {
            owner: Arc::new(()),
            dispatch: OnceLock::new(),
        }
    }
}

impl<S: Subscriber> Layer<S> for Activation {
    fn on_register_dispatch(&self, dispatch: &Dispatch) {
        // A weak reference avoids making the subscriber own itself.
        let _ = self.dispatch.set(dispatch.downgrade());
    }

    fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
        let suppressed = OtelContext::is_current_telemetry_suppressed();
        let context = self
            .dispatch
            .get()
            .and_then(WeakDispatch::upgrade)
            .and_then(|dispatch| tracing_opentelemetry::get_otel_context(id, &dispatch));
        let guard = context.map(|context| {
            if suppressed {
                context.with_telemetry_suppressed()
            } else {
                context
            }
            .attach()
        });
        // Even an entry with no bridge context needs a matching exit. Neither
        // context lookup nor guard destruction may run under the TLS borrow.
        ENTRIES.with(|entries| {
            entries.borrow_mut().push(Entry {
                owner: self.owner.clone(),
                id: id.clone(),
                guard,
            });
        });
    }

    fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
        let entry = ENTRIES.with(|entries| {
            let mut entries = entries.borrow_mut();
            let position = entries
                .iter()
                .rposition(|entry| Arc::ptr_eq(&entry.owner, &self.owner) && entry.id == *id);
            position.map(|position| entries.remove(position))
        });
        // Distinct IDs retain OTel's out-of-order guard support; repeated IDs
        // cannot identify which tracing entry exited (see module limitation).
        drop(entry.map(|entry| entry.guard));
    }
}

// Match the SDK providers' ordinary five-second shutdown budget, without
// turning exporter cleanup into another process input or retention control.
const EXPORTER_RUNTIME_SHUTDOWN_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Keeps exporter-only transport workers alive through both provider shutdowns.
/// Runtime destruction lives on an ordinary owner thread so dropping the
/// telemetry guard inside an application's async task does not drop a Tokio
/// runtime there. Shutdown joins that thread after a fixed best-effort budget.
/// Unfinished blocking work can survive until it returns or the process exits,
/// including when this guard is dropped while the process continues. Joining
/// the owner does not prove all work stopped; Tokio reports no timeout outcome.
pub(super) struct ExporterRuntime {
    handle: tokio::runtime::Handle,
    stop: Option<std::sync::mpsc::Sender<()>>,
    owner: Option<std::thread::JoinHandle<()>>,
}

fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("otel-transport")
        .on_thread_start(|| {
            let guard = OtelContext::enter_telemetry_suppressed_scope();
            WORKER_SCOPE.with(|scope| *scope.borrow_mut() = Some(guard));
        })
        .on_thread_stop(|| {
            let guard = WORKER_SCOPE.with(|scope| scope.borrow_mut().take());
            drop(guard);
        })
        .enable_all()
        .build()
}

fn run_owner(
    build: impl FnOnce() -> std::io::Result<tokio::runtime::Runtime>,
    ready: &std::sync::mpsc::SyncSender<std::io::Result<tokio::runtime::Handle>>,
    stopped: &std::sync::mpsc::Receiver<()>,
) {
    match build() {
        Ok(runtime) => {
            if ready.send(Ok(runtime.handle().clone())).is_ok() {
                let _ = stopped.recv();
            }
            // Tokio can leave uncancellable blocking work alive after
            // this returns; the accepted policy is bounded best effort.
            runtime.shutdown_timeout(EXPORTER_RUNTIME_SHUTDOWN_BUDGET);
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

impl ExporterRuntime {
    pub(super) fn new() -> std::io::Result<Self> {
        Self::with_owner(|ready, stopped| run_owner(build_runtime, &ready, &stopped))
    }

    // Inject only the owned startup operation; channel and join policy stay here.
    fn with_owner(
        operation: impl FnOnce(
            std::sync::mpsc::SyncSender<std::io::Result<tokio::runtime::Handle>>,
            std::sync::mpsc::Receiver<()>,
        ) + Send
        + 'static,
    ) -> std::io::Result<Self> {
        let (ready, receiver) = std::sync::mpsc::sync_channel(1);
        let (stop, stopped) = std::sync::mpsc::channel();
        let owner = std::thread::Builder::new()
            .name("otel-runtime-owner".into())
            .spawn(move || operation(ready, stopped))?;
        match receiver.recv() {
            Ok(Ok(handle)) => Ok(Self {
                handle,
                stop: Some(stop),
                owner: Some(owner),
            }),
            result => {
                if owner.join().is_err() {
                    super::fallback(super::FallbackKind::ExporterRuntimeShutdown);
                }
                match result {
                    Ok(Err(error)) => Err(error),
                    _ => Err(std::io::Error::other("exporter runtime owner unavailable")),
                }
            }
        }
    }

    pub(super) fn in_scope<R>(&self, build: impl FnOnce() -> R) -> R {
        // Ordinary exporters construct their lazy channel and spawn its buffer
        // worker on this entered runtime. Application work never gets its handle.
        let _entered = self.handle.enter();
        let _suppressed = OtelContext::enter_telemetry_suppressed_scope();
        build()
    }
}

impl Drop for ExporterRuntime {
    fn drop(&mut self) {
        drop(self.stop.take());
        if self.owner.take().is_some_and(|owner| owner.join().is_err()) {
            super::fallback(super::FallbackKind::ExporterRuntimeShutdown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exporter_runtime_builder_failure_preserves_io_error_and_joins_owner() {
        let (result, output) = super::super::fallback_capture::capture(|| {
            ExporterRuntime::with_owner(|ready, stopped| {
                run_owner(
                    || {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "synthetic-secret",
                        ))
                    },
                    &ready,
                    &stopped,
                );
            })
        });
        let error = result.err().expect("runtime build must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "synthetic-secret");
        assert!(output.is_empty(), "a build error is not an owner panic");
    }

    #[test]
    fn exporter_runtime_closed_ready_channel_is_unavailable_without_shutdown_warning() {
        let (result, output) = super::super::fallback_capture::capture(|| {
            ExporterRuntime::with_owner(|_ready, _stopped| {})
        });
        assert_eq!(
            result.err().expect("closed readiness").to_string(),
            "exporter runtime owner unavailable"
        );
        assert!(output.is_empty());
    }

    #[test]
    fn exporter_runtime_startup_panic_joins_and_reports_fixed_shutdown_fallback() {
        let (result, output) = super::super::tests::assert_runtime_fallback_is_nonrecursive(|| {
            super::super::fallback_capture::capture(|| {
                ExporterRuntime::with_owner(|_ready, _stopped| {
                    panic!("injected owner startup failure")
                })
            })
        });
        assert_eq!(
            result.err().expect("owner panic").to_string(),
            "exporter runtime owner unavailable"
        );
        assert_eq!(
            output,
            "host.telemetry.exporter_runtime_shutdown: telemetry shutdown failed\n"
        );
    }

    #[test]
    fn exporter_runtime_drop_joins_panicking_owner_with_nonrecursive_fallback() {
        let ((), output) = super::super::tests::assert_runtime_fallback_is_nonrecursive(|| {
            super::super::fallback_capture::capture(|| {
                let runtime = ExporterRuntime::with_owner(|ready, stopped| {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .expect("runtime");
                    ready.send(Ok(runtime.handle().clone())).expect("ready");
                    let _ = stopped.recv();
                    drop(runtime);
                    panic!("injected owner shutdown failure");
                })
                .expect("startup");
                drop(runtime);
            })
        });
        assert_eq!(
            output,
            "host.telemetry.exporter_runtime_shutdown: telemetry shutdown failed\n"
        );
        assert!(!OtelContext::is_current_telemetry_suppressed());
    }

    #[test]
    fn exporter_runtime_descendants_are_suppressed_without_application_leakage() {
        let runtime = ExporterRuntime::new().expect("export runtime");
        let (done, received) = std::sync::mpsc::channel();
        runtime.in_scope(|| {
            assert!(OtelContext::is_current_telemetry_suppressed());
            tokio::spawn(async move {
                assert!(OtelContext::is_current_telemetry_suppressed());
                tokio::task::yield_now().await;
                assert!(OtelContext::is_current_telemetry_suppressed());
                tokio::spawn(async {
                    tokio::task::yield_now().await;
                    assert!(OtelContext::is_current_telemetry_suppressed());
                })
                .await
                .expect("nested worker");
                tokio::task::spawn_blocking(|| {
                    assert!(OtelContext::is_current_telemetry_suppressed());
                })
                .await
                .expect("blocking worker");
                done.send(()).expect("done");
            });
        });
        assert!(!OtelContext::is_current_telemetry_suppressed());
        received
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("descendants completed");
        let app = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("app runtime");
        app.block_on(async {
            assert!(!OtelContext::is_current_telemetry_suppressed());
            tokio::spawn(async {
                assert!(!OtelContext::is_current_telemetry_suppressed());
            })
            .await
            .expect("application task");
            drop(runtime); // Must synchronously join without an async-context drop panic.
        });
        assert!(!OtelContext::is_current_telemetry_suppressed());
    }

    #[test]
    fn exporter_runtime_shutdown_budget_returns_before_owned_blocking_worker_release() {
        let runtime = ExporterRuntime::new().expect("runtime");
        let (release, blocked) = std::sync::mpsc::channel();
        let (started, start_received) = std::sync::mpsc::channel();
        let (completed, completion_received) = std::sync::mpsc::channel();
        let blocking = runtime.in_scope(|| {
            tokio::task::spawn_blocking(move || {
                started.send(()).expect("worker started");
                // Disconnect also releases the worker if the test unwinds.
                let _released = blocked.recv();
                completed.send(()).expect("worker completed");
            })
        });
        start_received
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("worker startup");
        let (finished, finish_received) = std::sync::mpsc::channel();
        let owner = std::thread::spawn(move || {
            drop(runtime);
            finished.send(()).expect("runtime shutdown returned");
        });
        // A bounded correctness watchdog, not a quiescent-host timing benchmark.
        // Always release and join before asserting so the red also cleans up.
        let stopped = finish_received.recv_timeout(std::time::Duration::from_secs(10));
        let still_blocked = completion_received.try_recv();
        release.send(()).expect("release owned worker");
        completion_received
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("released worker completed");
        owner.join().expect("shutdown owner joined");
        // The completion message precedes return; await the task itself as well
        // so the owned blocked-work proof leaves no lingering task behind.
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("cleanup runtime")
            .block_on(blocking)
            .expect("released blocking task joined");
        assert!(
            stopped.is_ok(),
            "shutdown must return before blocked work is released: {stopped:?}"
        );
        assert!(
            matches!(still_blocked, Err(std::sync::mpsc::TryRecvError::Empty)),
            "owner join must not be mistaken for blocking-worker completion"
        );
    }

    #[test]
    fn exporter_runtime_retains_ordinary_trace_and_metric_builders() {
        let runtime = ExporterRuntime::new().expect("runtime");
        runtime.in_scope(|| {
            use opentelemetry_otlp::WithExportConfig as _;
            // Lazy construction only: no Collector or delivery claim.
            let trace = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint("http://127.0.0.1:4317")
                .build()
                .expect("trace builder");
            let metric = opentelemetry_otlp::MetricExporter::builder()
                .with_tonic()
                .with_endpoint("http://127.0.0.1:4317")
                .build()
                .expect("metric builder");
            drop((trace, metric));
        });
        drop(runtime);
        assert!(!OtelContext::is_current_telemetry_suppressed());
    }
}
