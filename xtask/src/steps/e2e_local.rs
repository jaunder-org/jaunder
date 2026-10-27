//! Host e2e loop driver (#249): `cargo xtask e2e-local` OWNS the whole loop —
//! build the CSR bundle + server, start `jaunder serve` on an ephemeral port with
//! the VM's capture env, discover the port from the canonical
//! `<storage>/runtime.json`, seed via the shared `devtool seed-e2e`, run
//! Playwright against the discovered URL, and tear
//! the server down on every exit path. Each run gets a fresh temp storage dir + DB
//! (distinct ephemeral port + DB ⇒ concurrent runs don't collide at the server/DB
//! layer, and the dev `data/jaunder.db` is never touched). Loads the same
//! `playwright.config.ts` the CI VM loads, so "passes locally" == "passes in CI".
//! Host only.
//!
//! Canonical e2e-server env-var set the host driver and the flake both provide
//! (names shared, values per-environment; see also `nix/nixos.nix` `captureEnv`):
//! `JAUNDER_BIND`, `JAUNDER_DB`, `JAUNDER_CAPTURE_DIR` (the single capture-dir
//! contract, #227) — plus `JAUNDER_STORAGE_PATH` host-side only (the VM instead
//! relies on systemd `WorkingDirectory=/var/lib/jaunder` + the `./data` default).
//! Values differ per environment (host: a temp dir + ephemeral port; VM:
//! `/var/lib/jaunder` + `:3000`). The DB + capture-dir vars are ALSO set on the
//! Playwright process (with `target/debug` prepended to PATH) so
//! `mail.ts`/`websub.ts` resolve the same capture paths (via `test-support
//! capture-path`) the server writes, and `seed.ts`'s bare-`test-support`
//! `seedPostsViaTool` resolves the same binary + DB — VM parity for the
//! mail/websub/pagination specs.
mod capture;
mod process;

use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use self::capture::{RetainedCapture, allocate_retained_capture, finalize_capture};
use self::process::{CollectorGuard, CollectorStartError};
use anyhow::Context;
use xshell::{Shell, cmd};

use crate::cli::E2eLocalBrowser;
use crate::result::{CommandResult, StepResult};
use crate::steps::host_server::{
    HostArtifactConfig, HostArtifacts, HostServerSession, ServerSessionConfig, ServerStartPhase,
};
use crate::steps::host_signals::{ServerExit, ServerSignals};
use crate::steps::process::Process;

const COLLECTOR_PORT_ATTEMPTS: usize = 8;

/// Reserve distinct loopback addresses immediately before the collector binds.
/// A bind race is retried by the collector launcher rather than falling back to
/// shared fixed OTLP ports.
fn collector_endpoints() -> anyhow::Result<(SocketAddr, SocketAddr)> {
    let grpc = TcpListener::bind("127.0.0.1:0").context("allocating OTLP gRPC endpoint")?;
    let http = TcpListener::bind("127.0.0.1:0").context("allocating OTLP HTTP endpoint")?;
    Ok((
        grpc.local_addr().context("reading OTLP gRPC endpoint")?,
        http.local_addr().context("reading OTLP HTTP endpoint")?,
    ))
}
/// Advertise the loopback server through `localhost`, whose registrable domain
/// semantics let browser WebAuthn use it as an RP ID. The server remains bound
/// to `127.0.0.1`; only the browser-facing origin changes.
fn playwright_base_url(server_base_url: &str) -> String {
    let Ok(mut url) = url::Url::parse(server_base_url) else {
        return server_base_url.to_owned();
    };
    if url.host_str() == Some("127.0.0.1") && url.set_host(Some("localhost")).is_ok() {
        return url.as_str().trim_end_matches('/').to_owned();
    }
    server_base_url.to_owned()
}

fn start_collector(
    root: &Path,
    mut capture_dir: tempfile::TempDir,
) -> Result<CollectorGuard, CollectorStartError> {
    for attempt in 0..COLLECTOR_PORT_ATTEMPTS {
        let (grpc_endpoint, http_endpoint) = match collector_endpoints() {
            Ok(endpoints) => endpoints,
            Err(error) => {
                return Err(CollectorStartError {
                    error,
                    capture_dir,
                    stopped: true,
                    retryable_bind_collision: false,
                });
            }
        };
        match CollectorGuard::start_with_capture_dir(
            root,
            capture_dir,
            grpc_endpoint,
            http_endpoint,
        ) {
            Ok(collector) => return Ok(collector),
            Err(failure)
                if failure.stopped
                    && failure.retryable_bind_collision
                    && attempt + 1 < COLLECTOR_PORT_ATTEMPTS =>
            {
                capture_dir = failure.capture_dir;
            }
            Err(failure) => return Err(failure),
        }
    }
    unreachable!("the collector retry loop always returns")
}

