# Dedupe Playwright config (issue #153) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Make `end2end/playwright.config.ts` the single Playwright config that
both the host loop and the Nix/CI VM load, deleting the divergent
`nixPlaywrightConfig`, and promote the host runner to a first-class
`cargo xtask e2e-local` verb.

**Architecture:** One shared `.ts` config (already staged into the VM via
`e2ePackage`); the VM `testScript` loads it with
`--config playwright.config.ts`. The only host/VM difference is an invocation
flag in the host driver (`--reporter=html,line`

- `PLAYWRIGHT_HTML_OPEN=never`); everything else — admin-site quarantine,
  Firefox slimming, chromium sandbox args, workers, retries, timeout,
  trace/screenshot — is identical shared config. Chromium sandbox args are
  applied always; the json report goes to `test-results/results.json`
  (conventional name, inside the default `outputDir`).

**Tech Stack:** Playwright (TS config), Nix flake (`flake.nix`), Rust `xtask`
(clap subcommand + `xshell`), cargo-leptos (`end2end-cmd`).

**Spec:**
`docs/superpowers/specs/2026-07-04-issue-153-dedupe-playwright-config.md` — the
plan is "how"; the spec is "what/why". Referenced by section below; not
restated.

## Scope

- **In:** unify the config (Task 1); repoint the flake + delete
  `nixPlaywrightConfig` (Task 2); red-run diagnostics proof (Task 3); promote
  the host runner to `cargo xtask e2e-local` (Task 4); skill guidance (Task 5);
  ADR draft (Task 6); full-matrix parity gate (Task 7).
