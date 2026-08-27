//! Host e2e loop driver (#249): `cargo xtask e2e-local` OWNS the whole loop —
//! build the CSR bundle + server, start `jaunder serve` on an ephemeral port with
//! the VM's capture env, discover the port from the runtime file, seed via the
//! shared `devtool seed-e2e`, run Playwright against the discovered URL, and tear
//! the server down on every exit path. Each run gets a fresh temp storage dir + DB
//! (distinct ephemeral port + DB ⇒ concurrent runs don't collide at the server/DB
//! layer, and the dev `data/jaunder.db` is never touched). Loads the same
//! `playwright.config.ts` the CI VM loads, so "passes locally" == "passes in CI".
//! Host only.
//!
//! Canonical e2e-server env-var set the host driver and the flake both provide
//! (names shared, values per-environment; see also `flake.nix` `captureEnv`):
//! `JAUNDER_BIND`, `JAUNDER_DB`, `JAUNDER_RUNTIME_FILE`, `JAUNDER_CAPTURE_DIR`
//! (the single capture-dir contract, #227) — plus `JAUNDER_STORAGE_PATH`
//! host-side only (the VM instead relies on systemd
//! `WorkingDirectory=/var/lib/jaunder` + the `./data` default). Values differ per
//! environment (host: a temp dir + ephemeral port; VM: `/var/lib/jaunder` +
//! `:3000`). The DB + capture-dir vars are ALSO set on the Playwright process (with
//! `target/debug` prepended to PATH) so `mail.ts`/`websub.ts` resolve the same
//! capture paths (via `test-support capture-path`) the server writes, and
//! `seed.ts`'s bare-`test-support` `seedPostsViaTool` resolves the same binary +
//! DB — VM parity for the mail/websub/pagination specs.
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::{JoinHandle, sleep};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use xshell::{Shell, cmd};

use crate::git;
use crate::result::{CommandResult, StepResult};

/// Parse the server's `runtime.json` (`{"ip","port"}`, ADR-0035) into a base URL.
/// `None` on malformed JSON or a missing field — the caller keeps polling.
fn base_url_from_runtime(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let ip = v.get("ip")?.as_str()?;
    let port = v.get("port")?.as_u64()?;
    Some(format!("http://{ip}:{port}"))
}

/// Stream the server's stderr to the live terminal and the per-run verifier
/// input without accumulating the log in memory.
fn mirror_server_stderr(
    mut reader: impl Read,
    mut terminal: impl Write,
    mut capture: impl Write,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            terminal.flush()?;
            capture.flush()?;
            return Ok(());
        }
        terminal.write_all(&buffer[..read])?;
        terminal.flush()?;
        capture.write_all(&buffer[..read])?;
    }
}

/// Owns the spawned server and its stderr mirror. Stopping the child closes the
/// pipe, after which joining the mirror proves every byte reached its capture.
struct ServerChild {
    child: Option<Child>,
    stderr_mirror: Option<JoinHandle<std::io::Result<()>>>,
}

impl ServerChild {
    fn new(mut child: Child, capture: File) -> anyhow::Result<Self> {
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("jaunder serve stderr was not piped"))?;
        let stderr_mirror =
            std::thread::spawn(move || mirror_server_stderr(stderr, std::io::stderr(), capture));
        Ok(Self {
            child: Some(child),
            stderr_mirror: Some(stderr_mirror),
        })
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        let mut failures = Vec::new();
        if let Some(mut child) = self.child.take() {
            match child.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Err(error) = send_sigterm(&child, "jaunder serve") {
                        failures.push(format!("failed to stop jaunder serve: {error}"));
                    } else {
                        let deadline = Instant::now() + SERVER_SHUTDOWN_TIMEOUT;
                        loop {
                            match child.try_wait() {
                                Ok(Some(_)) => break,
                                Ok(None) if Instant::now() < deadline => {
                                    sleep(Duration::from_millis(25));
                                }
                                Ok(None) => {
                                    if let Err(error) = child.kill() {
                                        failures.push(format!(
                                            "failed to force-stop jaunder serve: {error}"
                                        ));
                                    }
                                    break;
                                }
                                Err(error) => {
                                    failures
                                        .push(format!("failed to inspect jaunder serve: {error}"));
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(error) => failures.push(format!("failed to inspect jaunder serve: {error}")),
            }
            if let Err(error) = child.wait() {
                failures.push(format!("failed to reap jaunder serve: {error}"));
            }
        }
        if let Some(mirror) = self.stderr_mirror.take() {
            match mirror.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("failed to mirror server stderr: {error}")),
                Err(_) => failures.push("server stderr mirror panicked".to_owned()),
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            anyhow::bail!(failures.join("; "))
        }
    }

    fn stop_for_drop_with(
        &mut self,
        stop: impl FnOnce(&mut Self) -> anyhow::Result<()>,
        stderr: &mut impl Write,
    ) {
        if stop(self).is_err() {
            let _ = writeln!(
                stderr,
                "xtask: warning: xtask.e2e.server_cleanup: ignored failure while stopping e2e-local server during drop"
            );
        }
    }
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        self.stop_for_drop_with(|server| server.stop(), &mut std::io::stderr());
    }
}
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

const COLLECTOR_READINESS_TIMEOUT: Duration = Duration::from_secs(5);
const COLLECTOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
const COLLECTOR_PORT_ATTEMPTS: usize = 8;

/// The host-local OTLP collector for one E2E lifecycle. URLs become available
struct CollectorGuard {
    child: Option<Child>,
    stderr_mirror: Option<JoinHandle<std::io::Result<()>>>,
    capture_dir: Option<tempfile::TempDir>,
    grpc_endpoint: SocketAddr,
    http_endpoint: SocketAddr,
    stderr_path: PathBuf,
}

struct CollectorStartError {
    error: anyhow::Error,
    capture_dir: tempfile::TempDir,
}

