//! Host e2e loop driver (#249): this driver OWNS the whole loop —
//! select immutable server/seed artifacts, start one `jaunder serve` per
//! Playwright worker (#2076) on an ephemeral port with the VM's capture env,
//! discover each port from its canonical `<storage>/runtime.json`, seed each via
//! the shared `test-support seed-e2e`, run Playwright against the instances,
//! and tear every server down on every exit path. Each instance gets a fresh
//! temp storage dir + DB (distinct ephemeral port + DB ⇒ neither workers nor
//! concurrent runs collide at the server/DB layer, and the dev
//! `data/jaunder.db` is never touched). Loads the same `playwright.config.ts`
//! the CI VM loads, so "passes locally" == "passes in CI". Host only.
//!
//! Canonical per-instance e2e-server env-var set the host driver and the flake
//! both provide (names shared, values per-environment; see also
//! `nix/checks.nix` `e2eInstances`): `JAUNDER_BIND`, `JAUNDER_DB`,
//! `JAUNDER_STORAGE_PATH` and `JAUNDER_CAPTURE_DIR` (the single capture-dir
//! contract, #227). Values differ per environment (host: a temp storage dir +
//! ephemeral port per instance; VM instance `i`: storage
//! `/var/lib/jaunder/data-<i>` + port `3000+i`). Each server writes its capture
//! streams to its own instance subdirectory of the shared collector capture
//! (`host::capture::instance_directory`). Playwright
//! receives the instances as `JAUNDER_E2E_INSTANCES` (with the retained
//! bundle's bin directory on PATH); each worker binds its own entry's DB,
//! storage and capture dir, so `mail.ts`/`websub.ts` resolve the capture paths
//! (via `test-support capture-path`) its server writes, and `seed.ts`'s
//! bare-`test-support` `seedPostsViaTool` resolves the same binary + DB — VM
//! parity for the mail/websub/pagination specs.
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

use crate::report::{CommandResult, StepResult};

/// Ordinary Playwright browser project selected for a focused run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eLocalBrowser {
    Chromium,
    Firefox,
    Webkit,
}

impl E2eLocalBrowser {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Firefox => "firefox",
            Self::Webkit => "webkit",
        }
    }
}
use crate::steps::host_server::{
    HostArtifacts, HostServerSession, ServerSessionConfig, ServerStartPhase,
};
use crate::steps::host_signals::ServerSignals;
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

/// The browser's one project, so a positional filter finds a visual or a
/// non-visual test alike.
fn browser_invocation(browser: E2eLocalBrowser, test_filter: Option<&str>) -> PlaywrightInvocation {
    let mut args = owned_args(&[
        "test",
        "--project",
        browser.as_str(),
        "--reporter=html,line",
    ]);
    args.extend(test_filter.map(str::to_owned));
    PlaywrightInvocation {
        label: "browser",
        args,
    }
}

fn normal_invocations(
    browser: E2eLocalBrowser,
    test_filter: Option<&str>,
) -> Vec<PlaywrightInvocation> {
    vec![browser_invocation(browser, test_filter)]
}

fn resolve_e2e_env(
    workers: Result<String, env::VarError>,
    path: Option<OsString>,
    stderr: &mut impl Write,
) -> anyhow::Result<(usize, OsString)> {
    // The driver starts one server instance per worker, so the count must be
    // a real positive integer rather than whatever Playwright would coerce.
    let workers = match workers {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|workers| *workers > 0)
            .with_context(|| {
                format!("JAUNDER_E2E_WORKERS must be a positive integer, got {value:?}")
            })?,
        Err(env::VarError::NotPresent) => 1,
        Err(env::VarError::NotUnicode(_)) => {
            let _ = writeln!(
                stderr,
                "e2e-local: warning: e2e-local.e2e.workers_config: ignored invalid-Unicode JAUNDER_E2E_WORKERS; using 1"
            );
            1
        }
    };
    let inherited = path
        .ok_or(env::VarError::NotPresent)
        .map_err(|error| anyhow::Error::new(error).context("reading PATH for e2e-local"))?;
    Ok((workers, inherited))
}

