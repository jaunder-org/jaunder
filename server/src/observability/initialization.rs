//! Server tracing initialization composes host telemetry with the server-owned
//! scoped diagnostics layer and its independent panic hook.

use super::diagnostics;
use host::telemetry::{self, TelemetryConfig, TelemetryGuard};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::layer::Layer;

fn init_tracing_impl(config: &TelemetryConfig, diag_path: Option<PathBuf>) -> TelemetryGuard {
    // The composition root resolves capture once, then injects this leaf path into
    // the diagnostic layer and panic hook.
    let diag_log_layer = diag_path
        .as_deref()
        .and_then(diagnostics::open_diag_file_or_fallback)
        .map(|file| diagnostics::diag_layer(Arc::new(file)).boxed());

    let guard = telemetry::init_tracing_with_layer(config, diag_log_layer);

    // Install the scoped-diag panic hook (a no-op when disabled). It is
    // independent of the subscriber above and deliberately does not route
    // through it — see `diagnostics::install_diag_panic_hook` for the
    // deadlock-safety reasoning.
    diagnostics::install_diag_panic_hook(diag_path);

    guard
}

pub fn init_server_tracing(config: &TelemetryConfig, diag_path: Option<PathBuf>) -> TelemetryGuard {
    init_tracing_impl(config, diag_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::diagnostics::{assert_zero_error_metrics, with_process_globals};

    fn test_telemetry() -> host::telemetry::TelemetryConfig {
        host::telemetry::TelemetryConfig::from_raw(
            false,
            host::telemetry::TelemetryRawConfig {
                log_filter: Ok(None),
                rust_log: Ok(None),
                log_format: Ok(None),
                jaunder_otlp_endpoint: Ok(None),
                otlp_endpoint: Ok(None),
                slow_op_ms: Ok(None),
                e2e_seed_process: Ok(None),
            },
        )
    }

    #[test]
    fn init_tracing_impl_creates_diag_file_when_capture_is_configured() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        with_process_globals(|| {
            let path = dir.path().join("diag.log");
            let previous = std::panic::take_hook();
            init_tracing_impl(&test_telemetry(), Some(path.clone()));
            std::panic::set_hook(previous);
            assert!(path.exists(), "diag file should be created when configured");
        });
    }

    #[test]
    fn init_tracing_impl_survives_unopenable_diag_path() {
        const CHILD: &str = "JAUNDER_TEST_DIAG_OPEN_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let diag_path =
                host::capture::CaptureDirectory::from_raw(std::env::var_os(host::capture::DIR_ENV))
                    .expect("prepared capture directory")
                    .map(|directory| directory.path(host::capture::Stream::Diag));
            with_process_globals(|| {
                assert_zero_error_metrics(|| {
                    let guard = init_tracing_impl(&test_telemetry(), diag_path);
                    drop(guard);
                });
            });
            return;
        }

        let capture = tempfile::TempDir::new().expect("capture directory");
        std::fs::create_dir(capture.path().join("diag.log")).expect("diagnostic directory");
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .arg("--exact")
            .arg("observability::initialization::tests::init_tracing_impl_survives_unopenable_diag_path")
            .arg("--nocapture")
            .env(CHILD, "1")
            .env(host::capture::DIR_ENV, capture.path())
            .env_remove("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .output()
            .expect("run isolated diag-open test");
        assert!(
            output.status.success(),
            "child status: {}; stdout: {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout), // cov:ignore
            String::from_utf8_lossy(&output.stderr)  // cov:ignore
        );
        let stderr = String::from_utf8(output.stderr).expect("stderr utf8");
        assert_eq!(
            stderr.matches("server.observability.diag_log_open").count(),
            1,
            "stderr: {stderr}"
        );
    }
}