impl CollectorGuard {
    fn start_with_capture_dir(
        root: &Path,
        mut capture_dir: tempfile::TempDir,
    ) -> Result<Self, CollectorStartError> {
        let config = root.join("end2end/otel-collector.yaml");

        for attempt in 0..COLLECTOR_PORT_ATTEMPTS {
            let (grpc_endpoint, http_endpoint) = match collector_endpoints() {
                Ok(endpoints) => endpoints,
                Err(error) => return Err(CollectorStartError { error, capture_dir }),
            };
            let stderr_path = capture_dir.path().join("otelcol-contrib.stderr.log");
            let stderr =
                match File::create(&stderr_path).context("creating collector stderr capture") {
                    Ok(stderr) => stderr,
                    Err(error) => return Err(CollectorStartError { error, capture_dir }),
                };
            let mut command = Command::new("otelcol-contrib");
            command
                .arg("--config")
                .arg(&config)
                .env("OTELCOL_GRPC_ENDPOINT", grpc_endpoint.to_string())
                .env("OTELCOL_HTTP_ENDPOINT", http_endpoint.to_string())
                .env("JAUNDER_CAPTURE_DIR", capture_dir.path())
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::piped());
            let child = match command
                .spawn()
                .with_context(|| format!("starting otelcol-contrib with {}", config.display()))
            {
                Ok(child) => child,
                Err(error) => return Err(CollectorStartError { error, capture_dir }),
            };
            let mut guard = Self::from_child(
                child,
                stderr,
                capture_dir,
                grpc_endpoint,
                http_endpoint,
                stderr_path,
            );
            match guard.wait_until_ready() {
                Ok(()) => return Ok(guard),
                Err(_)
                    if guard.stderr_diagnostics().is_ok_and(|diagnostics| {
                        diagnostics.contains("address already in use")
                    }) && attempt + 1 < COLLECTOR_PORT_ATTEMPTS =>
                {
                    let _ = guard.kill_and_reap();
                    capture_dir = guard.take_capture_dir();
                }
                Err(error) => {
                    let diagnostics = guard
                        .stderr_diagnostics()
                        .unwrap_or_else(|_| "<unavailable>".to_owned());
                    let _ = guard.kill_and_reap();
                    return Err(CollectorStartError {
                        error: anyhow::anyhow!("{error}; collector stderr: {diagnostics}"),
                        capture_dir: guard.take_capture_dir(),
                    });
                }
            }
        }
        Err(CollectorStartError {
            error: anyhow::anyhow!(
                "otelcol-contrib could not acquire distinct loopback OTLP receiver endpoints"
            ),
            capture_dir,
        })
    }

    fn from_child(
        mut child: Child,
        stderr_capture: File,
        capture_dir: tempfile::TempDir,
        grpc_endpoint: SocketAddr,
        http_endpoint: SocketAddr,
        stderr_path: PathBuf,
    ) -> Self {
        let stderr = child
            .stderr
            .take()
            .expect("collector stderr is piped before construction");
        let stderr_mirror = std::thread::spawn(move || {
            mirror_server_stderr(stderr, std::io::stderr(), stderr_capture)
        });
        Self {
            child: Some(child),
            stderr_mirror: Some(stderr_mirror),
            capture_dir: Some(capture_dir),
            grpc_endpoint,
            http_endpoint,
            stderr_path,
        }
    }

    fn grpc_exporter_url(&self) -> String {
        format!("http://{}", self.grpc_endpoint)
    }

    fn browser_http_trace_url(&self) -> String {
        format!("http://{}/v1/traces", self.http_endpoint)
    }

    fn capture_dir(&self) -> &Path {
        self.capture_dir
            .as_ref()
            .map(tempfile::TempDir::path)
            .expect("collector capture directory is retained or live")
    }

    fn keep_capture_dir(&mut self) -> PathBuf {
        self.take_capture_dir().keep()
    }

    fn take_capture_dir(&mut self) -> tempfile::TempDir {
        self.capture_dir
            .take()
            .expect("collector capture directory is retained only once")
    }

    fn wait_until_ready(&mut self) -> anyhow::Result<()> {
        let deadline = Instant::now() + COLLECTOR_READINESS_TIMEOUT;
        loop {
            self.ensure_running()?;
            if receiver_listens(self.grpc_endpoint) && receiver_listens(self.http_endpoint) {
                self.ensure_running()?;
                return Ok(());
            }
            if Instant::now() >= deadline {
                anyhow::bail!(
                    "otelcol-contrib did not expose both OTLP receivers within {} seconds",
                    COLLECTOR_READINESS_TIMEOUT.as_secs()
                );
            }
            sleep(Duration::from_millis(25));
        }
    }

    fn shutdown(&mut self) -> anyhow::Result<()> {
        let mut child = self
            .child
            .take()
            .ok_or_else(|| anyhow::anyhow!("otelcol-contrib was already stopped"))?;
        if let Err(error) = send_sigterm(&child, "otelcol-contrib") {
            self.child = Some(child);
            return Err(error);
        }
        let deadline = Instant::now() + COLLECTOR_SHUTDOWN_TIMEOUT;
        loop {
            match child
                .try_wait()
                .context("checking otelcol-contrib during shutdown")?
            {
                Some(status) if status.success() => {
                    self.join_stderr_mirror()?;
                    return Ok(());
                }
                Some(status) => {
                    self.join_stderr_mirror()?;
                    anyhow::bail!(
                        "otelcol-contrib exited unsuccessfully during shutdown ({status}); collector stderr: {}",
                        self.stderr_diagnostics()?
                    );
                }
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    self.join_stderr_mirror()?;
                    anyhow::bail!("otelcol-contrib did not stop after SIGTERM");
                }
                None => sleep(Duration::from_millis(25)),
            }
        }
    }

    fn ensure_running(&mut self) -> anyhow::Result<()> {
        let status = self
            .child
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("otelcol-contrib was already stopped"))?
            .try_wait()
            .context("checking otelcol-contrib")?;
        if let Some(status) = status {
            self.join_stderr_mirror()?;
            anyhow::bail!(
                "otelcol-contrib exited before readiness ({status}); collector stderr: {}",
                self.stderr_diagnostics()?
            );
        }
        Ok(())
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

    fn join_stderr_mirror(&mut self) -> anyhow::Result<()> {
        if let Some(mirror) = self.stderr_mirror.take() {
            match mirror.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(error)) => anyhow::bail!("failed to mirror collector stderr: {error}"),
                Err(_) => anyhow::bail!("collector stderr mirror panicked"),
            }
        } else {
            Ok(())
        }
    }

    fn kill_and_reap(&mut self) -> anyhow::Result<()> {
        if let Some(mut child) = self.child.take() {
            if child.try_wait()?.is_none() {
                child.kill().context("killing otelcol-contrib")?;
            }
            child.wait().context("reaping otelcol-contrib")?;
        }
        self.join_stderr_mirror()
    }
}

impl Drop for CollectorGuard {
    fn drop(&mut self) {
        let _ = self.kill_and_reap();
    }
}

fn collector_endpoints() -> anyhow::Result<(SocketAddr, SocketAddr)> {
    let grpc = TcpListener::bind("127.0.0.1:0").context("allocating OTLP gRPC endpoint")?;
    let http = TcpListener::bind("127.0.0.1:0").context("allocating OTLP HTTP endpoint")?;
    Ok((
        grpc.local_addr().context("reading OTLP gRPC endpoint")?,
        http.local_addr().context("reading OTLP HTTP endpoint")?,
    ))
}

fn receiver_listens(endpoint: SocketAddr) -> bool {
    TcpStream::connect_timeout(&endpoint, Duration::from_millis(25)).is_ok()
}

fn send_sigterm(child: &Child, name: &str) -> anyhow::Result<()> {
    let pid = i32::try_from(child.id()).with_context(|| format!("converting {name} PID"))?;
    // SAFETY: `pid` names the child this guard spawned and SIGTERM carries no
    // pointer or memory-safety contract. The return code is checked immediately.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).with_context(|| format!("sending SIGTERM to {name}"))
    }
}

enum CaptureFinalization {
    Trace(PathBuf),
    Missing { run_dir: PathBuf, expected: PathBuf },
}

