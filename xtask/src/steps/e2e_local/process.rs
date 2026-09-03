//! Processkit-backed supervision for the local E2E server and collector.

use std::fs::File;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::steps::process::Process;

use anyhow::Context as _;
use processkit::{Command, Outcome, StdioMode};

const COLLECTOR_READINESS_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// The Jaunder server lifecycle. Processkit drains the raw stderr tee before
/// shutdown resolves, so the panic verifier sees a complete server log.
pub(super) struct ServerProcess(Process);

impl ServerProcess {
    pub(super) fn start(command: Command, stderr: File) -> anyhow::Result<Self> {
        let stderr = tokio::fs::File::from_std(stderr);
        Ok(Self(Process::start(
            command
                .stderr_raw_tee(stderr)
                .on_stderr_line(|line| eprintln!("{line}")),
        )?))
    }
    /// Starting a processkit readiness probe also starts its background output
    /// pumps; without one, a chatty server can fill stderr while xtask waits via
    /// an external HTTP probe.
    pub(super) fn wait_for_path(&mut self, path: &Path, within: Duration) -> anyhow::Result<()> {
        self.0.wait_for_path(path, within)
    }

    pub(super) fn stop(&mut self) -> anyhow::Result<()> {
        self.0.shutdown(PROCESS_SHUTDOWN_TIMEOUT).map(|_| ())
    }

    pub(super) fn stopped(&self) -> bool {
        self.0.is_stopped()
    }
}

/// One collector and its temporary capture directory. Endpoint allocation and
/// artifact retention remain Jaunder policy; processkit owns containment,
/// readiness probing, graceful SIGTERM/escalation, and drop cleanup.
pub(super) struct CollectorGuard {
    process: Process,
    capture_dir: Option<tempfile::TempDir>,
    grpc_endpoint: SocketAddr,
    http_endpoint: SocketAddr,
    stderr_path: PathBuf,
}

pub(super) struct CollectorStartError {
    pub(super) error: anyhow::Error,
    pub(super) capture_dir: tempfile::TempDir,
    pub(super) stopped: bool,
    pub(super) retryable_bind_collision: bool,
}

impl CollectorGuard {
    pub(super) fn start_with_capture_dir(
        root: &Path,
        capture_dir: tempfile::TempDir,
        grpc_endpoint: SocketAddr,
        http_endpoint: SocketAddr,
    ) -> Result<Self, CollectorStartError> {
        let config = root.join("end2end/otel-collector.yaml");
        let stderr_path = capture_dir.path().join("otelcol-contrib.stderr.log");
        let stderr = match File::create(&stderr_path).context("creating collector stderr capture") {
            Ok(stderr) => stderr,
            Err(error) => {
                return Err(CollectorStartError {
                    error,
                    capture_dir,
                    stopped: true,
                    retryable_bind_collision: false,
                });
            }
        };
        let command = Command::new("otelcol-contrib")
            .arg("--config")
            .arg(&config)
            .env("OTELCOL_GRPC_ENDPOINT", grpc_endpoint.to_string())
            .env("OTELCOL_HTTP_ENDPOINT", http_endpoint.to_string())
            .stdin(processkit::Stdin::empty())
            .env("JAUNDER_CAPTURE_DIR", capture_dir.path())
            .stdout(StdioMode::Inherit)
            .stderr_raw_tee(tokio::fs::File::from_std(stderr))
            .on_stderr_line(|line| eprintln!("{line}"));
        let mut process = match Process::start(command)
            .with_context(|| format!("starting otelcol-contrib with {}", config.display()))
        {
            Ok(process) => process,
            Err(error) => {
                return Err(CollectorStartError {
                    error,
                    capture_dir,
                    stopped: true,
                    retryable_bind_collision: false,
                });
            }
        };
        if let Err(error) = process.wait_for_port(grpc_endpoint, COLLECTOR_READINESS_TIMEOUT) {
            return Err(collector_readiness_failure(
                process,
                error,
                capture_dir,
                &stderr_path,
            ));
        }
        if let Err(error) = process.wait_for_port(http_endpoint, COLLECTOR_READINESS_TIMEOUT) {
            return Err(collector_readiness_failure(
                process,
                error,
                capture_dir,
                &stderr_path,
            ));
        }
        Ok(Self {
            process,
            capture_dir: Some(capture_dir),
            grpc_endpoint,
            http_endpoint,
            stderr_path,
        })
    }

    pub(super) fn grpc_exporter_url(&self) -> String {
        format!("http://{}", self.grpc_endpoint)
    }
    pub(super) fn browser_http_trace_url(&self) -> String {
        format!("http://{}/v1/traces", self.http_endpoint)
    }
    pub(super) fn capture_dir(&self) -> &Path {
        self.capture_dir
            .as_ref()
            .expect("capture retained once")
            .path()
    }
    pub(super) fn take_capture_dir(&mut self) -> tempfile::TempDir {
        self.capture_dir.take().expect("capture retained once")
    }

    pub(super) fn shutdown(&mut self) -> anyhow::Result<()> {
        self.process
            .wait_for_port(self.grpc_endpoint, Duration::from_secs(1))
            .context("otelcol-contrib exited prematurely before shutdown")?;
        match self.process.shutdown(PROCESS_SHUTDOWN_TIMEOUT)? {
            Outcome::Exited(0) => Ok(()),
            outcome => anyhow::bail!(
                "otelcol-contrib exited unsuccessfully during shutdown ({outcome:?}); collector stderr: {}",
                self.stderr_diagnostics()?
            ),
        }
    }

    pub(super) fn stopped(&self) -> bool {
        self.process.is_stopped()
    }

    fn stderr_diagnostics(&self) -> anyhow::Result<String> {
        let text = std::fs::read_to_string(&self.stderr_path)
            .with_context(|| format!("reading {}", self.stderr_path.display()))?;
        Ok(if text.is_empty() {
            "<no stderr output>".to_owned()
        } else {
            text
        })
    }
}

fn collector_readiness_failure(
    mut process: Process,
    readiness: anyhow::Error,
    capture_dir: tempfile::TempDir,
    stderr_path: &Path,
) -> CollectorStartError {
    let cleanup = process.shutdown(Duration::ZERO).err();
    let stopped = process.is_stopped();
    let diagnostics = std::fs::read_to_string(stderr_path)
        .unwrap_or_else(|error| format!("<collector stderr unavailable: {error}>"));
    let retryable_bind_collision = is_bind_collision(&diagnostics);
    let error = match cleanup {
        Some(cleanup) => anyhow::anyhow!(
            "{readiness}; collector stderr: {diagnostics}; failed to clean up collector: {cleanup}"
        ),
        None => anyhow::anyhow!("{readiness}; collector stderr: {diagnostics}"),
    };
    CollectorStartError {
        error,
        capture_dir,
        stopped,
        retryable_bind_collision,
    }
}

fn is_bind_collision(diagnostics: &str) -> bool {
    diagnostics.contains("address already in use")
}

#[cfg(test)]
mod tests {
    use super::is_bind_collision;

    #[test]
    fn only_bind_diagnostics_are_retryable() {
        assert!(is_bind_collision("listen tcp: address already in use"));
        assert!(!is_bind_collision("collector configuration is invalid"));
    }
}