fn resolve_e2e_env_for_run(
    result: &mut CommandResult,
    workers: Result<String, env::VarError>,
    path: Option<OsString>,
    stderr: &mut impl Write,
) -> Option<(usize, OsString)> {
    match resolve_e2e_env(workers, path, stderr) {
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
                            browser,
                            "--grep",
                            "@visual",
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

fn instance_step_name(browser: &str, index: usize, suffix: &str) -> String {
    step_name(browser, &format!("instance-{index}-{suffix}"))
}

/// Each instance's server-written capture streams live in their own
/// subdirectory of the shared collector capture.
fn instance_capture(capture: &Path, index: usize) -> PathBuf {
    capture.join(host::capture::instance_directory(index))
}

fn record_step(
    result: &mut CommandResult,
    step: &str,
    outcome: Result<(), String>,
    duration: Duration,
) {
    match outcome {
        Ok(()) => result.push(StepResult::ok(step).with_duration(duration)),
        Err(detail) => result.push(
            StepResult::fail(step)
                .detail(detail)
                .with_duration(duration),
        ),
    }
}

/// Run the shared zero-panic verifier once per instance, each over its own
/// retained capture subdirectory and server stderr.
fn record_panic_gates(
    result: &mut CommandResult,
    verification: &LifecycleVerification<'_>,
    capture: Option<&Path>,
    instances: usize,
) {
    for index in 0..instances {
        let started = Instant::now();
        let outcome = match capture {
            Some(capture) => {
                let sh = verification.sh;
                let test_support = verification.test_support;
                let capture = instance_capture(capture, index);
                let server_stderr = capture.join("server-stderr.log");
                cmd!(
                    sh,
                    "{test_support} verify-no-panics --capture-dir {capture} --server-log {server_stderr}"
                )
                .run()
                .map_err(|_| "shared zero-panic verifier failed".to_owned())
            }
            None => Err(
                "zero-panic verification skipped because capture writers could not be stopped"
                    .to_owned(),
            ),
        };
        record_step(
            result,
            &instance_step_name(verification.browser, index, "panic-gate"),
            outcome,
            started.elapsed(),
        );
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

/// Stop every started instance, then the collector, then verify each
/// instance's retained capture.
fn finish_lifecycle(
    result: &mut CommandResult,
    instances: &mut [Instance],
    collector: &mut CollectorGuard,
    retained: &RetainedCapture,
    verification: &LifecycleVerification<'_>,
    playwright_result: Option<(Result<(), String>, Duration)>,
    signals: &mut ServerSignals,
) {
    let mut servers_stopped = true;
    for (index, instance) in instances.iter_mut().enumerate() {
        let mut shutdown_signal = None;
        let stopped = match instance.server.stop_interruptible(async {
            shutdown_signal = Some(signals.next_signal_code().await);
        }) {
            Ok(()) => true,
            Err(error) => {
                result.push(
                    StepResult::fail(&instance_step_name(
                        verification.browser,
                        index,
                        "server-log",
                    ))
                    .detail(format!("failed to finalize server stderr capture: {error}")),
                );
                instance.server.is_stopped()
            }
        };
        servers_stopped &= stopped;
        if result.exit_override.is_none() {
            result.exit_override = shutdown_signal;
        }
    }

    let capture = stop_and_finalize_collector(
        result,
        verification.browser,
        retained,
        collector,
        servers_stopped,
    );

    if let Some((playwright_result, playwright_duration)) = playwright_result {
        record_step(
            result,
            &step_name(verification.browser, "playwright"),
            playwright_result,
            playwright_duration,
        );
    }
    record_panic_gates(result, verification, capture.as_deref(), instances.len());
}

fn seed_command<'a>(
    sh: &'a Shell,
    db: &str,
    test_support: &str,
    jaunder: &Path,
    storage: &Path,
    capture: &str,
    endpoint: &str,
) -> xshell::Cmd<'a> {
    // The already-selected immutable test-support artifact owns the canonical
    // seed list and runs its own user/mail steps; no tools-workspace build.
    // Its `jaunder` steps use the instance's storage root, not `./data` under
    // whatever directory the shell is in.
    cmd!(
        sh,
        "{test_support} seed-e2e --db {db} --test-support-bin {test_support} --jaunder-bin {jaunder}"
    )
    .env("JAUNDER_STORAGE_PATH", storage)
    .env("JAUNDER_CAPTURE_DIR", capture)
    .env("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT", endpoint)
}

/// One Playwright worker's server: its own storage root, SQLite database,
/// capture subdirectory and seeded `jaunder serve` (#2076).
struct Instance {
    // Owned until the lifecycle ends, after the server has stopped.
    storage: tempfile::TempDir,
    db: String,
    capture: PathBuf,
    server: HostServerSession,
}

/// The harness's description of one instance, as `JAUNDER_E2E_INSTANCES`
/// carries it to Playwright workers by parallel index.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct InstanceEntry<'a> {
    base_url: String,
    db: &'a str,
    storage_path: &'a Path,
    capture_dir: &'a Path,
}

/// Serialization fails only for a path that is not UTF-8.
fn instance_map(entries: &[InstanceEntry<'_>]) -> serde_json::Result<String> {
    serde_json::to_string(entries)
}

/// Start and seed the next instance. Any instance whose server process exists
/// joins `instances`, ready or not, so the caller's teardown stops and verifies
/// it; returns whether the new instance is ready for Playwright.
fn start_instance(
    sh: &Shell,
    result: &mut CommandResult,
    artifacts: &HostArtifacts,
    browser: &str,
    collector: &CollectorGuard,
    signals: &mut ServerSignals,
    instances: &mut Vec<Instance>,
) -> bool {
    let index = instances.len();
    let step = |suffix: &str| instance_step_name(browser, index, suffix);
    let setup_start = Instant::now();
    let storage = match tempfile::tempdir() {
        Ok(storage) => storage,
        Err(error) => {
            result.push(
                StepResult::fail(&step("tmpdir"))
                    .detail(format!("cannot create temp storage dir: {error}"))
                    .with_duration(setup_start.elapsed()),
            );
            return false;
        }
    };
    let capture = instance_capture(collector.capture_dir(), index);
    let stderr_capture = match std::fs::create_dir(&capture)
        .and_then(|()| File::create(capture.join("server-stderr.log")))
    {
        Ok(file) => file,
        Err(error) => {
            result.push(
                StepResult::fail(&step("server-log"))
                    .detail(format!("failed to create server stderr capture: {error}"))
                    .with_duration(setup_start.elapsed()),
            );
            return false;
        }
    };
    let db = format!("sqlite:{}/jaunder.db", storage.path().display());

    let server_start = Instant::now();
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
    let server =
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
                    StepResult::fail(&step("server"))
                        .detail(detail)
                        .with_duration(server_start.elapsed()),
                );
                instances.extend(failure.into_session().map(|server| Instance {
                    storage,
                    db,
                    capture,
                    server,
                }));
                return false;
            }
        };
    result.push(StepResult::ok(&step("server")).with_duration(server_start.elapsed()));

    let test_support = artifacts.test_support.display().to_string();
    let seed_start = Instant::now();
    let seeded = run_consumer(
        seed_command(
            sh,
            &db,
            &test_support,
            &artifacts.jaunder,
            storage.path(),
            &capture.display().to_string(),
            &collector.grpc_exporter_url(),
        ),
        result,
        signals,
    );
    instances.push(Instance {
        storage,
        db,
        capture,
        server,
    });
    if seeded.is_err() {
        result.push(
            StepResult::fail(&step("seed"))
                .detail("test-support seed-e2e failed".to_owned())
                .with_duration(seed_start.elapsed()),
        );
        return false;
    }
    result.push(StepResult::ok(&step("seed")).with_duration(seed_start.elapsed()));
    true
}