fn copy_capture_directory(source: &Path, destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
    for entry in fs::read_dir(source).with_context(|| format!("reading {}", source.display()))? {
        let entry = entry.with_context(|| format!("reading {}", source.display()))?;
        let destination_entry = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("reading {}", entry.path().display()))?;
        if file_type.is_dir() {
            copy_capture_directory(&entry.path(), &destination_entry)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &destination_entry).with_context(|| {
                format!(
                    "copying {} to {}",
                    entry.path().display(),
                    destination_entry.display()
                )
            })?;
        } else {
            anyhow::bail!("unsupported capture entry {}", entry.path().display());
        }
    }
    Ok(())
}

fn finalize_capture(
    root: &Path,
    browser: &str,
    source: &Path,
) -> anyhow::Result<CaptureFinalization> {
    let retained_root = root.join(".xtask/e2e-local");
    fs::create_dir_all(&retained_root)
        .with_context(|| format!("creating {}", retained_root.display()))?;
    let run_dir = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&retained_root)
        .context("creating unique retained e2e-local run directory")?
        .keep();
    let capture = run_dir.join(browser).join("capture");
    copy_capture_directory(source, &capture)?;
    let expected = capture.join("otel-traces.jsonl");
    if expected.is_file() {
        println!("e2e-local trace: {}", expected.display());
        Ok(CaptureFinalization::Trace(expected))
    } else {
        println!(
            "e2e-local capture retained at {}; expected trace missing: {}",
            run_dir.display(),
            expected.display()
        );
        Ok(CaptureFinalization::Missing { run_dir, expected })
    }
}
#[derive(Debug, Eq, PartialEq)]
struct PlaywrightInvocation {
    label: &'static str,
    args: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct BrowserLifecycle {
    browser: &'static str,
    invocations: Vec<PlaywrightInvocation>,
}

#[derive(Debug, Eq, PartialEq)]
struct E2eLocalPlan {
    release_csr: bool,
    update_visual_snapshots: bool,
    lifecycles: Vec<BrowserLifecycle>,
}
fn owned_args(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

fn normal_invocations(test_filter: Option<&str>) -> Vec<PlaywrightInvocation> {
    match test_filter {
        None => vec![PlaywrightInvocation {
            label: "ordinary",
            args: owned_args(&[
                "test",
                "--project",
                "chromium",
                "--project",
                "chromium-admin-site",
                "--project",
                "chromium-admin",
                "--reporter=html,line",
            ]),
        }],
        Some(filter) => vec![
            PlaywrightInvocation {
                label: "visual",
                args: owned_args(&[
                    "test",
                    "--project",
                    "chromium-visual",
                    "--no-deps",
                    "--pass-with-no-tests",
                    "--reporter=html,line",
                    filter,
                ]),
            },
            PlaywrightInvocation {
                label: "ordinary",
                args: owned_args(&[
                    "test",
                    "--project",
                    "chromium",
                    "--project",
                    "chromium-admin-site",
                    "--project",
                    "chromium-admin",
                    "--no-deps",
                    "--pass-with-no-tests",
                    "--reporter=html,line",
                    filter,
                ]),
            },
        ],
    }
}

fn resolve_e2e_env(
    root: &Path,
    workers: Result<String, env::VarError>,
    path: Option<OsString>,
    stderr: &mut impl Write,
) -> anyhow::Result<(String, OsString)> {
    let workers = match workers {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => "1".to_owned(),
        Err(env::VarError::NotUnicode(_)) => {
            let _ = writeln!(
                stderr,
                "xtask: warning: xtask.e2e.workers_config: ignored invalid-Unicode JAUNDER_E2E_WORKERS; using 1"
            );
            "1".to_owned()
        }
    };
    let inherited = path
        .ok_or(env::VarError::NotPresent)
        .map_err(|error| anyhow::Error::new(error).context("reading PATH for e2e-local"))?;
    let mut paths = Vec::new();
    paths.push(root.join("target/debug"));
    paths.extend(env::split_paths(&inherited));
    let path = env::join_paths(paths).context("prepending target/debug to e2e-local PATH")?;
    Ok((workers, path))
}

fn resolve_e2e_env_for_run(
    result: &mut CommandResult,
    root: &Path,
    workers: Result<String, env::VarError>,
    path: Option<OsString>,
    stderr: &mut impl Write,
) -> Option<(String, OsString)> {
    match resolve_e2e_env(root, workers, path, stderr) {
        Ok(values) => Some(values),
        Err(error) => {
            result.push(StepResult::fail("e2e-local-env").detail(format!(
                "cannot configure Playwright environment: {error:#}"
            )));
            None
        }
    }
}

fn e2e_local_plan(test_filter: Option<&str>, update_visual_snapshots: bool) -> E2eLocalPlan {
    if update_visual_snapshots {
        E2eLocalPlan {
            release_csr: true,
            update_visual_snapshots: true,
            lifecycles: ["chromium", "firefox"]
                .into_iter()
                .map(|browser| BrowserLifecycle {
                    browser,
                    invocations: vec![PlaywrightInvocation {
                        label: "visual",
                        args: owned_args(&[
                            "test",
                            "--project",
                            &format!("{browser}-visual"),
                            "--no-deps",
                            "--update-snapshots",
                            "--reporter=html,line",
                        ]),
                    }],
                })
                .collect(),
        }
    } else {
        E2eLocalPlan {
            release_csr: false,
            update_visual_snapshots: false,
            lifecycles: vec![BrowserLifecycle {
                browser: "chromium",
                invocations: normal_invocations(test_filter),
            }],
        }
    }
}

fn step_name(browser: &str, suffix: &str) -> String {
    format!("e2e-local-{browser}-{suffix}")
}

fn record_post_playwright_results(
    result: &mut CommandResult,
    browser: &str,
    playwright_result: Result<(), String>,
    playwright_duration: Duration,
    panic_gate_result: Result<(), String>,
    panic_gate_duration: Duration,
) {
    let playwright_step = step_name(browser, "playwright");
    match playwright_result {
        Ok(()) => result.push(StepResult::ok(&playwright_step).with_duration(playwright_duration)),
        Err(detail) => result.push(
            StepResult::fail(&playwright_step)
                .detail(detail)
                .with_duration(playwright_duration),
        ),
    }

    record_panic_gate_result(result, browser, panic_gate_result, panic_gate_duration);
}

fn record_panic_gate_result(
    result: &mut CommandResult,
    browser: &str,
    panic_gate_result: Result<(), String>,
    duration: Duration,
) {
    let panic_step = step_name(browser, "panic-gate");
    match panic_gate_result {
        Ok(()) => result.push(StepResult::ok(&panic_step).with_duration(duration)),
        Err(detail) => result.push(
            StepResult::fail(&panic_step)
                .detail(detail)
                .with_duration(duration),
        ),
    }
}

struct LifecycleVerification<'a> {
    browser: &'a str,
    test_support: &'a str,
    capture: &'a str,
    server_stderr: &'a Path,
}

fn record_collector_result(
    result: &mut CommandResult,
    browser: &str,
    phase: &str,
    collector_result: anyhow::Result<()>,
    duration: Duration,
) {
    let step = step_name(browser, phase);
    match collector_result {
        Ok(()) => result.push(StepResult::ok(&step).with_duration(duration)),
        Err(error) => result.push(
            StepResult::fail(&step)
                .detail(format!("otelcol-contrib {phase} failed: {error}"))
                .with_duration(duration),
        ),
    }
}

fn shutdown_collector(result: &mut CommandResult, browser: &str, collector: &mut CollectorGuard) {
    let shutdown_start = Instant::now();
    record_collector_result(
        result,
        browser,
        "collector-shutdown",
        collector.shutdown(),
        shutdown_start.elapsed(),
    );
}

fn finalize_collector_capture(
    result: &mut CommandResult,
    root: &Path,
    browser: &str,
    collector: &mut CollectorGuard,
) {
    let finalization_start = Instant::now();
    let step = step_name(browser, "capture");
    match finalize_capture(root, browser, collector.capture_dir()) {
        Ok(CaptureFinalization::Trace(trace)) => result.push(
            StepResult::ok(&step)
                .detail(format!("trace retained at {}", trace.display()))
                .with_duration(finalization_start.elapsed()),
        ),
        Ok(CaptureFinalization::Missing { run_dir, expected }) => result.push(
            StepResult::fail(&step)
                .detail(format!(
                    "collector produced no trace file; retained capture directory: {}; expected: {}",
                    run_dir.display(),
                    expected.display()
                ))
                .with_duration(finalization_start.elapsed()),
        ),
        Err(error) => {
            let source = collector.keep_capture_dir();
            result.push(
                StepResult::fail(&step)
                    .detail(format!(
                        "failed to retain collector capture: {error}; source retained at {}",
                        source.display()
                    ))
                    .with_duration(finalization_start.elapsed()),
            );
        }
    }
}

fn finalize_unowned_capture(
    result: &mut CommandResult,
    root: &Path,
    browser: &str,
    source: tempfile::TempDir,
) {
    let finalization_start = Instant::now();
    let step = step_name(browser, "capture");
    match finalize_capture(root, browser, source.path()) {
        Ok(CaptureFinalization::Trace(trace)) => result.push(
            StepResult::ok(&step)
                .detail(format!("trace retained at {}", trace.display()))
                .with_duration(finalization_start.elapsed()),
        ),
        Ok(CaptureFinalization::Missing { run_dir, expected }) => result.push(
            StepResult::fail(&step)
                .detail(format!(
                    "collector produced no trace file; retained capture directory: {}; expected: {}",
                    run_dir.display(),
                    expected.display()
                ))
                .with_duration(finalization_start.elapsed()),
        ),
        Err(error) => {
            let source = source.keep();
            result.push(
                StepResult::fail(&step)
                    .detail(format!(
                        "failed to retain collector capture: {error}; source retained at {}",
                        source.display()
                    ))
                    .with_duration(finalization_start.elapsed()),
            );
        }
    }
}

fn record_collector_start_failure(
    result: &mut CommandResult,
    root: &Path,
    browser: &str,
    failure: CollectorStartError,
    duration: Duration,
) {
    result.push(
        StepResult::fail(&step_name(browser, "collector"))
            .detail(format!(
                "otelcol-contrib readiness failed: {}",
                failure.error
            ))
            .with_duration(duration),
    );
    finalize_unowned_capture(result, root, browser, failure.capture_dir);
}

fn finish_server_setup_failure(
    result: &mut CommandResult,
    root: &Path,
    browser: &str,
    collector: &mut CollectorGuard,
) {
    shutdown_collector(result, browser, collector);
    finalize_collector_capture(result, root, browser, collector);
}

fn finish_lifecycle(
    sh: &Shell,
    result: &mut CommandResult,
    root: &Path,
    server: &mut ServerChild,
    collector: &mut CollectorGuard,
    verification: &LifecycleVerification<'_>,
    playwright_result: Option<(Result<(), String>, Duration)>,
) {
    let server_log_step = step_name(verification.browser, "server-log");
    if let Err(error) = server.stop() {
        result.push(
            StepResult::fail(&server_log_step)
                .detail(format!("failed to finalize server stderr capture: {error}")),
        );
    }

    shutdown_collector(result, verification.browser, collector);
    finalize_collector_capture(result, root, verification.browser, collector);

    let test_support = verification.test_support;
    let capture = verification.capture;
    let server_stderr = verification.server_stderr;
    let panic_gate_start = std::time::Instant::now();
    let panic_gate_result = cmd!(
        sh,
        "{test_support} verify-no-panics --capture-dir {capture} --server-log {server_stderr}"
    )
    .run()
    .map_err(|_| "shared zero-panic verifier failed".to_owned());
    let panic_gate_duration = panic_gate_start.elapsed();
    if let Some(playwright_result) = playwright_result {
        let (playwright_result, playwright_duration) = playwright_result;
        record_post_playwright_results(
            result,
            verification.browser,
            playwright_result,
            playwright_duration,
            panic_gate_result,
            panic_gate_duration,
        );
    } else {
        record_panic_gate_result(
            result,
            verification.browser,
            panic_gate_result,
            panic_gate_duration,
        );
    }
}

fn run_lifecycle(
    sh: &Shell,
    result: &mut CommandResult,
    root: &Path,
    lifecycle: &BrowserLifecycle,
    workers: &str,
    path: &OsString,
) {
    let browser = lifecycle.browser;
    let tmpdir_step = step_name(browser, "tmpdir");
    let server_log_step = step_name(browser, "server-log");
    let server_step = step_name(browser, "server");
    let seed_step = step_name(browser, "seed");

    // A distinct temp storage directory gives every browser a fresh database,
    // capture directory, runtime file, port, server, and teardown.
    let tmpdir_start = std::time::Instant::now();
    let Ok(storage) = tempfile::tempdir() else {
        result.push(
            StepResult::fail(&tmpdir_step)
                .detail("cannot create temp storage dir".to_owned())
                .with_duration(tmpdir_start.elapsed()),
        );
        return;
    };
    let sp = storage.path().display();
    let db = format!("sqlite:{sp}/jaunder.db");
    let runtime = storage.path().join("runtime.json");
    let collector_start = Instant::now();
    let collector_capture = match tempfile::tempdir() {
        Ok(capture) => capture,
        Err(error) => {
            result.push(
                StepResult::fail(&step_name(browser, "collector"))
                    .detail(format!(
                        "cannot create collector capture directory: {error}"
                    ))
                    .with_duration(collector_start.elapsed()),
            );
            return;
        }
    };
    let mut collector = match CollectorGuard::start_with_capture_dir(root, collector_capture) {
        Ok(collector) => {
            result.push(
                StepResult::ok(&step_name(browser, "collector"))
                    .with_duration(collector_start.elapsed()),
            );
            collector
        }
        Err(failure) => {
            record_collector_start_failure(
                result,
                root,
                browser,
                failure,
                collector_start.elapsed(),
            );
            return;
        }
    };
    let capture = collector.capture_dir().display().to_string();
    let server_stderr = collector.capture_dir().join("server-stderr.log");
    let server_log_start = std::time::Instant::now();
    let stderr_capture = match File::create(&server_stderr) {
        Ok(file) => file,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to create server stderr capture: {error}"))
                    .with_duration(server_log_start.elapsed()),
            );
            finish_server_setup_failure(result, root, browser, &mut collector);
            return;
        }
    };

    let server_start = std::time::Instant::now();
    let child = match Command::new(root.join("target/debug/jaunder"))
        .arg("serve")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_STORAGE_PATH", storage.path())
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_RUNTIME_FILE", &runtime)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .env("RUST_LOG", "info")
        .env(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            collector.grpc_exporter_url(),
        )
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            result.push(
                StepResult::fail(&server_step)
                    .detail(format!("failed to spawn jaunder serve: {error}"))
                    .with_duration(server_start.elapsed()),
            );
            finish_server_setup_failure(result, root, browser, &mut collector);
            return;
        }
    };
    let mut server = match ServerChild::new(child, stderr_capture) {
        Ok(server) => server,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to start server stderr capture: {error}"))
                    .with_duration(server_log_start.elapsed()),
            );
            finish_server_setup_failure(result, root, browser, &mut collector);
            return;
        }
    };

    let test_support = root.join("target/debug/test-support").display().to_string();
    let verification = LifecycleVerification {
        browser,
        test_support: &test_support,
        capture: &capture,
        server_stderr: &server_stderr,
    };
    let mut discovered = None;
    for _ in 0..30 {
        if let Ok(contents) = std::fs::read_to_string(&runtime)
            && let Some(url) = base_url_from_runtime(&contents)
            && cmd!(sh, "curl -sf {url}/").quiet().run().is_ok()
        {
            discovered = Some(url);
            break;
        }
        sleep(Duration::from_millis(500));
    }
    let Some(base_url) = discovered else {
        result.push(
            StepResult::fail(&server_step)
                .detail("server not reachable via runtime.json within 15s".to_owned())
                .with_duration(server_start.elapsed()),
        );
        finish_lifecycle(
            sh,
            result,
            root,
            &mut server,
            &mut collector,
            &verification,
            None,
        );
        return;
    };
    result.push(StepResult::ok(&server_step).with_duration(server_start.elapsed()));

    let tools = root.join("tools/Cargo.toml");
    let jaunder = root.join("target/debug/jaunder");
    let seed_start = std::time::Instant::now();
    if cmd!(
        sh,
        "cargo run --manifest-path {tools} -- seed-e2e --db {db} --test-support-bin {test_support} --jaunder-bin {jaunder}"
    )
    .env("JAUNDER_CAPTURE_DIR", &capture)
    .run()
    .is_err()
    {
        result.push(
            StepResult::fail(&seed_step)
                .detail("devtool seed-e2e failed".to_owned())
                .with_duration(seed_start.elapsed()),
        );
        finish_lifecycle(
            sh,
            result,
            root,
            &mut server,
            &mut collector,
            &verification,
            None,
        );
        return;
    }
    result.push(StepResult::ok(&seed_step).with_duration(seed_start.elapsed()));

    // Playwright uses the environment resolved before any subprocess. The DB,
    // capture directory, and target/debug-prefixed PATH match the VM contract.
    sh.change_dir(root.join("end2end"));
    let playwright_start = std::time::Instant::now();
    let mut playwright_result = Ok(());
    for invocation in &lifecycle.invocations {
        if cmd!(sh, "playwright")
            .args(&invocation.args)
            .env("JAUNDER_E2E_BASE_URL", &base_url)
            .env("JAUNDER_DB", &db)
            .env("JAUNDER_CAPTURE_DIR", &capture)
            .env(
                "JAUNDER_E2E_OTLP_HTTP_ENDPOINT",
                collector.browser_http_trace_url(),
            )
            .env("JAUNDER_E2E_WORKERS", workers)
            .env("PLAYWRIGHT_HTML_OPEN", "never")
            .env("PATH", path)
            .run()
            .is_err()
        {
            playwright_result = Err(format!(
                "{} Playwright invocation reported failures",
                invocation.label
            ));
            break;
        }
    }

    finish_lifecycle(
        sh,
        result,
        root,
        &mut server,
        &mut collector,
        &verification,
        Some((playwright_result, playwright_start.elapsed())),
    );
}