fn new_trace_id() -> anyhow::Result<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    loop {
        let mut bytes = [0_u8; 16];
        File::open("/dev/urandom")
            .context("opening /dev/urandom for e2e trace id")?
            .read_exact(&mut bytes)
            .context("reading e2e trace id")?;
        if bytes.iter().all(|byte| *byte == 0) {
            continue;
        }
        let mut trace_id = String::with_capacity(32);
        for byte in bytes {
            trace_id.push(HEX[usize::from(byte >> 4)] as char);
            trace_id.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        return Ok(trace_id);
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

fn ordinary_projects(
    browser: E2eLocalBrowser,
    test_filter: Option<&str>,
) -> &'static [&'static str] {
    match (browser, test_filter.is_some()) {
        // Preserve the omitted-selector Chromium command, including its existing
        // dependency chain. A filtered run instead selects every Chromium project
        // so a visual, ordinary, or serialized admin test can all be found by its
        // positional Playwright filter.
        (E2eLocalBrowser::Chromium, false) => {
            &["chromium", "chromium-admin-site", "chromium-admin"]
        }
        (E2eLocalBrowser::Chromium, true) => &[
            "chromium-visual",
            "chromium",
            "chromium-admin-site",
            "chromium-admin",
        ],
        // Theme, admin-site, media, and invite tests are deliberately excluded
        // from Firefox's ordinary project, but remain runnable through its visual
        // and dependent projects when a local filter explicitly selects one.
        (E2eLocalBrowser::Firefox, false) => &["firefox"],
        (E2eLocalBrowser::Firefox, true) => &[
            "firefox-visual",
            "firefox",
            "firefox-admin-site",
            "firefox-admin",
        ],
        // WebKit likewise keeps its fast ordinary default. Its theme/admin
        // project joins filtered runs, so file:line filters cannot silently skip
        // the tests that `webkit` excludes.
        (E2eLocalBrowser::Webkit, false) => &["webkit"],
        (E2eLocalBrowser::Webkit, true) => &["webkit", "webkit-theme"],
    }
}

fn ordinary_invocation(
    browser: E2eLocalBrowser,
    test_filter: Option<&str>,
) -> PlaywrightInvocation {
    let mut args = owned_args(&["test"]);
    for project in ordinary_projects(browser, test_filter) {
        args.extend(["--project".to_owned(), (*project).to_owned()]);
    }
    if let Some(filter) = test_filter {
        args.extend(owned_args(&["--no-deps", "--reporter=html,line", filter]));
    } else {
        args.push("--reporter=html,line".to_owned());
    }
    PlaywrightInvocation {
        label: "ordinary",
        args,
    }
}

fn normal_invocations(
    browser: E2eLocalBrowser,
    test_filter: Option<&str>,
) -> Vec<PlaywrightInvocation> {
    vec![ordinary_invocation(browser, test_filter)]
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

fn e2e_local_plan(
    browser: Option<E2eLocalBrowser>,
    test_filter: Option<&str>,
    update_visual_snapshots: bool,
) -> E2eLocalPlan {
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
        let browser = browser.unwrap_or(E2eLocalBrowser::Chromium);
        E2eLocalPlan {
            release_csr: false,
            update_visual_snapshots: false,
            lifecycles: vec![BrowserLifecycle {
                browser: browser.as_str(),
                invocations: normal_invocations(browser, test_filter),
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
    sh: &'a Shell,
    browser: &'a str,
    test_support: &'a str,
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

fn shutdown_collector(
    result: &mut CommandResult,
    browser: &str,
    collector: &mut CollectorGuard,
) -> bool {
    let shutdown_start = Instant::now();
    let shutdown = collector.shutdown();
    let stopped = collector.stopped();
    record_collector_result(
        result,
        browser,
        "collector-shutdown",
        shutdown,
        shutdown_start.elapsed(),
    );
    stopped
}

fn record_capture_finalization(
    result: &mut CommandResult,
    browser: &str,
    retained: &RetainedCapture,
    source: tempfile::TempDir,
) -> PathBuf {
    let finalization_start = Instant::now();
    let step = step_name(browser, "capture");
    match finalize_capture(retained, source.path()) {
        Ok(true) => {
            result.push(
                StepResult::ok(&step)
                    .detail(format!(
                        "trace retained at {}",
                        retained.trace_path.display()
                    ))
                    .with_duration(finalization_start.elapsed()),
            );
            retained.capture_dir.clone()
        }
        Ok(false) => {
            result.push(
                StepResult::fail(&step)
                    .detail(format!(
                        "collector produced no trace file; retained capture directory: {}; expected: {}",
                        retained.run_dir.display(),
                        retained.trace_path.display()
                    ))
                    .with_duration(finalization_start.elapsed()),
            );
            retained.capture_dir.clone()
        }
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
            source
        }
    }
}

fn capture_writers_stopped(collector: bool, server: bool) -> bool {
    collector && server
}

fn stop_and_finalize_collector(
    result: &mut CommandResult,
    browser: &str,
    retained: &RetainedCapture,
    collector: &mut CollectorGuard,
    other_writers_stopped: bool,
) -> Option<PathBuf> {
    let stopped = shutdown_collector(result, browser, collector);
    let source = collector.take_capture_dir();
    if capture_writers_stopped(stopped, other_writers_stopped) {
        Some(record_capture_finalization(
            result, browser, retained, source,
        ))
    } else {
        let source = source.keep();
        result.push(
            StepResult::fail(&step_name(browser, "capture")).detail(format!(
                "capture writers could not be stopped; live capture source retained without copying at {}",
                source.display()
            )),
        );
        None
    }
}

fn record_capture_setup_failure(
    result: &mut CommandResult,
    browser: &str,
    retained: &RetainedCapture,
    error: &std::io::Error,
    duration: Duration,
) {
    result.push(
        StepResult::fail(&step_name(browser, "collector"))
            .detail(format!(
                "cannot create collector capture directory: {error}"
            ))
            .with_duration(duration),
    );
    result.push(
        StepResult::fail(&step_name(browser, "capture"))
            .detail(format!(
                "collector produced no trace file; retained capture directory: {}; expected: {}",
                retained.run_dir.display(),
                retained.trace_path.display()
            ))
            .with_duration(duration),
    );
}

fn record_collector_start_failure(
    result: &mut CommandResult,
    browser: &str,
    retained: &RetainedCapture,
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
    if failure.stopped {
        record_capture_finalization(result, browser, retained, failure.capture_dir);
    } else {
        let source = failure.capture_dir.keep();
        result.push(
            StepResult::fail(&step_name(browser, "capture")).detail(format!(
                "collector could not be stopped after startup failure; live capture source retained without copying at {}",
                source.display()
            )),
        );
    }
}

fn finish_server_setup_failure(
    result: &mut CommandResult,
    browser: &str,
    retained: &RetainedCapture,
    collector: &mut CollectorGuard,
) {
    stop_and_finalize_collector(result, browser, retained, collector, true);
}

fn finish_lifecycle(
    result: &mut CommandResult,
    server: &mut HostServerSession,
    collector: &mut CollectorGuard,
    retained: &RetainedCapture,
    verification: &LifecycleVerification<'_>,
    playwright_result: Option<(Result<(), String>, Duration)>,
    signals: &mut ServerSignals,
) {
    let server_log_step = step_name(verification.browser, "server-log");
    let mut shutdown_signal = None;
    let server_stopped = match server.stop_interruptible(async {
        shutdown_signal = Some(signals.next_signal_code().await);
    }) {
        Ok(()) => true,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to finalize server stderr capture: {error}")),
            );
            server.is_stopped()
        }
    };
    if result.exit_override.is_none() {
        result.exit_override = shutdown_signal;
    }

    let capture = stop_and_finalize_collector(
        result,
        verification.browser,
        retained,
        collector,
        server_stopped,
    );

    let panic_gate_start = Instant::now();
    let panic_gate_result = if let Some(capture) = capture {
        let sh = verification.sh;
        let test_support = verification.test_support;
        let server_stderr = capture.join("server-stderr.log");
        cmd!(
            sh,
            "{test_support} verify-no-panics --capture-dir {capture} --server-log {server_stderr}"
        )
        .run()
        .map_err(|_| "shared zero-panic verifier failed".to_owned())
    } else {
        Err(
            "zero-panic verification skipped because capture writers could not be stopped"
                .to_owned(),
        )
    };
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

fn devtool_seed_args(tools: &Path) -> [String; 6] {
    [
        "run".to_owned(),
        "--locked".to_owned(),
        "--manifest-path".to_owned(),
        tools.display().to_string(),
        "--".to_owned(),
        "seed-e2e".to_owned(),
    ]
}

fn devtool_seed_command<'a>(
    sh: &'a Shell,
    tools: &Path,
    db: &str,
    test_support: &str,
    jaunder: &Path,
    capture: &str,
    endpoint: &str,
) -> xshell::Cmd<'a> {
    let devtool_args = devtool_seed_args(tools);
    cmd!(
        sh,
        "cargo {devtool_args...} --db {db} --test-support-bin {test_support} --jaunder-bin {jaunder}"
    )
    .env("JAUNDER_CAPTURE_DIR", capture)
    .env("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
}

fn run_lifecycle(
    sh: &Shell,
    result: &mut CommandResult,
    artifacts: &HostArtifacts,
    lifecycle: &BrowserLifecycle,
    workers: &str,
    path: &OsString,
    signals: &mut ServerSignals,
) {
    let root = &artifacts.root;
    let browser = lifecycle.browser;
    let tmpdir_step = step_name(browser, "tmpdir");
    let server_log_step = step_name(browser, "server-log");
    let server_step = step_name(browser, "server");
    let seed_step = step_name(browser, "seed");
    let test_support_path = artifacts
        .test_support
        .as_deref()
        .expect("e2e-local requests the test-support artifact");
    let test_support = test_support_path.display().to_string();
    let retained = match allocate_retained_capture(root, browser, test_support_path) {
        Ok(retained) => retained,
        Err(error) => {
            result.push(
                StepResult::fail(&step_name(browser, "capture"))
                    .detail(format!("cannot allocate retained capture: {error:#}")),
            );
            return;
        }
    };

    // A distinct temp storage directory gives every browser a fresh database,
    // capture directory, runtime file, port, server, and teardown.
    let tmpdir_start = std::time::Instant::now();
    let storage = match tempfile::tempdir() {
        Ok(storage) => storage,
        Err(error) => {
            result.push(
                StepResult::fail(&tmpdir_step)
                    .detail(format!(
                        "cannot create temp storage dir: {error}; retained run: {}",
                        retained.run_dir.display()
                    ))
                    .with_duration(tmpdir_start.elapsed()),
            );
            result.push(
                StepResult::fail(&step_name(browser, "capture"))
                    .detail(format!(
                        "collector produced no trace file; retained capture directory: {}; expected: {}",
                        retained.run_dir.display(),
                        retained.trace_path.display()
                    ))
                    .with_duration(tmpdir_start.elapsed()),
            );
            return;
        }
    };
    let sp = storage.path().display();
    let db = format!("sqlite:{sp}/jaunder.db");
    let collector_start = Instant::now();
    let collector_capture = match tempfile::tempdir() {
        Ok(capture) => capture,
        Err(error) => {
            record_capture_setup_failure(
                result,
                browser,
                &retained,
                &error,
                collector_start.elapsed(),
            );
            return;
        }
    };
    let mut collector = match start_collector(root, collector_capture) {
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
                browser,
                &retained,
                failure,
                collector_start.elapsed(),
            );
            return;
        }
    };
    let trace_id = match new_trace_id() {
        Ok(trace_id) => trace_id,
        Err(error) => {
            result.push(
                StepResult::fail(&step_name(browser, "trace-context"))
                    .detail(format!("cannot create E2E trace context: {error:#}")),
            );
            finish_server_setup_failure(result, browser, &retained, &mut collector);
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
            finish_server_setup_failure(result, browser, &retained, &mut collector);
            return;
        }
    };

    let server_start = std::time::Instant::now();
    let config = ServerSessionConfig {
        shell: sh,
        jaunder: artifacts.jaunder.clone(),
        storage: storage.path().to_path_buf(),
        runtime_file: storage.path().join("runtime.json"),
        database_url: db.clone(),
        stderr: stderr_capture,
        extra_env: vec![
            ("JAUNDER_CAPTURE_DIR".into(), capture.clone().into()),
            ("RUST_LOG".into(), "info".into()),
            (
                "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT".into(),
                collector.grpc_exporter_url().into(),
            ),
        ],
    };
    let mut server =
        match HostServerSession::start_interruptible(config, || interrupted(result, signals)) {
            Ok(server) => server,
            Err(failure) => {
                let detail = match failure.phase() {
                    ServerStartPhase::Spawn => {
                        format!("failed to spawn jaunder serve: {}", failure.error())
                    }
                    ServerStartPhase::RuntimeFile => {
                        format!(
                            "server runtime file not ready within 15s: {}",
                            failure.error()
                        )
                    }
                    ServerStartPhase::Http => failure.error().to_string(),
                    ServerStartPhase::Interrupted => "E2E server startup interrupted".to_owned(),
                };
                result.push(
                    StepResult::fail(&server_step)
                        .detail(detail)
                        .with_duration(server_start.elapsed()),
                );
                if let Some(mut server) = failure.into_session() {
                    let verification = LifecycleVerification {
                        sh,
                        browser,
                        test_support: &test_support,
                    };
                    finish_lifecycle(
                        result,
                        &mut server,
                        &mut collector,
                        &retained,
                        &verification,
                        None,
                        signals,
                    );
                } else {
                    finish_server_setup_failure(result, browser, &retained, &mut collector);
                }
                return;
            }
        };
    let base_url = playwright_base_url(&server.base_url);
    result.push(StepResult::ok(&server_step).with_duration(server_start.elapsed()));
    let verification = LifecycleVerification {
        sh,
        browser,
        test_support: &test_support,
    };

    let tools = root.join("tools/Cargo.toml");
    let jaunder = &artifacts.jaunder;
    let seed_start = std::time::Instant::now();
    if run_consumer(
        devtool_seed_command(
            sh,
            &tools,
            &db,
            &test_support,
            jaunder,
            &capture,
            &collector.grpc_exporter_url(),
        ),
        result,
        signals,
    )
    .is_err()
    {
        result.push(
            StepResult::fail(&seed_step)
                .detail("devtool seed-e2e failed".to_owned())
                .with_duration(seed_start.elapsed()),
        );
        finish_lifecycle(
            result,
            &mut server,
            &mut collector,
            &retained,
            &verification,
            None,
            signals,
        );
        return;
    }
    result.push(StepResult::ok(&seed_step).with_duration(seed_start.elapsed()));

    // Playwright uses the same DB, storage root, capture directory, and
    // target/debug-prefixed PATH as the server and VM contract.
    sh.change_dir(root.join("end2end"));
    let playwright_start = std::time::Instant::now();
    let mut playwright_result = Ok(());
    for invocation in &lifecycle.invocations {
        if run_consumer(
            cmd!(sh, "playwright")
                .args(&invocation.args)
                .env("JAUNDER_E2E_BASE_URL", &base_url)
                .env(
                    "JAUNDER_E2E_PRODUCER_INPUT",
                    root.join(
                        ".xtask/diagnostics/e2e-host-local/producer-checkout-host-local.json",
                    ),
                )
                .env("JAUNDER_DB", &db)
                .env("JAUNDER_STORAGE_PATH", storage.path())
                .env("JAUNDER_TEST_EXECUTABLE", jaunder)
                .env("JAUNDER_CAPTURE_DIR", &capture)
                .env(
                    "JAUNDER_E2E_OTLP_HTTP_ENDPOINT",
                    collector.browser_http_trace_url(),
                )
                .env("JAUNDER_E2E_TRACE_ID", &trace_id)
                .env("JAUNDER_E2E_WORKERS", workers)
                .env("PLAYWRIGHT_HTML_OPEN", "never")
                .env("PATH", path),
            result,
            signals,
        )
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
        result,
        &mut server,
        &mut collector,
        &retained,
        &verification,
        Some((playwright_result, playwright_start.elapsed())),
        signals,
    );
}

fn validate_visual_update_platform(update: bool, linux: bool) -> Result<(), &'static str> {
    if update && !linux {
        Err(
            "canonical visual snapshot updates are Linux-only; run without --update-visual-snapshots for host browser investigation",
        )
    } else {
        Ok(())
    }
}

