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
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread::{JoinHandle, sleep};
use std::time::Duration;

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
}

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn record_post_playwright_results(
    result: &mut CommandResult,
    playwright_ok: bool,
    panic_gate_result: Result<(), String>,
) {
    if playwright_ok {
        result.push(StepResult::ok("e2e-local-playwright"));
    } else {
        result.push(
            StepResult::fail("e2e-local-playwright")
                .detail("Playwright reported failures".to_owned()),
        );
    }

    match panic_gate_result {
        Ok(()) => result.push(StepResult::ok("e2e-local-panic-gate")),
        Err(detail) => result.push(StepResult::fail("e2e-local-panic-gate").detail(detail)),
    }
}

/// Build, own a `jaunder serve` on an ephemeral port, seed, run Playwright, tear
/// down. `test_filter`, when set, passes through to Playwright (a spec path or
/// `-g` grep) for single-test runs.
pub fn run(sh: &Shell, result: &mut CommandResult, test_filter: Option<&str>) {
    // 1. Build the served CSR bundle via the shared devtool path (#236); this also
    // leaves the shell's cwd at the repo root.
    super::build_csr::run(sh, result, false);
    if !result.ok {
        return; // build_csr already recorded the failing step
    }
    let Ok(root) = git::toplevel(Path::new(".")) else {
        result.push(StepResult::fail("e2e-local").detail("cannot locate repo root".to_owned()));
        return;
    };

    // The server bin and the out-of-process seed impl.
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

    // 2. Per-run temp storage dir → fresh DB (no reset needed) + concurrency
    // isolation. Removed when this fn returns, after the server is torn down.
    let Ok(storage) = tempfile::tempdir() else {
        result.push(
            StepResult::fail("e2e-local-tmpdir")
                .detail("cannot create temp storage dir".to_owned()),
        );
        return;
    };
    let sp = storage.path().display();
    let db = format!("sqlite:{sp}/jaunder.db");
    let runtime = storage.path().join("runtime.json");
    // The single capture-dir contract (#227): a dedicated subdir the server writes
    // mail.jsonl/websub.jsonl/diag.log into. Kept separate from the storage root so it
    // holds only capture streams (VM parity: /var/lib/jaunder/capture).
    let capture = format!("{sp}/capture");
    let server_stderr = storage.path().join("server-stderr.log");
    let stderr_capture = match File::create(&server_stderr) {
        Ok(file) => file,
        Err(error) => {
            result.push(
                StepResult::fail("e2e-local-server-log")
                    .detail(format!("failed to create server stderr capture: {error}")),
            );
            return;
        }
    };

    // 3. Start `jaunder serve` on an EPHEMERAL port (:0) with the canonical capture
    // env, in the dev environment (default) so the schema auto-inits on start.
    // ServerChild reaps it on every exit path below (#249 AC 2).
    let child = match Command::new(format!("{root}/target/debug/jaunder"))
        .arg("serve")
        .env("JAUNDER_BIND", "127.0.0.1:0")
        .env("JAUNDER_STORAGE_PATH", storage.path())
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_RUNTIME_FILE", &runtime)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            result.push(
                StepResult::fail("e2e-local-server")
                    .detail(format!("failed to spawn jaunder serve: {e}")),
            );
            return;
        }
    };
    let mut server = match ServerChild::new(child, stderr_capture) {
        Ok(server) => server,
        Err(error) => {
            result.push(
                StepResult::fail("e2e-local-server-log")
                    .detail(format!("failed to start server stderr capture: {error}")),
            );
            return;
        }
    };

    // 4. Discover the OS-assigned port from the runtime file, then wait for the
    // server to answer (~15s: 30 × 0.5s).
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
            StepResult::fail("e2e-local-server")
                .detail("server not reachable via runtime.json within 15s".to_owned()),
        );
        return;
    };
    result.push(StepResult::ok("e2e-local-server"));

    // 5. Seed the canonical fixtures via the SHARED devtool subcommand (the same
    // list the flake VM's seed_db uses). Source-run devtool: its `seed-e2e`
    // subcommand may post-date the host's on-PATH binary. The temp DB is fresh, so
    // no reset is needed.
    let tools = format!("{root}/tools/Cargo.toml");
    let test_support = format!("{root}/target/debug/test-support");
    // The `site_config` seed steps run through the shipped `jaunder` binary — the
    // same real build already spawned for `serve` above (cheap-kdf OFF).
    let jaunder = format!("{root}/target/debug/jaunder");
    if cmd!(
        sh,
        "cargo run --manifest-path {tools} -- seed-e2e --db {db} --test-support-bin {test_support} --jaunder-bin {jaunder}"
    )
    .env("JAUNDER_CAPTURE_DIR", &capture)
    .run()
    .is_err()
    {
        result
            .push(StepResult::fail("e2e-local-seed").detail("devtool seed-e2e failed".to_owned()));
        return;
    }
    result.push(StepResult::ok("e2e-local-seed"));

    // 6. Playwright against the discovered baseURL, from end2end/. The host serves a
    // slow debug wasm bundle, so run serial by default (workers=1, overridable via
    // JAUNDER_E2E_WORKERS; the VM keeps the config default of 2). The DB + capture
    // vars and a target/debug-prefixed PATH match the VM's Playwright env so
    // mail/websub readers and `seedPostsViaTool` (bare `test-support`) see the same
    // files/DB/binary.
    let workers = std::env::var("JAUNDER_E2E_WORKERS").unwrap_or_else(|_| "1".to_owned());
    let path = format!(
        "{root}/target/debug:{}",
        std::env::var("PATH").unwrap_or_default()
    );
    sh.change_dir(format!("{root}/end2end"));
    let mut pw: Vec<&str> = vec![
        "test",
        "--project",
        "chromium",
        "--project",
        "chromium-admin",
        "--reporter=html,line",
    ];
    if let Some(f) = test_filter {
        pw.push(f);
    }
    let playwright_ok = cmd!(sh, "playwright")
        .args(pw)
        .env("JAUNDER_E2E_BASE_URL", &base_url)
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .env("JAUNDER_E2E_WORKERS", &workers)
        .env("PLAYWRIGHT_HTML_OPEN", "never")
        .env("PATH", &path)
        .run()
        .is_ok();

    // Stop first: child exit closes stderr, so a successful stop also proves the
    // mirror reached EOF and flushed the complete verifier input.
    if let Err(error) = server.stop() {
        result.push(
            StepResult::fail("e2e-local-server-log")
                .detail(format!("failed to finalize server stderr capture: {error}")),
        );
    }

    let panic_gate_result = cmd!(
        sh,
        "{test_support} verify-no-panics --capture-dir {capture} --server-log {server_stderr}"
    )
    .run()
    .map_err(|_| "shared zero-panic verifier failed".to_owned());
    record_post_playwright_results(result, playwright_ok, panic_gate_result);
    // `storage` drops here after the server is reaped and verification has read
    // its per-run files.
}

#[cfg(test)]
mod tests {
    use super::*;
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
            false,
            Err("shared verifier rejected a panic".to_owned()),
        );

        assert!(!result.ok);
        let playwright = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-playwright")
            .expect("playwright step");
        let panic_gate = result
            .steps
            .iter()
            .find(|step| step.name == "e2e-local-panic-gate")
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
        record_post_playwright_results(&mut result, true, Ok(()));
        assert!(result.ok);
        assert_eq!(
            result
                .steps
                .iter()
                .filter(|step| {
                    step.name == "e2e-local-playwright" || step.name == "e2e-local-panic-gate"
                })
                .filter(|step| step.ok)
                .count(),
            2
        );
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