/// Build the served CSR bundle and binaries once, then execute each planned
/// browser against its own complete server/database/capture lifecycle.
pub fn run(
    sh: &Shell,
    result: &mut CommandResult,
    test_filter: Option<&str>,
    update_visual_snapshots: bool,
) {
    // Resolve the whole Playwright environment before any subprocess. Missing
    // PATH is a configuration failure, not a late empty command search.
    let env_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest directory has a workspace parent");
    let Some((workers, path)) = resolve_e2e_env_for_run(
        result,
        env_root,
        env::var("JAUNDER_E2E_WORKERS"),
        env::var_os("PATH"),
        &mut std::io::stderr(),
    ) else {
        return;
    };
    let plan = e2e_local_plan(test_filter, update_visual_snapshots);
    super::build_csr::run(sh, result, plan.release_csr);
    if !result.ok {
        return;
    }
    let root_start = std::time::Instant::now();
    let Ok(root) = git::toplevel(Path::new(".")) else {
        result.push(
            StepResult::fail("e2e-local")
                .detail("cannot locate repo root".to_owned())
                .with_duration(root_start.elapsed()),
        );
        return;
    };

    for (pkg, label) in [
        ("jaunder", "e2e-local-build-server"),
        ("test-support", "e2e-local-build-support"),
    ] {
        let build_start = std::time::Instant::now();
        if cmd!(sh, "cargo build -p {pkg}").run().is_err() {
            result.push(
                StepResult::fail(label)
                    .detail(format!("cargo build -p {pkg} failed"))
                    .with_duration(build_start.elapsed()),
            );
            return;
        }
        result.push(StepResult::ok(label).with_duration(build_start.elapsed()));
    }

    for lifecycle in &plan.lifecycles {
        run_lifecycle(sh, result, Path::new(&root), lifecycle, &workers, &path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;

    static COLLECTOR_PATH_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn base_url_from_runtime_reads_ip_and_port() {
        assert_eq!(
            base_url_from_runtime(r#"{"ip":"127.0.0.1","port":54312}"#).as_deref(),
            Some("http://127.0.0.1:54312"),
        );
    }

    #[test]
    fn base_url_from_runtime_rejects_malformed() {
        assert_eq!(base_url_from_runtime("not json"), None);
        assert_eq!(base_url_from_runtime(r#"{"ip":"127.0.0.1"}"#), None); // no port
    }

    fn owned(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn unfiltered_run_keeps_project_dependencies_enabled() {
        let plan = e2e_local_plan(None, false);
        assert!(!plan.update_visual_snapshots);
        assert!(!plan.release_csr);
        assert_eq!(plan.lifecycles.len(), 1);
        let lifecycle = &plan.lifecycles[0];
        assert_eq!(lifecycle.browser, "chromium");
        assert_eq!(lifecycle.invocations.len(), 1);
        assert_eq!(lifecycle.invocations[0].label, "ordinary");
        assert_eq!(
            lifecycle.invocations[0].args,
            owned(&[
                "test",
                "--project",
                "chromium",
                "--project",
                "chromium-admin-site",
                "--project",
                "chromium-admin",
                "--reporter=html,line",
            ])
        );
    }

    #[test]
    fn filtered_run_scopes_visual_and_ordinary_invocations() {
        let plan = e2e_local_plan(Some("auth.spec.ts"), false);
        let invocations = &plan.lifecycles[0].invocations;
        assert_eq!(
            invocations
                .iter()
                .map(|invocation| invocation.label)
                .collect::<Vec<_>>(),
            vec!["visual", "ordinary"]
        );
        assert_eq!(
            invocations[0].args,
            owned(&[
                "test",
                "--project",
                "chromium-visual",
                "--no-deps",
                "--pass-with-no-tests",
                "--reporter=html,line",
                "auth.spec.ts",
            ])
        );
        assert_eq!(
            invocations[1].args,
            owned(&[
                "test",
                "--project",
                "chromium",
                "--project",
                "chromium-admin-site",
                "--project",
                "chromium-admin",
                "--no-deps",
                "--pass-with-no-tests",
                "--reporter=html,line",
                "auth.spec.ts",
            ])
        );
    }

    #[test]
    fn visual_update_plan_uses_release_csr_and_fresh_browser_lifecycles() {
        let plan = e2e_local_plan(None, true);
        assert!(plan.release_csr);
        assert!(plan.update_visual_snapshots);
        assert_eq!(
            plan.lifecycles
                .iter()
                .map(|lifecycle| lifecycle.browser)
                .collect::<Vec<_>>(),
            vec!["chromium", "firefox"]
        );
        assert_eq!(
            plan.lifecycles
                .iter()
                .map(|lifecycle| &lifecycle.invocations[0].args)
                .collect::<Vec<_>>(),
            vec![
                &owned(&[
                    "test",
                    "--project",
                    "chromium-visual",
                    "--no-deps",
                    "--update-snapshots",
                    "--reporter=html,line",
                ]),
                &owned(&[
                    "test",
                    "--project",
                    "firefox-visual",
                    "--no-deps",
                    "--update-snapshots",
                    "--reporter=html,line",
                ]),
            ]
        );
    }

    #[test]
    fn stderr_mirror_copies_every_byte_to_both_sinks() {
        let input = b"first\n\xff panicked at src/x.rs:1:2: boom\n";
        let mut terminal = Vec::new();
        let mut capture = Vec::new();

        mirror_server_stderr(&input[..], &mut terminal, &mut capture).expect("mirror succeeds");

        assert_eq!(terminal, input);
        assert_eq!(capture, input);
    }

    #[test]
    fn playwright_and_panic_failures_are_both_recorded() {
        let mut result = CommandResult::new("e2e-local");

        record_post_playwright_results(
            &mut result,
            "firefox",
            Err("visual Playwright invocation reported failures".to_owned()),
            Duration::from_millis(11),
            Err("shared verifier rejected a panic".to_owned()),
            Duration::from_millis(7),
        );

        assert!(!result.ok);
        let playwright = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-firefox-playwright")
            .expect("playwright step");
        let panic_gate = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-firefox-panic-gate")
            .expect("panic step");
        assert!(!playwright.ok);
        assert!(!panic_gate.ok);
        assert_eq!(playwright.duration_ms, 11);
        assert_eq!(panic_gate.duration_ms, 7);
        assert_eq!(
            panic_gate.detail.as_deref(),
            Some("shared verifier rejected a panic")
        );
    }

    #[test]
    fn clean_post_playwright_results_are_both_successful() {
        let mut result = CommandResult::new("e2e-local");
        record_post_playwright_results(
            &mut result,
            "chromium",
            Ok(()),
            Duration::from_millis(11),
            Ok(()),
            Duration::from_millis(7),
        );
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| {
                    step.name == "e2e-local-chromium-playwright"
                        || step.name == "e2e-local-chromium-panic-gate"
                })
                .filter(|step| step.ok)
                .count(),
            2
        );
    }

    #[test]
    fn e2e_env_invalid_unicode_workers_warns_once_and_keeps_default() {
        let mut stderr = Vec::new();
        let mut result = CommandResult::new("e2e-local");
        let before = serde_json::to_string(&result).unwrap();
        let (workers, path) = resolve_e2e_env_for_run(
            &mut result,
            Path::new("/repo"),
            Err(env::VarError::NotUnicode(OsString::from("sensitive"))),
            Some(OsString::from("/usr/bin")),
            &mut stderr,
        )
        .unwrap();
        assert_eq!(serde_json::to_string(&result).unwrap(), before);
        assert_eq!(workers, "1");
        assert_eq!(
            env::split_paths(&path).collect::<Vec<_>>(),
            vec![
                PathBuf::from("/repo/target/debug"),
                PathBuf::from("/usr/bin")
            ]
        );
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.e2e.workers_config").count(), 1);
        assert!(!warning.contains("sensitive"));
    }
    #[test]
    fn e2e_env_prefix_is_workspace_root_even_for_subdirectory_invocation() {
        let subdirectory_cwd = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        assert_ne!(subdirectory_cwd, workspace_root);
        let mut result = CommandResult::new("e2e-local");
        let (_, path) = resolve_e2e_env_for_run(
            &mut result,
            workspace_root,
            Err(env::VarError::NotPresent),
            Some(OsString::from("/usr/bin")),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            env::split_paths(&path).next().unwrap(),
            workspace_root.join("target/debug")
        );
        assert!(result.ok);
    }

    #[cfg(unix)]
    #[test]
    fn e2e_env_non_unicode_path_is_preserved_and_absence_is_typed() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let inherited = OsString::from_vec(vec![b'/', b'x', 0xff]);
        let mut stderr = Vec::new();
        let mut result = CommandResult::new("e2e-local");
        let (_, path) = resolve_e2e_env_for_run(
            &mut result,
            Path::new("/repo"),
            Err(env::VarError::NotPresent),
            Some(inherited.clone()),
            &mut stderr,
        )
        .unwrap();
        let paths = env::split_paths(&path).collect::<Vec<_>>();
        assert_eq!(
            paths[1].as_os_str().as_bytes(),
            inherited.as_os_str().as_bytes()
        );
        assert!(stderr.is_empty());

        let mut owner = CommandResult::new("e2e-local");
        assert!(
            resolve_e2e_env_for_run(
                &mut owner,
                Path::new("/repo"),
                Err(env::VarError::NotPresent),
                None,
                &mut Vec::new(),
            )
            .is_none()
        );
        assert!(!owner.ok);
        let owner_json = serde_json::to_string(&owner).unwrap();
        assert!(
            owner_json.contains("reading PATH for e2e-local"),
            "{owner_json}"
        );

        let error = resolve_e2e_env(
            Path::new("/repo"),
            Err(env::VarError::NotPresent),
            None,
            &mut Vec::new(),
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<env::VarError>(),
            Some(env::VarError::NotPresent)
        ));
    }

    #[test]
    fn ancillary_warning_server_drop_failure_preserves_primary_result() {
        let child = Command::new("sleep")
            .arg("60")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let capture = tempfile::tempfile().expect("capture file");
        let mut guard = ServerChild::new(child, capture).expect("server guard");
        let primary = CommandResult::new("e2e-local");
        let before = serde_json::to_string(&primary).unwrap();
        let mut stderr = Vec::new();
        guard.stop_for_drop_with(|_| anyhow::bail!("sensitive stop failure"), &mut stderr);
        assert_eq!(serde_json::to_string(&primary).unwrap(), before);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.e2e.server_cleanup").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
        drop(guard);
    }

    #[test]
    fn server_child_kills_on_drop() {
        let child = Command::new("sleep")
            .arg("60")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn sleep");
        let capture = tempfile::tempfile().expect("capture file");
        let pid = child.id();
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let guard = ServerChild::new(child, capture).expect("server guard");
        assert!(proc.exists(), "child should be alive before drop");
        drop(guard); // Drop stops and waits (reaps the zombie so /proc/<pid> clears)
        // Linux-only (xtask is host-only Linux): once stopped + reaped, /proc/<pid>
        // is gone. Zero-dependency liveness check — no external `kill` binary.
        assert!(!proc.exists(), "child must be reaped after drop");
    }

    #[test]
    fn server_child_stop_delivers_sigterm_and_reaps() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let sentinel = temp.path().join("graceful-stop");
        let child = Command::new("sh")
            .arg("-c")
            .arg("trap 'printf graceful > \"$1\"; exit 0' TERM; while :; do sleep 1; done")
            .arg("sh")
            .arg(&sentinel)
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn server");
        let pid = child.id();
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let capture = tempfile::tempfile().expect("capture file");
        let mut server = ServerChild::new(child, capture).expect("server guard");
        sleep(Duration::from_millis(50));

        server.stop().expect("graceful server shutdown");

        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "graceful");
        assert!(!proc.exists(), "server should be reaped");
    }
    fn test_collector(child: Child, grpc: SocketAddr, http: SocketAddr) -> CollectorGuard {
        let capture_dir = tempfile::tempdir().expect("capture directory");
        let stderr_path = capture_dir.path().join("collector.stderr");
        let stderr = File::create(&stderr_path).expect("collector stderr");
        CollectorGuard::from_child(child, stderr, capture_dir, grpc, http, stderr_path)
    }

    fn sleeping_collector() -> Child {
        Command::new("sleep")
            .arg("60")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn collector")
    }

    #[test]
    fn collector_readiness_requires_a_live_child_and_both_listening_receivers() {
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let mut guard = test_collector(
            sleeping_collector(),
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );

        guard
            .wait_until_ready()
            .expect("both receivers become ready");
        assert!(guard.grpc_exporter_url().starts_with("http://127.0.0.1:"));
        assert!(guard.browser_http_trace_url().ends_with("/v1/traces"));
    }

    #[test]
    fn collector_early_exit_reports_the_process_failure() {
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let child = Command::new("sh")
            .arg("-c")
            .arg("echo collector-startup-failure >&2; exit 17")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn failing collector");
        let mut guard = test_collector(
            child,
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );
        sleep(Duration::from_millis(50));

        let error = guard.wait_until_ready().expect_err("collector exits early");
        assert!(error.to_string().contains("collector-startup-failure"));
    }

    #[test]
    fn collector_shutdown_uses_sigterm_and_reaps_a_successful_exit() {
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let child = Command::new("sh")
            .arg("-c")
            .arg("trap 'exit 0' TERM; while :; do sleep 1; done")
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn collector");
        let pid = child.id();
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let mut guard = test_collector(
            child,
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );
        sleep(Duration::from_millis(50));

        guard.shutdown().expect("graceful collector shutdown");
        assert!(!proc.exists(), "collector should be reaped");
    }

    #[test]
    fn collector_drop_kills_and_reaps_an_unfinished_child() {
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let child = sleeping_collector();
        let pid = child.id();
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let guard = test_collector(
            child,
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );

        drop(guard);
        assert!(!proc.exists(), "collector must be reaped during drop");
    }

    #[test]
    fn simultaneous_collectors_start_with_distinct_endpoints_and_capture_directories() {
        let _path_lock = COLLECTOR_PATH_LOCK.lock().expect("collector path lock");
        let fake_collector_dir = tempfile::tempdir().expect("fake collector directory");
        let source = fake_collector_dir.path().join("collector.c");
        std::fs::write(
            &source,
            r#"
#include <arpa/inet.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static volatile sig_atomic_t running = 1;
static void stop(int signal) { (void)signal; running = 0; }
static int receiver(const char *endpoint) {
  char host[16]; unsigned short port;
  struct sockaddr_in address = { .sin_family = AF_INET };
  if (sscanf(endpoint, "%15[^:]:%hu", host, &port) != 2 ||
      inet_pton(AF_INET, host, &address.sin_addr) != 1) return -1;
  address.sin_port = htons(port);
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0 || bind(fd, (struct sockaddr *)&address, sizeof(address)) ||
      listen(fd, 4)) return -1;
  return fd;
}
int main(int argc, char **argv) {
  if (argc != 3 || strcmp(argv[1], "--config") ||
      !getenv("JAUNDER_CAPTURE_DIR")) return 2;
  int grpc = receiver(getenv("OTELCOL_GRPC_ENDPOINT"));
  int http = receiver(getenv("OTELCOL_HTTP_ENDPOINT"));
  if (grpc < 0 || http < 0) return 3;
  signal(SIGTERM, stop);
  while (running) pause();
  close(grpc); close(http);
  return 0;
}
"#,
        )
        .expect("write fake collector source");
        let collector = fake_collector_dir.path().join("otelcol-contrib");
        assert!(
            Command::new("cc")
                .arg(&source)
                .arg("-o")
                .arg(&collector)
                .status()
                .expect("compile fake collector")
                .success(),
            "fake collector must compile"
        );
        let old_path = env::var_os("PATH");
        let mut paths = vec![fake_collector_dir.path().to_owned()];
        if let Some(path) = &old_path {
            paths.extend(env::split_paths(path));
        }
        // SAFETY: this test serializes its short-lived PATH override, and restores
        // the process environment after both children have been spawned and reaped.
        unsafe { env::set_var("PATH", env::join_paths(paths).expect("valid PATH")) };

        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask has workspace parent");
        let mut first = CollectorGuard::start_with_capture_dir(
            workspace_root,
            tempfile::tempdir().expect("first capture"),
        )
        .map_err(|failure| failure.error)
        .expect("first collector starts");
        let mut second = CollectorGuard::start_with_capture_dir(
            workspace_root,
            tempfile::tempdir().expect("second capture"),
        )
        .map_err(|failure| failure.error)
        .expect("second collector starts");

        assert_ne!(first.grpc_exporter_url(), second.grpc_exporter_url());
        assert_ne!(
            first.browser_http_trace_url(),
            second.browser_http_trace_url()
        );
        assert_ne!(first.capture_dir(), second.capture_dir());

        first.shutdown().expect("first collector shuts down");
        second.shutdown().expect("second collector shuts down");
        // SAFETY: restore the environment mutation made for this boundary test.
        unsafe {
            match old_path {
                Some(path) => env::set_var("PATH", path),
                None => env::remove_var("PATH"),
            }
        }
    }
    #[test]
    fn collector_shutdown_failure_is_recorded_without_masking_playwright_failure() {
        let mut result = CommandResult::new("e2e-local");
        record_post_playwright_results(
            &mut result,
            "chromium",
            Err("ordinary Playwright invocation reported failures".to_owned()),
            Duration::from_millis(7),
            Ok(()),
            Duration::from_millis(3),
        );
        record_collector_result(
            &mut result,
            "chromium",
            "collector-shutdown",
            Err(anyhow::anyhow!("collector exited unsuccessfully")),
            Duration::from_millis(5),
        );

        assert!(!result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| !step.ok)
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "e2e-local-chromium-playwright",
                "e2e-local-chromium-collector-shutdown"
            ]
        );
    }
    #[test]
    fn finalize_capture_recursively_copies_trace_and_diagnostics() {
        let workspace = tempfile::tempdir().expect("workspace");
        let source = tempfile::tempdir().expect("capture source");
        let nested = source.path().join("nested");
        fs::create_dir(&nested).expect("nested capture directory");
        fs::write(source.path().join("otel-traces.jsonl"), "trace\n").expect("trace");
        fs::write(nested.join("collector.stderr.log"), "diagnostic\n").expect("diagnostic");

        let CaptureFinalization::Trace(trace) =
            finalize_capture(workspace.path(), "chromium", source.path()).expect("finalize")
        else {
            panic!("trace should be retained");
        };

        assert_eq!(fs::read_to_string(&trace).unwrap(), "trace\n");
        assert_eq!(
            fs::read_to_string(trace.parent().unwrap().join("nested/collector.stderr.log"))
                .unwrap(),
            "diagnostic\n"
        );
    }

    #[test]
    fn finalize_capture_reports_missing_trace_with_retained_run_directory() {
        let workspace = tempfile::tempdir().expect("workspace");
        let source = tempfile::tempdir().expect("capture source");
        fs::write(source.path().join("server-stderr.log"), "server\n").expect("server log");

        let CaptureFinalization::Missing { run_dir, expected } =
            finalize_capture(workspace.path(), "firefox", source.path()).expect("finalize")
        else {
            panic!("missing trace must fail finalization");
        };

        assert!(run_dir.is_dir());
        assert!(!expected.exists());
        assert_eq!(
            fs::read_to_string(run_dir.join("firefox/capture/server-stderr.log")).unwrap(),
            "server\n"
        );
    }

    #[test]
    fn finalization_failure_keeps_source_and_records_existing_failures() {
        let workspace = tempfile::tempdir().expect("workspace");
        fs::write(workspace.path().join(".xtask"), "not a directory").expect("blocking file");
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let mut collector = test_collector(
            sleeping_collector(),
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );
        fs::write(
            collector.capture_dir().join("otel-traces.jsonl"),
            "partial\n",
        )
        .expect("partial trace");
        let source = collector.capture_dir().to_owned();
        let mut result = CommandResult::new("e2e-local");
        record_post_playwright_results(
            &mut result,
            "chromium",
            Err("Playwright failed".to_owned()),
            Duration::from_millis(1),
            Ok(()),
            Duration::from_millis(1),
        );

        finalize_collector_capture(&mut result, workspace.path(), "chromium", &mut collector);

        let failures = result
            .steps
            .iter()
            .filter(|step| !step.ok)
            .map(|step| step.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            failures,
            vec![
                "e2e-local-chromium-playwright",
                "e2e-local-chromium-capture"
            ]
        );
        assert!(
            source.exists(),
            "failed finalization keeps the source capture"
        );
    }

    #[test]
    fn finalization_allocates_unique_retained_run_directories() {
        let workspace = tempfile::tempdir().expect("workspace");
        let first_source = tempfile::tempdir().expect("first source");
        let second_source = tempfile::tempdir().expect("second source");
        fs::write(first_source.path().join("otel-traces.jsonl"), "one\n").expect("first trace");
        fs::write(second_source.path().join("otel-traces.jsonl"), "two\n").expect("second trace");

        let CaptureFinalization::Trace(first) =
            finalize_capture(workspace.path(), "chromium", first_source.path()).expect("first")
        else {
            panic!("first trace should be retained");
        };
        let CaptureFinalization::Trace(second) =
            finalize_capture(workspace.path(), "chromium", second_source.path()).expect("second")
        else {
            panic!("second trace should be retained");
        };

        assert_ne!(
            first.parent().unwrap().parent(),
            second.parent().unwrap().parent()
        );
    }
    #[test]
    fn readiness_failure_retains_a_missing_trace_capture_in_the_workspace_area() {
        let workspace = tempfile::tempdir().expect("workspace");
        let capture = tempfile::tempdir().expect("capture");
        let mut result = CommandResult::new("e2e-local");

        record_collector_start_failure(
            &mut result,
            workspace.path(),
            "chromium",
            CollectorStartError {
                error: anyhow::anyhow!("collector exited before readiness"),
                capture_dir: capture,
            },
            Duration::from_millis(1),
        );

        let collector = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-chromium-collector")
            .expect("collector failure");
        let capture = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-chromium-capture")
            .expect("capture failure");
        assert!(!collector.ok);
        assert!(!capture.ok);
        let detail = capture.detail.as_deref().expect("capture detail");
        assert!(detail.contains(".xtask/e2e-local/run-"));
        assert!(detail.contains("otel-traces.jsonl"));
    }

    #[test]
    fn server_setup_failure_is_preserved_with_collector_finalization() {
        let workspace = tempfile::tempdir().expect("workspace");
        let grpc = TcpListener::bind("127.0.0.1:0").expect("gRPC listener");
        let http = TcpListener::bind("127.0.0.1:0").expect("HTTP listener");
        let mut collector = test_collector(
            sleeping_collector(),
            grpc.local_addr().expect("gRPC address"),
            http.local_addr().expect("HTTP address"),
        );
        fs::write(
            collector.capture_dir().join("otel-traces.jsonl"),
            "partial\n",
        )
        .expect("partial trace");
        let mut result = CommandResult::new("e2e-local");
        result.push(
            StepResult::fail("e2e-local-chromium-server").detail("server setup failed".to_owned()),
        );

        finish_server_setup_failure(&mut result, workspace.path(), "chromium", &mut collector);

        assert!(!result.ok);
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "e2e-local-chromium-server" && !step.ok)
        );
        assert!(
            result
                .steps
                .iter()
                .any(|step| step.name == "e2e-local-chromium-capture" && step.ok)
        );
    }
}