- **Out:** `fixtures.ts` per-test budget helpers (already config-agnostic
  post-#155); the CI e2e matrix shape / `e2e-gate` aggregation (ADR-0034) —
  untouched.
- **Separable concerns filed as issues:** none surfaced during the spec
  interview — the config unify, the driver promotion, and the guidance are one
  cohesive unit.

## Tasks (one line each)

1. Unify `playwright.config.ts` into the single source of truth (add admin
   quarantine, Firefox slimming, always-on chromium args, shared
   workers/retries/trace, drop the duplicated timeout constant, keep webkit
   defined).
2. Point the flake at the shared config; delete `nixPlaywrightConfig`; repoint
   the json flat-copy; scrub comment references. Verify one VM combo green (AC5,
   AC1).
3. Red-run diagnostics proof: force a failing spec, confirm the rescued bundle
   (AC6).
4. Promote `run-e2e.sh` → `cargo xtask e2e-local` (seed + Playwright, assumes a
   running server); repoint `end2end-cmd`; delete the script. Verify host
   green + workers=2 quarantine (AC3, AC8).
5. Add jaunder skill guidance to use the host fast loop + single-test invocation
   (AC4).
6. Record the decision as an ADR draft (single config; host loop
   assumes-running-server).
7. Full-matrix parity: `cargo xtask validate` green (AC7).

## Key risks & decisions

- **Server-lifecycle decision (plan-level, per spec §5):**
  `cargo xtask e2e-local` **assumes a server is already running** and does
  seed + Playwright only. cargo-leptos keeps ownership of build/serve/teardown
  by invoking it as `end2end-cmd`; standalone `cargo xtask e2e-local` is for
  fast re-runs against an already-serving instance. Rationale: reimplementing
  cargo-leptos's server orchestration in xtask is fragile and out of proportion
  to a config-dedup issue.
- **Red-run diagnostics (high-worry):** trace/screenshot policy MUST be
  `retain-on-failure`/`only-on-failure` (VM has `retries:0`, so `on-first-retry`
  captures nothing). Task 3 guards this with a real failing run — a green run
  proves nothing.
- **`.ts` config in the VM (low, loud):** loads via Playwright's own transform
  (the `.spec.ts` tests already do); fails loudly if wrong. AC5 pins it.
- **AC1 literal grep:** the token `nixPlaywrightConfig` must be gone from
  _comments_ too (`flake.nix:1052`), not just the `writeText`.
- **`cargo xtask` alias is cwd-relative** (`.cargo/config.toml:2` =
  `run --manifest-path xtask/Cargo.toml --`), so it can't be the `end2end-cmd`
  (which runs from `end2end/`). Task 4 uses
  `cargo run --manifest-path ../xtask/Cargo.toml -- e2e-local` and the driver
  resolves the repo root itself.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit (user global override).
- **Per-commit gate:** the pre-commit hook runs `cargo xtask check`; run it
  first so it passes clean (**jaunder-commit**). Do not commit without explicit
  user approval.
- **Preserve the external artifact-name contract:** the copied-out diagnostics
  keep the name `playwright-report-${backend}.json` (consumed by
  `docs/observability.md`, the xtask rescue predicate
  `name.starts_with("playwright-report-")`, and the #152 trace-analysis
  tooling). Only the _in-VM source_ path changes.
- **ADR-0039** (admin-site singleton serialization) and **ADR-0034** (CI e2e
  matrix) are invariants this work preserves, not modifies.
- e2e VM runs are host-only via `devtool run -- cargo xtask …` (worktree-aware);
  long runs → Bash background mode.

---

### Task 1: Unify `end2end/playwright.config.ts` into the single source of truth

**Files:**

- Modify: `end2end/playwright.config.ts` (whole file — currently 123 lines)
- Test: `end2end/playwright.config.ts` is validated by `playwright test --list`
  (no new test file; a Playwright config's contract is which projects/tests it
  resolves)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: a config exporting exactly these **project names** — `chromium`,
  `chromium-admin`, `firefox`, `firefox-admin`, `webkit` — with
  `admin-site.spec.ts` matched _only_ by the `*-admin` projects and ignored by
  the base projects. The flake (Task 2) selects
  `--project ${browser} --project ${browser}-admin`; the host driver (Task 4)
  selects `--project chromium --project chromium-admin`. These names are the
  contract both later tasks depend on.

The config, mirroring the current `nixPlaywrightConfig` (`flake.nix:509-624`)
settings so the VM is unchanged, plus keeping `webkit` for host use:

```ts
import { devices, defineConfig } from "@playwright/test";

const traceParent = process.env.JAUNDER_E2E_TRACEPARENT;
// Worker count is env-driven (#155), default 2. See flake.nix history / #155.
const workers = parseInt(process.env.JAUNDER_E2E_WORKERS || "2", 10);

// Firefox in a headless VM defaults to Fission + a content-process pool; each
// Playwright worker is a separate instance, so RSS multiplies. These prefs
// collapse each instance to one content process and trim caches — transparent to
// the app-level tests, and harmless on the host (#155, #61).
const firefoxLaunchOptions = {
  firefoxUserPrefs: {
    "fission.autostart": false,
    "dom.ipc.processCount": 1,
    "dom.ipc.processCount.webIsolated": 1,
    "browser.sessionhistory.max_total_viewers": 0,
    "browser.cache.memory.capacity": 51200,
  },
};

// Applied on every chromium project. Required in the Nix VM (runs as root);
// benign for a throwaway test browser locally, so shared rather than gated (#153).
const chromiumLaunchOptions = {
  args: ["--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage"],
};

export default defineConfig({
  testDir: "./tests",
  timeout: 30 * 1000,
  expect: { timeout: 5000 },
  fullyParallel: workers > 1,
  forbidOnly: !!process.env.CI,
  workers,
  // CI default reporter: streamed line output for the build log + a machine-readable
  // report at the conventional name, inside the default outputDir. The host driver
  // (cargo xtask e2e-local) overrides this with --reporter=html,line for interactive
  // runs (#153).
  reporter: [["line"], ["json", { outputFile: "test-results/results.json" }]],
  use: {
    actionTimeout: 0,
    // Capture forensics only on failure so a green run writes nothing extra (#123/#49).
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    ...(traceParent ? { extraHTTPHeaders: { traceparent: traceParent } } : {}),
  },
  // admin-site mutates site.title/base_url global singletons, so under fullyParallel it
  // must not overlap specs that read them (ADR-0039). Each browser splits into a parallel
  // main project (admin-site ignored) and a serial *-admin project that runs admin-site
  // alone AFTER the main project (dependencies + fullyParallel:false). At workers=1 this
  // is inert. webkit is defined for host use; the VM never selects it (WPE SIGABRT), so no
  // gating needed.
  projects: [
    {
      name: "chromium",
      testIgnore: /admin-site\.spec\.ts/,
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: chromiumLaunchOptions,
      },
    },
    {
      name: "chromium-admin",
      testMatch: /admin-site\.spec\.ts/,
      fullyParallel: false,
      dependencies: ["chromium"],
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: chromiumLaunchOptions,
      },
    },
    {
      name: "firefox",
      testIgnore: /admin-site\.spec\.ts/,
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: firefoxLaunchOptions,
      },
    },
    {
      name: "firefox-admin",
      testMatch: /admin-site\.spec\.ts/,
      fullyParallel: false,
      dependencies: ["firefox"],
      use: {
        ...devices["Desktop Firefox"],
        launchOptions: firefoxLaunchOptions,
      },
    },
    {
      name: "webkit",
      testIgnore: /admin-site\.spec\.ts/,
      use: { ...devices["Desktop Safari"] },
    },
  ],
});
```

Note: the old top-level `hydrationHeavyTimeoutScale = 2.2` constant and the
static per-project `timeout` overrides are **removed** — per-test budgets come
from `fixtures.ts` helpers (already config-agnostic, #155). `retries` is left
unset (→ 0), matching what CI actually runs today (the old host
`retries: process.env.CI ? 2 : 0` never applied in the VM; spec Problem table).

- [ ] **Step 1: Rewrite the config.** Replace the entire contents of
      `end2end/playwright.config.ts` with the block above.

- [ ] **Step 2: Verify the project set resolves (the config's contract test).**

Run (from `end2end/`): `devtool run -- npx playwright test --list` Then inspect
the parked output (Grep the `.xtask/run/<id>.out`): expect it to list tests
under `[chromium]`, `[chromium-admin]`, `[firefox]`, `[firefox-admin]`,
`[webkit]`, with `admin-site.spec.ts` appearing **only** under the `*-admin`
projects and **absent** under the base `chromium`/`firefox`/`webkit` projects.
Expected: PASS (config parses, five projects, admin split correct). This is the
fast proof that the `.ts` config is valid and the quarantine is wired before any
browser launches.

- [ ] **Step 3: Commit.**

Run `cargo xtask check` first (**jaunder-commit**); then:

```bash
git add end2end/playwright.config.ts
git commit -m "test(e2e): make playwright.config.ts the single source of truth (#153)"
```

---

### Task 2: Point the flake at the shared config; delete `nixPlaywrightConfig`

**Files:**

- Modify: `flake.nix` — delete the `nixPlaywrightConfig` `writeText`
  (`509-624`); change `--config playwright.nix.config.js` →
  `--config playwright.config.ts` (`738`); drop the two
  `cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js` lines (`866`,
  `970`); change the json flat-copy source `/tmp/e2e/playwright-report.json` →
  `/tmp/e2e/test-results/results.json` (`765`) **keeping** the output name
  `/tmp/playwright-report-${backend}.json`; scrub the `nixPlaywrightConfig`
  doc-comment references (`1052` and the `workers` note).

**Interfaces:**

- Consumes: the shared config's project names from Task 1 (the run still passes
  `--project ${browser} --project ${browser}-admin`, unchanged at `739`).
- Produces: a flake whose e2e checks realize the shared `.ts` config. No new
  symbols.

- [ ] **Step 1: Delete `nixPlaywrightConfig`.** Remove the whole
      `nixPlaywrightConfig = pkgs.writeText … '';` binding
      (`flake.nix:509-624`).

- [ ] **Step 2: Repoint the run + copy lines.** At `flake.nix:738` change
      `--config playwright.nix.config.js` to `--config playwright.config.ts`.
      Delete the two
      `machine.succeed("cp ${nixPlaywrightConfig} /tmp/e2e/playwright.nix.config.js")`
      lines (`866`, `970`) — the shared config is already staged via
      `e2ePackage` (`647`, `cp -r ${e2ePackage} /tmp/e2e`). At `765` change the
      copy source to `/tmp/e2e/test-results/results.json`, leaving the
      destination `/tmp/playwright-report-${backend}.json` unchanged.

- [ ] **Step 3: Scrub comment references.** Remove/rewrite the
      `nixPlaywrightConfig` mentions in comments (`flake.nix:1052` warm-gate
      note and the `workers` comment) so the token no longer appears anywhere in
      `flake.nix`.

- [ ] **Step 4: Verify the token is gone (AC1).**

Run: `rg -n 'nixPlaywrightConfig|playwright\.nix\.config' flake.nix` Expected:
FAIL — no matches (rg exits 1). The config path in the run is now
`playwright.config.ts`.

- [ ] **Step 5: Verify one VM combo green (AC5) — the load-bearing `.ts`-in-VM
      proof.**

Run (Bash background mode — long/cold):
`devtool run -- cargo xtask e2e sqlite chromium` Then check the sidecar:
`jq '.ok' .xtask/last-result.json` → `true`, and the `xtask-done: … ok=true`
sentinel. This proves Playwright loads the `.ts` config in the VM and the
sqlite/chromium combo passes. Expected: PASS. (A VM boot flake is infra — retry;
don't generalize from one failure.)

- [ ] **Step 6: Commit.**

Run `cargo xtask check` first; then:

```bash
git add flake.nix
git commit -m "test(e2e): load the shared playwright.config.ts in the Nix VM; drop nixPlaywrightConfig (#153)"
```

---

### Task 3: Red-run diagnostics proof (AC6)

**Files:**

- Create (throwaway, removed before the commit):
  `end2end/tests/_dedupe-canary.spec.ts`
- Modify: none permanently.

**Interfaces:**

- Consumes: Task 2's flake (the rescue path). Produces: nothing — a verification
  gate.

This mirrors the #123 diagnostics-capture proof: a real failing run is the
_only_ way to prove `retain-on-failure`/`only-on-failure` actually captures
forensics under the VM's `retries: 0`.

- [ ] **Step 1: Add a deterministically-failing spec.**

```ts
// end2end/tests/_dedupe-canary.spec.ts — TEMPORARY, remove before committing.
import { test, expect } from "@playwright/test";
test("canary: force a red run to prove diagnostics capture", async () => {
  expect(1).toBe(2);
});
```

- [ ] **Step 2: Run one combo and let it fail.**

Run (Bash background): `devtool run -- cargo xtask e2e sqlite chromium`
Expected: FAIL (`jq '.ok' .xtask/last-result.json` → `false`; the `xtask-done:`
sentinel present with `ok=false`, proving the process finished rather than
died).

- [ ] **Step 3: Confirm the rescued bundle.**

Run: `ls .xtask/diagnostics/e2e-sqlite-chromium/` Expected: contains
`playwright-report-sqlite.json` (the contract-named json report, sourced from
`test-results/results.json` in the VM) **and**
`playwright-artifacts-sqlite.tar.gz` (the tarred `test-results/` with the
canary's trace + screenshot). Optionally verify the tarball is non-empty:
`tar tzf .xtask/diagnostics/e2e-sqlite-chromium/playwright-artifacts-sqlite.tar.gz`
lists `test-results/…` entries.

- [ ] **Step 4: Remove the canary.**

Run: `git status` to confirm `_dedupe-canary.spec.ts` is untracked, then delete
it: `rm end2end/tests/_dedupe-canary.spec.ts`. Re-run `ls end2end/tests/` to
confirm it's gone. **No commit** — this task leaves the tree exactly as Task 2
left it (its value is the verification, recorded by ticking the boxes).

---

### Task 4: Promote `run-e2e.sh` → `cargo xtask e2e-local`

**Files:**

- Modify: `xtask/src/lib.rs` (add the `E2eLocal` command variant, its
  `command_name()` arm, its `run()` arm, a parse test, and `pub mod e2e_local;`
  in the inline `mod steps` block at `lib.rs:11-18` — there is no
  `steps/mod.rs`)
- Create: `xtask/src/steps/e2e_local.rs` (the seed + Playwright driver)
- Modify: `Cargo.toml:132` `end2end-cmd = "bash run-e2e.sh"` →
  `end2end-cmd = "cargo run --manifest-path ../xtask/Cargo.toml -- e2e-local"`
  (the `cargo xtask` alias is a **cwd-relative** manifest path,
  `.cargo/config.toml:2`, so it fails from `end2end/`; the explicit
  `../xtask/Cargo.toml` resolves there)
- Delete: `end2end/run-e2e.sh`

**Interfaces:**

- Consumes: the shared config's project names (Task 1) and `crate::sh::step` /
  `crate::result::{CommandResult, StepResult}` (existing).
- Produces: `Command::E2eLocal` (clap subcommand `e2e-local`) and
  `steps::e2e_local::run(sh: &Shell, result: &mut CommandResult, test_filter: Option<&str>)`.

**Server-lifecycle contract (plan decision):** `e2e-local` **assumes the dev
server is already listening on `:3000`** — it does _not_ start one. cargo-leptos
owns build/serve/ teardown and invokes `e2e-local` as `end2end-cmd`; run the
full loop with `cargo leptos end-to-end`. Standalone `cargo xtask e2e-local` is
for fast re-runs against an already-serving instance.

The driver replicates `run-e2e.sh` (seed via `test-support`, then Playwright) in
Rust, with the unified-config invocation flags. The seed commands (verbatim from
`run-e2e.sh:40-45`): three `create-user`, one `set-site-config` for the
registration policy, one for the WebSub hub URL, and `reset-mail`. Make the
three `create-user` calls tolerant of an already-seeded DB (a standalone re-run)
rather than aborting.

`steps::e2e_local::run` behavior, pinned by the steps below:

```rust
// xtask/src/steps/e2e_local.rs
use xshell::Shell;
use crate::result::CommandResult;
use crate::sh::step;

/// Seed fixtures via `test-support` and run Playwright against an ALREADY-RUNNING
/// dev server (`:3000`). Invoked by cargo-leptos as `end2end-cmd`, or standalone for
/// fast re-runs. Server lifecycle is cargo-leptos's, not ours (#153).
pub fn run(sh: &Shell, result: &mut CommandResult, test_filter: Option<&str>) {
    // 1. Build test-support (same code path as the flake VM's seed_db).
    result.push(step(sh, "e2e-local-build-support",
        "cargo", &["build", "-p", "test-support"]));
    // 2. Seed (idempotent create-user; see steps below for the exact contract).
    // 3. Playwright: chromium + chromium-admin, html+line reporter, no auto-open.
    //    Append `-- <test_filter>` (a spec path/grep) when Some, for single-test runs.
}
```

- [ ] **Step 1: Add the failing parse test** (mirrors
      `e2e_combo_parses_backend_and_browser` at `xtask/src/lib.rs:499`).

```rust
#[test]
fn e2e_local_parses_with_optional_filter() {
    let cli = Cli::try_parse_from(["xtask", "e2e-local"]).unwrap();
    match cli.command {
        Command::E2eLocal { test } => assert_eq!(test, None),
        _ => panic!("expected e2e-local"),
    }
    let cli = Cli::try_parse_from(["xtask", "e2e-local", "auth-flow.spec.ts"]).unwrap();
    match cli.command {
        Command::E2eLocal { test } => assert_eq!(test.as_deref(), Some("auth-flow.spec.ts")),
        _ => panic!("expected e2e-local with filter"),
    }
}
```

- [ ] **Step 2: Run it, verify it fails.**

Run: `cargo test --manifest-path xtask/Cargo.toml e2e_local_parses` Expected:
FAIL — `Command::E2eLocal` does not exist yet (compile error).

- [ ] **Step 3: Add the command + module + `run()` arm.**

In `xtask/src/lib.rs` add to `enum Command` (with a doc comment noting it
assumes a running server and is host-only):

```rust
/// Run the host e2e loop (seed + Playwright chromium) against an ALREADY-RUNNING
/// dev server. cargo-leptos invokes this as end2end-cmd; run the full loop with
/// `cargo leptos end-to-end`. Optionally pass a spec path/grep to run one test.
E2eLocal {
    /// A spec file or -g grep passed through to Playwright (single-test runs).
    test: Option<String>,
},
```

Add its **`command_name()`** arm (there is no `label()` — the exhaustive match
is `command_name()` at `lib.rs:175-188`, and a missing arm won't compile):
`Command::E2eLocal { .. } => "e2e-local"`. Add the `run()` arm:

```rust
Command::E2eLocal { test } => {
    let sh = xshell::Shell::new()?;
    let start = std::time::Instant::now();
    let mut result = CommandResult::new("e2e-local");
    steps::e2e_local::run(&sh, &mut result, test.as_deref());
    finalize(&mut result, start);
    Ok(result)
}
```

Declare `pub mod e2e_local;` **inside the inline `mod steps { … }` block in
`xtask/src/lib.rs:11-18`** (there is no `steps/mod.rs` — the module is declared
inline), and create `xtask/src/steps/e2e_local.rs`.

**Working dir + binary paths (from the blocker review):** `end2end-cmd` runs
from `<root>/end2end` (cargo-leptos), but standalone `cargo xtask e2e-local`
runs from wherever the user is. So the driver first resolves `root` =
`git rev-parse --show-toplevel` and runs seed + Playwright with the shell's cwd
set to `<root>/end2end` (xshell `sh.change_dir`), so both entry points behave
identically and `playwright` finds `playwright.config.ts` + `node_modules`.
`test-support` is invoked by **absolute path**
`<root>/target/debug/test-support` (it is not on PATH — mirrors
`run-e2e.sh:20`), after `cargo build -p test-support`.

**Server-readiness wait (from review):** before seeding, poll
`curl -sf http://localhost:3000/` up to ~15s (mirrors `run-e2e.sh:26-31`) so
seeding never races the server's SQLite schema init.

The seed + Playwright steps, replicating `run-e2e.sh`:

- **create-user (non-fatal):**
  `<root>/target/debug/test-support create-user --username testlogin --password testpassword123`
  (and `testnoemail`, and `testoperator --operator`). These must be
  **non-fatal** on a duplicate — `test-support create-user` hits a
  UNIQUE-constraint error and exits non-zero when the user already exists
  (standalone re-run against a persistent DB). **Do NOT use `crate::sh::step`**
  for these — it records any non-zero exit as a _failed_ `StepResult` that sets
  `result.ok=false`. Instead run them via `sh.cmd(...).ignore_status()` and push
  an `ok`/skipped `StepResult` regardless (or pre-check existence). Keep the
  exact usernames.
- **fatal seed (via `step`):**
  `test-support set-site-config --key site.registration_policy --value open`,
  `test-support set-site-config --key feeds.websub_hub_url --value https://hub.test.local/`,
  `test-support reset-mail --path "$JAUNDER_MAIL_CAPTURE_FILE"` (default
  `/tmp/jaunder-mail.jsonl`, matching `run-e2e.sh:22`).
- **Playwright (via `step`):** program `playwright`, args
  `["test", "--project", "chromium", "--project", "chromium-admin",   "--reporter=html,line"]`,
  plus the `test_filter` appended when `Some`, with `PLAYWRIGHT_HTML_OPEN=never`
  in the environment (so a red run doesn't spawn a blocking report server).
  `JAUNDER_*` env mirrors `run-e2e.sh` (`JAUNDER_DB_PATH` default
  `../data/jaunder.db` relative to `<root>/end2end`,
  `JAUNDER_DB=sqlite:$JAUNDER_DB_PATH`).

- [ ] **Step 4: Run the parse test, verify it passes.**

Run: `cargo test --manifest-path xtask/Cargo.toml e2e_local_parses` Expected:
PASS.

- [ ] **Step 5: Repoint `end2end-cmd` and delete the script.**

In `Cargo.toml:132` set
`end2end-cmd = "cargo run --manifest-path ../xtask/Cargo.toml -- e2e-local"`
(see the Files block for why the bare `cargo xtask` alias fails from
`end2end/`). Delete `end2end/run-e2e.sh` (`git rm end2end/run-e2e.sh`).

- [ ] **Step 6: Verify the host loop green + workers=2 quarantine (AC3, AC8).**

The unified config already defaults `workers=2` (`JAUNDER_E2E_WORKERS || "2"`),
so a single full-loop run exercises both the green path and the >1-worker
quarantine — no separate workers run needed.

Run (Bash background — builds server + wasm):
`devtool run -- cargo leptos end-to-end` Expected: PASS. Then inspect the parked
log to confirm **both** AC facets:

- chromium + chromium-admin ran against the freshly-served host, HTML report
  produced under `end2end/playwright-report/`, and a red result would **not**
  spawn a blocking `show-report` server (`PLAYWRIGHT_HTML_OPEN=never`);
- `chromium-admin` tests ran **after** `chromium` completed (its
  `dependencies: ['chromium']`
  - `fullyParallel:false` — admin-site never overlapping the parallel specs),
    proving the fast loop is safe at the default 2 workers.

- [ ] **Step 7: Commit.**

Run `cargo xtask check` first; then:

```bash
git add xtask/src/lib.rs xtask/src/steps/e2e_local.rs Cargo.toml
git rm end2end/run-e2e.sh
git commit -m "feat(xtask): add e2e-local host runner; retire run-e2e.sh (#153)"
```

---

### Task 5: jaunder skill guidance — use the host fast loop (AC4)

**Files:**

- Modify: `docs/agents/` — the e2e/testing guidance doc (identify the file that
  documents e2e running; e.g. a testing or domain doc under `docs/agents/`). If
  none exists, add a short section to the most relevant existing agent doc
  rather than creating a stray file.

**Interfaces:** consumes the `cargo xtask e2e-local` command name from Task 4.

- [ ] **Step 1: Locate the guidance surface.**

Run: `rg -n 'e2e|end-to-end|playwright|cargo xtask e2e' docs/agents/` Pick the
doc where e2e running is (or should be) documented.

- [ ] **Step 2: Write the guidance.** Add a short, concrete section instructing
      contributors to use the host fast loop:
  - Full loop (builds + serves + runs): `cargo leptos end-to-end`.
  - Fast re-run against an already-serving instance: `cargo xtask e2e-local`.
  - **Single test:** `cargo xtask e2e-local <spec-or-grep>` (the passthrough
    from Task 4), e.g. `cargo xtask e2e-local auth-flow.spec.ts`.
  - Note that this runs the _same_ `playwright.config.ts` the CI VM runs, so
    "passes locally" now equals "passes in CI" (the point of #153), and that the
    CI/VM path is `cargo xtask e2e <backend> <browser>` /
    `cargo xtask validate`.

- [ ] **Step 3: Verify the command name is present (AC4).**

Run: `rg -n 'cargo xtask e2e-local' docs/agents/ .claude/skills/` Expected: PASS
— the guidance references the concrete command.

- [ ] **Step 4: Commit.**

```bash
git add docs/agents/
git commit -m "docs(agents): direct contributors to the cargo xtask e2e-local fast loop (#153)"
```

---

### Task 6: Record the decision as an ADR draft

**Files:**

- Create: a numberless ADR draft in `docs/adr/drafts/` (via **jaunder-adr** —
  the draft-out-of-git flow; `cargo xtask adr promote` numbers it at ship).

**Interfaces:** none (documentation).

- [ ] **Step 1: Draft the ADR** using **jaunder-adr**. Capture the decision and
      its rationale so a future reader doesn't reverse-engineer it:
  - **Decision:** one shared `playwright.config.ts` is the single source of
    truth, loaded by both the host driver and the Nix VM
    (`--config playwright.config.ts`); `nixPlaywrightConfig` is deleted.
  - **How host/VM differ:** only via an invocation flag in the host driver
    (`--reporter=html,line` + `PLAYWRIGHT_HTML_OPEN=never`); chromium sandbox
    args are shared (applied always); the json report uses
    `test-results/results.json` while the copied-out artifact keeps the
    `playwright-report-<backend>.json` contract name.
  - **Host loop:** `cargo xtask e2e-local` assumes a running server;
    cargo-leptos owns the server lifecycle via `end2end-cmd`.
  - **Why:** eliminates the host/CI drift that bit #152/#155 by construction
    (one file); relates to ADR-0039 (admin quarantine) and ADR-0034 (CI matrix),
    which are preserved.

- [ ] **Step 2: Commit** the draft (jaunder-adr flow; no `Co-Authored-By`).

```bash
git add docs/adr/drafts/
git commit -m "docs(adr): draft single-source Playwright config decision (#153)"
```

---

### Task 7: Full-matrix parity gate (AC7)

**Files:** none — the final verification.

**Interfaces:** consumes everything above.

- [ ] **Step 1: Run the full local gate.**

Run (Bash background — long): `devtool run -- cargo xtask validate` This
realizes all four `{sqlite,postgres}×{chromium,firefox}` combos on the shared
config plus the static/coverage steps. Expected: PASS
(`jq '.ok' .xtask/last-result.json` → `true`, `xtask-done: … ok=true`). Confirms
Firefox slimming still applies and the admin quarantine is serial across every
combo — no combo regressed under the unified config.

- [ ] **Step 2: No commit** — `validate` never mutates the tree. Ticking this
      box records the green full-matrix gate, satisfying AC7 and readiness for
      **jaunder-ship**.
