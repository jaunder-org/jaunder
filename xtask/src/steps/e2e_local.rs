//! Host e2e loop driver (#153): seed fixtures via `test-support`, then run
//! Playwright (`chromium` + `chromium-admin`) against an ALREADY-RUNNING dev
//! server on `:3000`.
//!
//! cargo-leptos invokes this as its `end2end-cmd`, so the full loop (build +
//! serve + this) is `cargo leptos end-to-end`. A standalone `cargo xtask
//! e2e-local` assumes a server is already serving — handy for fast re-runs while
//! iterating. Server lifecycle stays with cargo-leptos, not us. This loads the
//! same `playwright.config.ts` the CI VM loads, so "passes locally" == "passes in
//! CI". Host only.
use std::thread::sleep;
use std::time::Duration;

use xshell::{cmd, Shell};

use crate::result::{CommandResult, StepResult};

/// Seed users/config, then run Playwright. See the module docs for the
/// server-lifecycle contract. `test_filter`, when set, is passed through to
/// Playwright (a spec path or `-g` grep) for single-test runs.
pub fn run(sh: &Shell, result: &mut CommandResult, test_filter: Option<&str>) {
    // Resolve the repo root so both entry points behave identically: cargo-leptos
    // runs `end2end-cmd` from `end2end/`, while a standalone invocation runs from
    // wherever the user is.
    let Ok(root) = cmd!(sh, "git rev-parse --show-toplevel").quiet().read() else {
        result.push(StepResult::fail("e2e-local").detail("cannot locate repo root".to_owned()));
        return;
    };
    let root = root.trim().to_owned();

    // Build test-support here (links storage — the same seed code path as the flake
    // VM's seed_db); cargo-leptos builds only the server.
    if cmd!(sh, "cargo build -p test-support").run().is_err() {
        result.push(
            StepResult::fail("e2e-local-build-support")
                .detail("cargo build -p test-support failed".to_owned()),
        );
        return;
    }
    result.push(StepResult::ok("e2e-local-build-support"));
    let test_support = format!("{root}/target/debug/test-support");

    // Seed + Playwright run from end2end/ (the config, node_modules, tests, and the
    // `../data/jaunder.db` relative default all resolve there).
    sh.change_dir(format!("{root}/end2end"));

    // Fixture env, defaulted like the retired run-e2e.sh so a standalone run works.
    let db_path =
        std::env::var("JAUNDER_DB_PATH").unwrap_or_else(|_| "../data/jaunder.db".to_owned());
    let db = format!("sqlite:{db_path}");
    let mail = std::env::var("JAUNDER_MAIL_CAPTURE_FILE")
        .unwrap_or_else(|_| "/tmp/jaunder-mail.jsonl".to_owned());
    // The host `cargo leptos end-to-end` serves an unoptimized *debug* CSR wasm
    // bundle whose hydration is slow, so run serial by default (workers=1, like the
    // retired run-e2e.sh) — each test gets full CPU. Overridable via
    // JAUNDER_E2E_WORKERS; the CI VM keeps the config default of 2 (release wasm).
    let workers = std::env::var("JAUNDER_E2E_WORKERS").unwrap_or_else(|_| "1".to_owned());

    // Wait for the dev server (cargo-leptos may still be starting it): ~15s, matching
    // run-e2e.sh (30 * 0.5s).
    let mut up = false;
    for _ in 0..30 {
        // No `.ignore_status()`: `curl -sf` exits non-zero until the server answers,
        // so `run()` returns Err and the poll actually waits (with it, run() is always
        // Ok and the loop would break on the first iteration — a no-op wait).
        if cmd!(sh, "curl -sf http://localhost:3000/")
            .quiet()
            .run()
            .is_ok()
        {
            up = true;
            break;
        }
        sleep(Duration::from_millis(500));
    }
    if !up {
        result.push(StepResult::fail("e2e-local-server").detail(
            "dev server not reachable on :3000 after 15s (start it with `cargo leptos end-to-end`)".to_owned(),
        ));
        return;
    }
    result.push(StepResult::ok("e2e-local-server"));

    // create-user is non-fatal: on a standalone re-run against a persistent DB the
    // user already exists (UNIQUE violation → non-zero exit), which must not abort.
    let users: [&[&str]; 3] = [
        &[
            "create-user",
            "--username",
            "testlogin",
            "--password",
            "testpassword123",
        ],
        &[
            "create-user",
            "--username",
            "testnoemail",
            "--password",
            "testpassword123",
        ],
        &[
            "create-user",
            "--username",
            "testoperator",
            "--password",
            "testpassword123",
            "--operator",
        ],
    ];
    for args in users {
        let _ = cmd!(sh, "{test_support}")
            .args(args.iter().copied())
            .env("JAUNDER_DB", &db)
            .quiet()
            .ignore_status()
            .run();
    }

    // set-site-config / reset-mail are fatal — a run against an unseeded site config
    // is meaningless.
    let seeds: [&[&str]; 3] = [
        &[
            "set-site-config",
            "--key",
            "site.registration_policy",
            "--value",
            "open",
        ],
        &[
            "set-site-config",
            "--key",
            "feeds.websub_hub_url",
            "--value",
            "https://hub.test.local/",
        ],
        &["reset-mail", "--path", mail.as_str()],
    ];
    for args in seeds {
        if cmd!(sh, "{test_support}")
            .args(args.iter().copied())
            .env("JAUNDER_DB", &db)
            .quiet()
            .run()
            .is_err()
        {
            result.push(
                StepResult::fail("e2e-local-seed")
                    .detail(format!("test-support {} failed", args[0])),
            );
            return;
        }
    }
    result.push(StepResult::ok("e2e-local-seed"));

    // Playwright: the same unified config the CI VM loads. The host overrides the
    // reporter for interactive use; PLAYWRIGHT_HTML_OPEN=never keeps a red run from
    // spawning a blocking `show-report` server (the fast loop's iterate-on-failure
    // case). chromium + chromium-admin mirror the VM so the admin-site quarantine
    // is exercised locally too.
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
        .env("JAUNDER_E2E_WORKERS", &workers)
        .env("PLAYWRIGHT_HTML_OPEN", "never")
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
}