fn run_lifecycle(
    sh: &Shell,
    result: &mut CommandResult,
    artifacts: &HostArtifacts,
    lifecycle: &BrowserLifecycle,
    workers: usize,
    path: &OsString,
    signals: &mut ServerSignals,
) {
    let root = &artifacts.root;
    let browser = lifecycle.browser;
    let test_support_path = artifacts.test_support.as_path();
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
    let verification = LifecycleVerification {
        sh,
        browser,
        test_support: &test_support,
    };
    let trace_id = match new_trace_id() {
        Ok(trace_id) => trace_id,
        Err(error) => {
            result.push(
                StepResult::fail(&step_name(browser, "trace-context"))
                    .detail(format!("cannot create E2E trace context: {error:#}")),
            );
            finish_lifecycle(
                result,
                &mut [],
                &mut collector,
                &retained,
                &verification,
                None,
                signals,
            );
            return;
        }
    };

    // One fresh, seeded server per Playwright worker; every browser lifecycle
    // starts its own set.
    let mut instances = Vec::with_capacity(workers);
    for _ in 0..workers {
        if !start_instance(
            sh,
            result,
            artifacts,
            browser,
            &collector,
            signals,
            &mut instances,
        ) {
            finish_lifecycle(
                result,
                &mut instances,
                &mut collector,
                &retained,
                &verification,
                None,
                signals,
            );
            return;
        }
    }
    let entries: Vec<_> = instances
        .iter()
        .map(|instance| InstanceEntry {
            base_url: playwright_base_url(&instance.server.base_url),
            db: &instance.db,
            storage_path: instance.storage.path(),
            capture_dir: &instance.capture,
        })
        .collect();
    let instance_map = match instance_map(&entries) {
        Ok(instance_map) => instance_map,
        Err(error) => {
            result.push(
                StepResult::fail(&step_name(browser, "instance-map"))
                    .detail(format!("cannot describe instances to Playwright: {error}")),
            );
            finish_lifecycle(
                result,
                &mut instances,
                &mut collector,
                &retained,
                &verification,
                None,
                signals,
            );
            return;
        }
    };

    // Each worker binds its own instance's DB, storage root and capture
    // directory from the map; the selected immutable seed executable is on PATH.
    sh.change_dir(root.join("end2end"));
    let playwright_start = std::time::Instant::now();
    let mut playwright_result = Ok(());
    for invocation in &lifecycle.invocations {
        if run_consumer(
            cmd!(sh, "playwright")
                .args(&invocation.args)
                .env("JAUNDER_E2E_INSTANCES", &instance_map)
                .env(
                    "JAUNDER_E2E_PRODUCER_INPUT",
                    root.join(
                        ".xtask/diagnostics/e2e-host-local/producer-checkout-host-local.json",
                    ),
                )
                .env("JAUNDER_TEST_EXECUTABLE", &artifacts.jaunder)
                .env(
                    "JAUNDER_E2E_OTLP_HTTP_ENDPOINT",
                    collector.browser_http_trace_url(),
                )
                .env("JAUNDER_E2E_TRACE_ID", &trace_id)
                .env("JAUNDER_E2E_WORKERS", workers.to_string())
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
        &mut instances,
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
    // Validate workers and inherited PATH before any subprocess. Add the selected
    // bundle's bin only after selection, never a mutable checkout target.
    let env_root = sh.current_dir();
    let mut provenance = crate::e2e_provenance::Request::start(&env_root, "host-local");
    let Some((workers, path)) = resolve_e2e_env_for_run(
        result,
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
    let Some(artifacts) =
        HostArtifacts::prepare_interruptible(sh, result, plan.release_csr, &mut signals)
    else {
        provenance.completed(false);
        return;
    };

    let bin = artifacts._root.path().join("selected/bin");
    let path = match env::join_paths(std::iter::once(bin).chain(env::split_paths(&path))) {
        Ok(path) => path,
        Err(error) => {
            result.push(
                StepResult::fail("e2e-local-env")
                    .detail(format!("prepending immutable bundle bin: {error}")),
            );
            provenance.completed(false);
            return;
        }
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
            workers,
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

pub(super) fn run_consumer(
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
        // stdout carries only the JSON report; forward child stdout to stderr
        // as it arrives, retaining none of it.
        .on_stdout_line(|line| eprintln!("{line}"))
        .output_buffer(processkit::OutputBufferPolicy::bounded(0))
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
    let mut signal = None;
    let outcome = process.wait_interruptible(async {
        signal = Some(signals.next_signal_code().await);
    })?;
    match (outcome, signal) {
        (Some(processkit::Outcome::Exited(0)), _) => Ok(()),
        (Some(outcome), _) => anyhow::bail!("E2E consumer failed: {outcome:?}"),
        (None, signal) => {
            result.exit_override = signal;
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
        use std::fs;

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
        let retained = tempfile::tempdir().unwrap();
        let retained_path = retained.path().to_path_buf();
        let artifacts = HostArtifacts {
            root: temp.path().to_path_buf(),
            jaunder: temp.path().join("unused"),
            test_support: temp.path().join("unused-support"),
            _root: retained,
        };
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
        assert!(
            retained_path.exists(),
            "root held through foreground teardown"
        );
        server.stop_interruptible(signals.next_signal()).unwrap();
        assert!(
            server.is_stopped(),
            "server teardown completes under held ownership"
        );
        assert!(retained_path.exists(), "root held through server teardown");
        drop(server);
        drop(artifacts);
        assert!(
            !retained_path.exists(),
            "root released after both consumers"
        );
    }

    #[test]
    fn consumer_with_more_stdout_than_a_pipe_holds_runs_to_exit() {
        // A full Playwright suite prints far more than the ~64 KiB pipe
        // buffer; its stdout must be forwarded while it runs, not only after
        // it exits, or the child blocks writing and never exits.
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let shell = Shell::new().unwrap();
            let mut signals = ServerSignals::install().unwrap();
            let mut result = CommandResult::new("e2e-local");
            let script = "head -c 1048576 /dev/zero | tr '\\0' 'x' | fold -w 100";
            let outcome = run_consumer(cmd!(shell, "sh -c {script}"), &mut result, &mut signals);
            done.send(outcome.map_err(|error| error.to_string()))
                .unwrap();
        });
        let outcome = finished
            .recv_timeout(Duration::from_secs(30))
            .expect("consumer exits instead of blocking on a full stdout pipe");
        assert_eq!(outcome, Ok(()));
    }

    #[test]
    fn local_collectors_overlap_while_default_metrics_port_is_occupied() {
        let occupied = match TcpListener::bind("127.0.0.1:8888") {
            Ok(listener) => Some(listener),
            Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => None,
            Err(error) => panic!("occupying collector metrics port: {error}"),
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("host-proof lives at tools/host-proof in the checkout");
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
    fn seed_launch_runs_the_selected_test_support_artifact() {
        let sh = Shell::new().expect("test shell");
        let command: std::process::Command = seed_command(
            &sh,
            "sqlite:/tmp/local.db",
            "/repo/target/debug/test-support",
            Path::new("/repo/target/debug/jaunder"),
            Path::new("/tmp/local"),
            "/tmp/local/capture",
            "http://127.0.0.1:54317",
        )
        .into();
        assert_eq!(command.get_program(), "/repo/target/debug/test-support");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "seed-e2e",
                "--db",
                "sqlite:/tmp/local.db",
                "--test-support-bin",
                "/repo/target/debug/test-support",
                "--jaunder-bin",
                "/repo/target/debug/jaunder",
            ]
        );
    }

    #[test]
    fn local_seed_receives_owned_collector_endpoint() {
        let sh = Shell::new().expect("test shell");
        let command: std::process::Command = seed_command(
            &sh,
            "sqlite:/tmp/local.db",
            "/repo/target/debug/test-support",
            Path::new("/repo/target/debug/jaunder"),
            Path::new("/tmp/local"),
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
        assert_eq!(
            environment.get(OsStr::new("JAUNDER_STORAGE_PATH")),
            Some(&Some(OsStr::new("/tmp/local")))
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
                label: "browser",
                args: owned_args(&["test", "--project", "chromium", "--reporter=html,line"]),
            }]
        );
    }

    #[test]
    fn filtered_firefox_run_selects_the_browser_project() {
        let plan = e2e_local_plan(
            Some(E2eLocalBrowser::Firefox),
            Some("theme.spec.ts:63"),
            false,
        );
        assert_eq!(
            plan.lifecycles[0].invocations,
            [PlaywrightInvocation {
                label: "browser",
                args: owned_args(&[
                    "test",
                    "--project",
                    "firefox",
                    "--reporter=html,line",
                    "theme.spec.ts:63",
                ]),
            }]
        );
    }

    #[test]
    fn filtered_post_actions_run_includes_each_browser_project() {
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

    fn step_outcomes(result: &CommandResult) -> Vec<(&str, bool)> {
        result
            .steps
            .iter()
            .map(|step| (step.name.as_str(), step.ok))
            .collect()
    }

    #[test]
    fn each_instance_gets_its_own_panic_gate() {
        let temp = tempfile::tempdir().expect("verifier workspace");
        // `sh` reads the fake verifier as a script named by the subcommand, so
        // nothing executes a freshly written file (ETXTBSY under parallel
        // tests). It records each invocation's arguments in the capture
        // directory it was given, and reports a panic only for instance 1.
        std::fs::write(
            temp.path().join("verify-no-panics"),
            "printf '%s\\n' \"$0\" \"$@\" > \"$2/argv\"\ncase \"$2\" in */instance-1) exit 1;; esac\n",
        )
        .expect("write fake verifier");
        let capture = temp.path().join("capture");
        for index in 0..2 {
            std::fs::create_dir_all(instance_capture(&capture, index)).expect("instance capture");
        }
        let sh = Shell::new().expect("test shell");
        sh.change_dir(temp.path());
        let verification = LifecycleVerification {
            sh: &sh,
            browser: "chromium",
            test_support: "sh",
        };
        let mut result = CommandResult::new("e2e-local");

        record_panic_gates(&mut result, &verification, Some(&capture), 2);

        assert_eq!(
            step_outcomes(&result),
            [
                ("e2e-local-chromium-instance-0-panic-gate", true),
                ("e2e-local-chromium-instance-1-panic-gate", false),
            ]
        );
        assert!(!result.ok);
        let instance = capture.join("instance-0");
        assert_eq!(
            std::fs::read_to_string(instance.join("argv")).expect("instance 0 verifier argv"),
            format!(
                "verify-no-panics\n--capture-dir\n{}\n--server-log\n{}\n",
                instance.display(),
                instance.join("server-stderr.log").display()
            )
        );
    }

    #[test]
    fn unstoppable_capture_writers_fail_every_instance_panic_gate() {
        let sh = Shell::new().expect("test shell");
        let verification = LifecycleVerification {
            sh: &sh,
            browser: "firefox",
            test_support: "/nonexistent/test-support",
        };
        let mut result = CommandResult::new("e2e-local");

        record_panic_gates(&mut result, &verification, None, 2);

        assert_eq!(
            step_outcomes(&result),
            [
                ("e2e-local-firefox-instance-0-panic-gate", false),
                ("e2e-local-firefox-instance-1-panic-gate", false),
            ]
        );
    }

    #[test]
    fn instance_map_lists_each_worker_instance_in_index_order() {
        let entries = [
            InstanceEntry {
                base_url: "http://localhost:4100".to_owned(),
                db: "sqlite:/tmp/a/jaunder.db",
                storage_path: Path::new("/tmp/a"),
                capture_dir: Path::new("/tmp/capture/instance-0"),
            },
            InstanceEntry {
                base_url: "http://localhost:4200".to_owned(),
                db: "sqlite:/tmp/b/jaunder.db",
                storage_path: Path::new("/tmp/b"),
                capture_dir: Path::new("/tmp/capture/instance-1"),
            },
        ];
        assert_eq!(
            instance_map(&entries).unwrap(),
            concat!(
                r#"[{"baseUrl":"http://localhost:4100","db":"sqlite:/tmp/a/jaunder.db","#,
                r#""storagePath":"/tmp/a","captureDir":"/tmp/capture/instance-0"},"#,
                r#"{"baseUrl":"http://localhost:4200","db":"sqlite:/tmp/b/jaunder.db","#,
                r#""storagePath":"/tmp/b","captureDir":"/tmp/capture/instance-1"}]"#,
            )
        );
    }

    #[test]
    #[cfg(unix)]
    fn instance_map_refuses_a_path_json_cannot_carry() {
        use std::os::unix::ffi::OsStrExt;

        let entries = [InstanceEntry {
            base_url: "http://localhost:4100".to_owned(),
            db: "sqlite:/tmp/a/jaunder.db",
            storage_path: Path::new(OsStr::from_bytes(b"/tmp/\xff")),
            capture_dir: Path::new("/tmp/capture/instance-0"),
        }];
        assert!(instance_map(&entries).is_err());
    }

    #[test]
    fn worker_count_selects_how_many_instances_start() {
        let path = || Some(OsString::from("/bin"));
        let workers = |value: Result<String, env::VarError>| {
            resolve_e2e_env(value, path(), &mut Vec::new()).map(|(workers, _)| workers)
        };
        assert_eq!(workers(Err(env::VarError::NotPresent)).unwrap(), 1);
        assert_eq!(workers(Ok("2".to_owned())).unwrap(), 2);
        for invalid in ["0", "two", "-1", ""] {
            let error = workers(Ok(invalid.to_owned())).unwrap_err();
            assert!(
                error.to_string().contains("positive integer"),
                "{invalid:?}: {error:#}"
            );
        }
    }

    #[test]
    fn capture_copy_requires_every_writer_to_stop() {
        assert!(capture_writers_stopped(true, true));
        assert!(!capture_writers_stopped(false, true));
        assert!(!capture_writers_stopped(true, false));
        assert!(!capture_writers_stopped(false, false));
    }
}
