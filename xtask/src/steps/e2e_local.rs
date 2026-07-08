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
//! (names shared, values per-environment; see also `flake.nix` `mailCaptureEnv`):
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
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

use xshell::{cmd, Shell};

use crate::result::{CommandResult, StepResult};

/// Parse the server's `runtime.json` (`{"ip","port"}`, ADR-0035) into a base URL.
/// `None` on malformed JSON or a missing field — the caller keeps polling.
fn base_url_from_runtime(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let ip = v.get("ip")?.as_str()?;
    let port = v.get("port")?.as_u64()?;
    Some(format!("http://{ip}:{port}"))
}

/// Owns the spawned `jaunder serve` child and reaps it on `Drop`, so no exit path
/// (early return, panic-unwind) leaks the server (#249 G1). `SIGKILL` is fine — the
/// child holds no state we need flushed; the per-run temp storage dir is dropped
/// separately.
struct ServerChild(Child);

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
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
    let Ok(root) = cmd!(sh, "git rev-parse --show-toplevel").quiet().read() else {
        result.push(StepResult::fail("e2e-local").detail("cannot locate repo root".to_owned()));
        return;
    };
    let root = root.trim().to_owned();

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
    let _server = ServerChild(child);

    // 4. Discover the OS-assigned port from the runtime file, then wait for the
    // server to answer (~15s: 30 × 0.5s).
    let mut discovered = None;
    for _ in 0..30 {
        if let Ok(contents) = std::fs::read_to_string(&runtime) {
            if let Some(url) = base_url_from_runtime(&contents) {
                if cmd!(sh, "curl -sf {url}/").quiet().run().is_ok() {
                    discovered = Some(url);
                    break;
                }
            }
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
    if cmd!(
        sh,
        "cargo run --manifest-path {tools} -- seed-e2e --db {db} --test-support-bin {test_support}"
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
    if cmd!(sh, "playwright")
        .args(pw)
        .env("JAUNDER_E2E_BASE_URL", &base_url)
        .env("JAUNDER_DB", &db)
        .env("JAUNDER_CAPTURE_DIR", &capture)
        .env("JAUNDER_E2E_WORKERS", &workers)
        .env("PLAYWRIGHT_HTML_OPEN", "never")
        .env("PATH", &path)
        .run()
        .is_err()
    {
        result.push(
            StepResult::fail("e2e-local-playwright")
                .detail("Playwright reported failures".to_owned()),
        );
        return;
    }
    result.push(StepResult::ok("e2e-local-playwright"));
    // `_server` (ServerChild) and `storage` (TempDir) drop here: server killed and
    // reaped, temp storage removed.
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
    fn server_child_kills_on_drop() {
        // A long-lived child stands in for `jaunder serve`.
        let child = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let guard = ServerChild(child);
        assert!(proc.exists(), "child should be alive before drop");
        drop(guard); // Drop kills AND waits (reaps the zombie so /proc/<pid> clears)
                     // Linux-only (xtask is host-only Linux): once killed + reaped, /proc/<pid>
                     // is gone. Zero-dependency liveness check — no external `kill` binary.
        assert!(!proc.exists(), "child must be reaped after drop");
    }
}