/// Build the served CSR bundle and binaries once, then execute each planned
/// browser against its own complete server/database/capture lifecycle.
pub fn run(
    sh: &Shell,
    result: &mut CommandResult,
    browser: Option<E2eLocalBrowser>,
    test_filter: Option<&str>,
    update_visual_snapshots: bool,
) {
    if let Err(detail) =
        validate_visual_update_platform(update_visual_snapshots, cfg!(target_os = "linux"))
    {
        result.push(StepResult::fail("e2e-local-platform").detail(detail));
        return;
    }
    // Resolve the whole Playwright environment before any subprocess. Missing
    // PATH is a configuration failure, not a late empty command search.
    let env_root = sh.current_dir();
    let mut provenance = crate::e2e_provenance::Request::start(&env_root, "host-local");
    let Some((workers, path)) = resolve_e2e_env_for_run(
        result,
        &env_root,
        env::var("JAUNDER_E2E_WORKERS"),
        env::var_os("PATH"),
        &mut std::io::stderr(),
    ) else {
        provenance.completed(false);
        return;
    };
    let plan = e2e_local_plan(browser, test_filter, update_visual_snapshots);
    let mut signals = match ServerSignals::install() {
        Ok(signals) => signals,
        Err(error) => {
            result.push(StepResult::fail("e2e-local-signals").detail(format!("{error:#}")));
            provenance.completed(false);
            return;
        }
    };
    let Some(artifacts) = HostArtifacts::prepare_interruptible(
        sh,
        result,
        HostArtifactConfig::e2e_local(plan.release_csr),
        || signals.take_pending(),
    ) else {
        provenance.completed(false);
        return;
    };

    for lifecycle in &plan.lifecycles {
        if interrupted(result, &mut signals) {
            break;
        }
        run_lifecycle(
            sh,
            result,
            &artifacts,
            lifecycle,
            &workers,
            &path,
            &mut signals,
        );
        if interrupted(result, &mut signals) {
            break;
        }
    }
    provenance.completed(result.ok && result.exit_override.is_none());
}

