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
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::{JoinHandle, sleep};
use std::time::Duration;

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
                    if let Err(error) = child.kill() {
                        failures.push(format!("failed to stop jaunder serve: {error}"));
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
    panic_gate_result: Result<(), String>,
) {
    let playwright_step = step_name(browser, "playwright");
    match playwright_result {
        Ok(()) => result.push(StepResult::ok(&playwright_step)),
        Err(detail) => result.push(StepResult::fail(&playwright_step).detail(detail)),
    }

    record_panic_gate_result(result, browser, panic_gate_result);
}

fn record_panic_gate_result(
    result: &mut CommandResult,
    browser: &str,
    panic_gate_result: Result<(), String>,
) {
    let panic_step = step_name(browser, "panic-gate");
    match panic_gate_result {
        Ok(()) => result.push(StepResult::ok(&panic_step)),
        Err(detail) => result.push(StepResult::fail(&panic_step).detail(detail)),
    }
}

struct LifecycleVerification<'a> {
    browser: &'a str,
    test_support: &'a str,
    capture: &'a str,
    server_stderr: &'a Path,
}

fn finish_lifecycle(
    sh: &Shell,
    result: &mut CommandResult,
    server: &mut ServerChild,
    verification: &LifecycleVerification<'_>,
    playwright_result: Option<Result<(), String>>,
) {
    let server_log_step = step_name(verification.browser, "server-log");
    if let Err(error) = server.stop() {
        result.push(
            StepResult::fail(&server_log_step)
                .detail(format!("failed to finalize server stderr capture: {error}")),
        );
    }

    let test_support = verification.test_support;
    let capture = verification.capture;
    let server_stderr = verification.server_stderr;
    let panic_gate_result = cmd!(
        sh,
        "{test_support} verify-no-panics --capture-dir {capture} --server-log {server_stderr}"
    )
    .run()
    .map_err(|_| "shared zero-panic verifier failed".to_owned());
    if let Some(playwright_result) = playwright_result {
        record_post_playwright_results(
            result,
            verification.browser,
            playwright_result,
            panic_gate_result,
        );
    } else {
        record_panic_gate_result(result, verification.browser, panic_gate_result);
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
    let Ok(storage) = tempfile::tempdir() else {
        result.push(
            StepResult::fail(&tmpdir_step).detail("cannot create temp storage dir".to_owned()),
        );
        return;
    };
    let sp = storage.path().display();
    let db = format!("sqlite:{sp}/jaunder.db");
    let runtime = storage.path().join("runtime.json");
    let capture = format!("{sp}/capture");
    let server_stderr = storage.path().join("server-stderr.log");
    let stderr_capture = match File::create(&server_stderr) {
        Ok(file) => file,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to create server stderr capture: {error}")),
            );
            return;
        }
    };

    let child = match Command::new(root.join("target/debug/jaunder"))
        .arg("serve")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_STORAGE_PATH", storage.path())
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_RUNTIME_FILE", &runtime)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            result.push(
                StepResult::fail(&server_step)
                    .detail(format!("failed to spawn jaunder serve: {error}")),
            );
            return;
        }
    };
    let mut server = match ServerChild::new(child, stderr_capture) {
        Ok(server) => server,
        Err(error) => {
            result.push(
                StepResult::fail(&server_log_step)
                    .detail(format!("failed to start server stderr capture: {error}")),
            );
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
                .detail("server not reachable via runtime.json within 15s".to_owned()),
        );
        finish_lifecycle(sh, result, &mut server, &verification, None);
        return;
    };
    result.push(StepResult::ok(&server_step));

    let tools = root.join("tools/Cargo.toml");
    let jaunder = root.join("target/debug/jaunder");
    if cmd!(
        sh,
        "cargo run --manifest-path {tools} -- seed-e2e --db {db} --test-support-bin {test_support} --jaunder-bin {jaunder}"
    )
    .env("JAUNDER_CAPTURE_DIR", &capture)
    .run()
    .is_err()
    {
        result.push(StepResult::fail(&seed_step).detail("devtool seed-e2e failed".to_owned()));
        finish_lifecycle(sh, result, &mut server, &verification, None);
        return;
    }
    result.push(StepResult::ok(&seed_step));

    // Playwright uses the environment resolved before any subprocess. The DB,
    // capture directory, and target/debug-prefixed PATH match the VM contract.
    sh.change_dir(root.join("end2end"));
    let mut playwright_result = Ok(());
    for invocation in &lifecycle.invocations {
        if cmd!(sh, "playwright")
            .args(&invocation.args)
            .env("JAUNDER_E2E_BASE_URL", &base_url)
            .env("JAUNDER_DB", &db)
            .env("JAUNDER_CAPTURE_DIR", &capture)
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
        &verification,
        Some(playwright_result),
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
    let Ok(root) = git::toplevel(Path::new(".")) else {
        result.push(StepResult::fail("e2e-local").detail("cannot locate repo root".to_owned()));
        return;
    };

    for (pkg, label) in [
        ("jaunder", "e2e-local-build-server"),
        ("test-support", "e2e-local-build-support"),
    ] {
        if cmd!(sh, "cargo build -p {pkg}").run().is_err() {
            result.push(StepResult::fail(label).detail(format!("cargo build -p {pkg} failed")));
            return;
        }
        result.push(StepResult::ok(label));
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
            Err("shared verifier rejected a panic".to_owned()),
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
        assert_eq!(
            panic_gate.detail.as_deref(),
            Some("shared verifier rejected a panic")
        );
    }

    #[test]
    fn clean_post_playwright_results_are_both_successful() {
        let mut result = CommandResult::new("e2e-local");
        record_post_playwright_results(&mut result, "chromium", Ok(()), Ok(()));
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
        drop(guard); // Drop kills AND waits (reaps the zombie so /proc/<pid> clears)
        // Linux-only (xtask is host-only Linux): once killed + reaped, /proc/<pid>
        // is gone. Zero-dependency liveness check — no external `kill` binary.
        assert!(!proc.exists(), "child must be reaped after drop");
    }
}
