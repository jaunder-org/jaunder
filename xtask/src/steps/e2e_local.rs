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
//! (names shared, values per-environment; see also `flake.nix` `captureEnv`):
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
use std::thread::sleep;
use std::time::{Duration, Instant};

use processkit::Command as ProcessCommand;

use self::capture::{RetainedCapture, allocate_retained_capture, finalize_capture};
use self::process::{CollectorGuard, CollectorStartError, ServerProcess};

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
    sh: &Shell,
    result: &mut CommandResult,
    server: &mut ServerProcess,
    collector: &mut CollectorGuard,
    retained: &RetainedCapture,
    verification: &LifecycleVerification<'_>,
    playwright_result: Option<(Result<(), String>, Duration)>,
) {
    let server_log_step = step_name(verification.browser, "server-log");
    let server_stopped = match server.stop() {
        Ok(()) => true,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to finalize server stderr capture: {error}")),
            );
            server.stopped()
        }
    };

    let capture = stop_and_finalize_collector(
        result,
        verification.browser,
        retained,
        collector,
        server_stopped,
    );

    let panic_gate_start = Instant::now();
    let panic_gate_result = if let Some(capture) = capture {
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
    let test_support_path = root.join("target/debug/test-support");
    let test_support = test_support_path.display().to_string();
    let retained = match allocate_retained_capture(root, browser, &test_support_path) {
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
    let runtime = storage.path().join("runtime.json");
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
    let command = ProcessCommand::new(root.join("target/debug/jaunder"))
        .arg("serve")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_STORAGE_PATH", storage.path())
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .env("RUST_LOG", "info")
        .env(
            "JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT",
            collector.grpc_exporter_url(),
        );
    let mut server = match ServerProcess::start(command, stderr_capture) {
        Ok(server) => server,
        Err(error) => {
            result.push(
                StepResult::fail(&server_step)
                    .detail(format!("failed to spawn jaunder serve: {error}"))
                    .with_duration(server_start.elapsed()),
            );
            finish_server_setup_failure(result, browser, &retained, &mut collector);
            return;
        }
    };

    let verification = LifecycleVerification {
        browser,
        test_support: &test_support,
    };
    if let Err(error) = server.wait_for_path(&runtime, Duration::from_secs(15)) {
        result.push(
            StepResult::fail(&server_step)
                .detail(format!("server runtime file not ready within 15s: {error}"))
                .with_duration(server_start.elapsed()),
        );
        finish_lifecycle(
            sh,
            result,
            &mut server,
            &mut collector,
            &retained,
            &verification,
            None,
        );
        return;
    }
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
            &mut server,
            &mut collector,
            &retained,
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
            &mut server,
            &mut collector,
            &retained,
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
            .env("JAUNDER_E2E_TRACE_ID", &trace_id)
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
        &mut server,
        &mut collector,
        &retained,
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

    #[test]
    fn visual_updates_keep_independent_browser_lifecycles() {
        let plan = e2e_local_plan(None, true);
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