fn interrupted(result: &mut CommandResult, signals: &mut ServerSignals) -> bool {
    if result.exit_override.is_none() {
        result.exit_override = signals.take_pending();
    }
    result.exit_override.is_some()
}

fn run_consumer(
    command: xshell::Cmd<'_>,
    result: &mut CommandResult,
    signals: &mut ServerSignals,
) -> anyhow::Result<()> {
    if interrupted(result, signals) {
        anyhow::bail!("E2E invocation interrupted");
    }
    eprintln!("$ {command}");
    let command: std::process::Command = command.into();
    let mut owned = processkit::Command::new(command.get_program())
        .args(command.get_args())
        .inherit_stdin()
        .stdout(processkit::StdioMode::Inherit)
        .stderr(processkit::StdioMode::Inherit);
    if let Some(cwd) = command.get_current_dir() {
        owned = owned.current_dir(cwd);
    }
    for (key, value) in command.get_envs() {
        owned = match value {
            Some(value) => owned.env(key, value),
            None => owned.env_remove(key),
        };
    }
    let mut process = Process::start(owned)?;
    match signals.wait_until_stopped(|| process.is_stopped()) {
        ServerExit::Unexpected(_) => match process.wait()? {
            processkit::Outcome::Exited(0) => Ok(()),
            outcome => anyhow::bail!("E2E consumer failed: {outcome:?}"),
        },
        ServerExit::Signal(code) => {
            result.exit_override = Some(code);
            signals.shutdown_process(&mut process)?;
            anyhow::bail!("E2E invocation interrupted");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn cancellation_stops_owned_consumer_before_artifact_guard_release() {
        use std::fs::{self, OpenOptions};

        const CHILD: &str = "JAUNDER_E2E_CANCEL_PROBE";
        let child_directory = std::env::var_os(CHILD);
        let isolated_child = child_directory
            .as_deref()
            .is_some_and(|directory| Path::new(directory) == std::env::current_dir().unwrap())
            && std::env::args().any(|argument| argument == "--exact");
        if !isolated_child {
            for signal in ["INT", "TERM"] {
                let isolated = tempfile::tempdir().unwrap();
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "steps::e2e_local::tests::cancellation_stops_owned_consumer_before_artifact_guard_release",
                        "--nocapture",
                    ])
                    .current_dir(isolated.path())
                    .env(CHILD, isolated.path().canonicalize().unwrap())
                    .env("JAUNDER_E2E_CANCEL_SIGNAL", signal)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let shell = Shell::new().unwrap();
        let mut signals = ServerSignals::install().unwrap();
        let artifacts = HostArtifacts {
            root: temp.path().to_path_buf(),
            jaunder: temp.path().join("unused"),
            test_support: None,
            _build_lock: Some(crate::steps::host_server::host_artifact_build_lock().unwrap()),
        };
        let peer = OpenOptions::new()
            .read(true)
            .write(true)
            .open(".xtask/host-artifacts.lock")
            .unwrap();
        // A retained startup session is still an owned server and must be
        // stopped after the cancelled foreground consumer, before unlocking.
        use std::os::unix::fs::PermissionsExt;
        let server_path = temp.path().join("server");
        fs::write(&server_path, "#!/bin/sh\nexec sleep 60\n").unwrap();
        fs::set_permissions(&server_path, fs::Permissions::from_mode(0o755)).unwrap();
        let failure = HostServerSession::start_interruptible(
            ServerSessionConfig {
                shell: &shell,
                jaunder: server_path,
                storage: temp.path().to_path_buf(),
                runtime_file: temp.path().join("runtime.json"),
                database_url: "sqlite:unused.db".to_owned(),
                stderr: File::create(temp.path().join("server.stderr")).unwrap(),
                extra_env: Vec::new(),
            },
            || true,
        )
        .err()
        .expect("retain owned startup session");
        let mut server = failure.into_session().unwrap();
        let stopped = temp.path().join("stopped");
        // Keep the wait and trap in the owned shell: a foreground sleep can
        // defer TERM handling until after the supervisor's escalation deadline.
        let script = r#"trap ': > "$STOPPED"; exit 0' TERM
kill -"$SIGNAL" "$PARENT"
while :; do :; done"#;
        let mut result = CommandResult::new("e2e-local");
        let signal = std::env::var("JAUNDER_E2E_CANCEL_SIGNAL").unwrap();
        let outcome = run_consumer(
            cmd!(shell, "sh -c {script}")
                .env("PARENT", std::process::id().to_string())
                .env("SIGNAL", &signal)
                .env("STOPPED", &stopped),
            &mut result,
            &mut signals,
        );
        assert!(outcome.is_err());
        assert_eq!(
            result.exit_override,
            Some(if signal == "INT" { 130 } else { 143 })
        );
        assert!(
            fs::metadata(stopped).unwrap().is_file(),
            "consumer teardown completed"
        );
        assert!(matches!(
            peer.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        server.stop_interruptible(signals.next_signal()).unwrap();
        assert!(
            server.is_stopped(),
            "server teardown completes under held ownership"
        );
        assert!(matches!(
            peer.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(server);
        drop(artifacts);
        peer.try_lock()
            .expect("consumer teardown precedes ownership release");
    }

    #[test]
    fn local_collectors_overlap_while_default_metrics_port_is_occupied() {
        let occupied = match TcpListener::bind("127.0.0.1:8888") {
            Ok(listener) => Some(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => None,
            Err(error) => panic!("occupying collector metrics port: {error}"),
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask manifest directory has a workspace parent");
        let first_capture = tempfile::tempdir().expect("first collector capture directory");
        let mut first = start_collector(root, first_capture)
            .unwrap_or_else(|failure| panic!("starting first collector: {}", failure.error));
        let second_capture = tempfile::tempdir().expect("second collector capture directory");
        let mut second = start_collector(root, second_capture)
            .unwrap_or_else(|failure| panic!("starting second collector: {}", failure.error));
        second.shutdown().expect("second collector shutdown");
        first.shutdown().expect("first collector shutdown");
        drop(occupied);
    }

    #[test]
    fn seed_launch_locks_the_tools_workspace() {
        assert_eq!(
            devtool_seed_args(Path::new("/repo/tools/Cargo.toml")),
            [
                "run",
                "--locked",
                "--manifest-path",
                "/repo/tools/Cargo.toml",
                "--",
                "seed-e2e",
            ]
        );
    }

    #[test]
    fn local_seed_receives_owned_collector_endpoint() {
        let sh = Shell::new().expect("test shell");
        let command: std::process::Command = devtool_seed_command(
            &sh,
            Path::new("/repo/tools/Cargo.toml"),
            "sqlite:/tmp/local.db",
            "/repo/target/debug/test-support",
            Path::new("/repo/target/debug/jaunder"),
            "/tmp/local/capture",
            "http://127.0.0.1:54317",
        )
        .into();
        let environment: std::collections::HashMap<_, _> = command.get_envs().collect();
        assert_eq!(
            environment.get(OsStr::new("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")),
            Some(&Some(OsStr::new("http://127.0.0.1:54317")))
        );
        assert_eq!(
            environment.get(OsStr::new("JAUNDER_CAPTURE_DIR")),
            Some(&Some(OsStr::new("/tmp/local/capture")))
        );
    }

    #[test]
    fn canonical_visual_updates_require_linux_but_browser_investigation_does_not() {
        assert!(validate_visual_update_platform(true, false).is_err());
        assert!(validate_visual_update_platform(true, true).is_ok());
        assert!(validate_visual_update_platform(false, false).is_ok());
        assert!(validate_visual_update_platform(false, true).is_ok());
    }

    #[test]
    fn visual_updates_keep_independent_browser_lifecycles() {
        let plan = e2e_local_plan(None, None, true);
        assert!(plan.release_csr);
        assert_eq!(
            plan.lifecycles
                .iter()
                .map(|lifecycle| lifecycle.browser)
                .collect::<Vec<_>>(),
            ["chromium", "firefox"]
        );
    }

    #[test]
    fn omitted_browser_preserves_chromium_plan() {
        let plan = e2e_local_plan(None, None, false);
        assert!(!plan.release_csr);
        assert!(!plan.update_visual_snapshots);
        assert_eq!(plan.lifecycles.len(), 1);
        let lifecycle = &plan.lifecycles[0];
        assert_eq!(lifecycle.browser, "chromium");
        assert_eq!(
            lifecycle.invocations,
            [PlaywrightInvocation {
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
            }]
        );
    }

    #[test]
    fn filtered_firefox_theme_run_selects_admin_site_project() {
        let plan = e2e_local_plan(
            Some(E2eLocalBrowser::Firefox),
            Some("theme.spec.ts:63"),
            false,
        );
        assert_eq!(
            plan.lifecycles[0].invocations,
            [PlaywrightInvocation {
                label: "ordinary",
                args: owned_args(&[
                    "test",
                    "--project",
                    "firefox-visual",
                    "--project",
                    "firefox",
                    "--project",
                    "firefox-admin-site",
                    "--project",
                    "firefox-admin",
                    "--no-deps",
                    "--reporter=html,line",
                    "theme.spec.ts:63",
                ]),
            }]
        );
    }

    #[test]
    fn filtered_webkit_theme_run_selects_theme_project() {
        let plan = e2e_local_plan(
            Some(E2eLocalBrowser::Webkit),
            Some("theme.spec.ts:63"),
            false,
        );
        assert!(
            plan.lifecycles[0].invocations[0]
                .args
                .windows(2)
                .any(|arguments| arguments == ["--project", "webkit-theme"])
        );
    }

    #[test]
    fn filtered_post_actions_run_includes_each_browser_ordinary_project() {
        for (browser, project) in [
            (E2eLocalBrowser::Chromium, "chromium"),
            (E2eLocalBrowser::Firefox, "firefox"),
            (E2eLocalBrowser::Webkit, "webkit"),
        ] {
            let plan = e2e_local_plan(Some(browser), Some("post-actions.spec.ts"), false);
            let invocation = &plan.lifecycles[0].invocations[0];
            assert!(
                invocation
                    .args
                    .windows(2)
                    .any(|arguments| arguments == ["--project", project])
            );
            assert_eq!(
                invocation.args.last(),
                Some(&"post-actions.spec.ts".to_owned())
            );
        }
    }

    #[test]
    fn filtered_runs_do_not_allow_vacuous_success() {
        for browser in [
            E2eLocalBrowser::Chromium,
            E2eLocalBrowser::Firefox,
            E2eLocalBrowser::Webkit,
        ] {
            let plan = e2e_local_plan(Some(browser), Some("unsupported.spec.ts"), false);
            let invocation = &plan.lifecycles[0].invocations[0];
            assert!(
                !invocation
                    .args
                    .iter()
                    .any(|argument| argument == "--pass-with-no-tests"),
                "{browser:?} must let Playwright fail when no selected project matches"
            );
        }
    }

    #[test]
    fn browser_origin_uses_localhost_for_loopback_webauthn() {
        assert_eq!(
            playwright_base_url("http://127.0.0.1:4312"),
            "http://localhost:4312"
        );
        assert_eq!(
            playwright_base_url("https://example.test:4312"),
            "https://example.test:4312"
        );
    }

    #[test]
    fn playwright_and_panic_failures_are_aggregated() {
        let mut result = CommandResult::new("e2e-local");
        record_post_playwright_results(
            &mut result,
            "chromium",
            Err("playwright failed".to_owned()),
            Duration::ZERO,
            Err("panic verifier failed".to_owned()),
            Duration::ZERO,
        );
        assert!(!result.ok);
        let encoded = serde_json::to_string(&result).expect("result serialization");
        assert!(encoded.contains("playwright failed"));
        assert!(encoded.contains("panic verifier failed"));
    }

    #[test]
    fn capture_copy_requires_every_writer_to_stop() {
        assert!(capture_writers_stopped(true, true));
        assert!(!capture_writers_stopped(false, true));
        assert!(!capture_writers_stopped(true, false));
        assert!(!capture_writers_stopped(false, false));
    }
}
