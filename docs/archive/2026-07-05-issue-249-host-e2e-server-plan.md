# Host e2e loop owns its server — Implementation Plan (issue #249)

> **For agentic workers:** Execute this plan task-by-task with
> **`jaunder-iterate`** (delegating an individual task to a subagent via
> **`jaunder-dispatch`** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking. Commit per **`jaunder-commit`** — the pre-commit hook runs the full
> `cargo xtask check`; run it first so it passes clean. **No `Co-Authored-By`
> trailer.**

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-249-host-e2e-server.md`](../specs/2026-07-05-issue-249-host-e2e-server.md)
**Base:** `origin/main` @ `7a5366cb` (#236 landed via PR #272 — `build-csr` +
`devtool csr-bundle` present). Re-derive any line refs before editing.

> **Shipped (2026-07-05).** All tasks landed; every §5 AC verified
> (`cargo xtask validate` green, incl. the full
> `{sqlite,postgres}×{chromium,firefox}` e2e matrix + a 71/71 host smoke-run).
> Deviations from the plan as written: **T4a and T4b were merged into one
> commit** (introducing `base_url_from_runtime` / `ServerChild` in the same
> commit that uses them avoids an xtask dead-code boundary); the Playwright
> process also receives `JAUNDER_DB` + the capture-file env + a
> `target/debug`-prepended PATH (so `seed.ts`/`mail.ts`/`websub.ts` resolve the
> server's files/DB/binary); and **#268 (complete cargo-leptos removal) was
> folded into this branch** — no one uses the `watch`/`serve` dev loop, so it
> was removed with no replacement. Review nits were polished and ADR-0051's
> stale server-ownership note updated. The PR closes #249 and #268.

**Goal:** Make `cargo xtask e2e-local` own the full host e2e lifecycle — build,
start `jaunder serve` on an ephemeral port with the VM's capture env, seed via a
shared `devtool seed-e2e`, run Playwright against the discovered URL, and tear
the server down on every exit path — so the host and the Nix VM draw from one
shared e2e infrastructure instead of duplicating it.

**Architecture:** Rust (`xtask` host driver + `devtool` in-sandbox tool), Nix
(`flake.nix` VM checks), TypeScript (Playwright config/helpers). The server's
ephemeral-port discovery (`runtime.json`, ADR-0035) and dev-env schema auto-init
already exist — this plan is orchestration + one shared seed subcommand, no
server code change.

---

## Review header (approve this layer)

### Goal

One host command owns build → serve (ephemeral `:0` + runtime.json) → seed → run
→ teardown; the triplicated fixture seed collapses to one `devtool seed-e2e`
both host and VM call; Playwright's `baseURL` becomes the shared seam (host
feeds discovered port, VM feeds `:3000`).

### Scope

**In:**

- **T1** `devtool seed-e2e` — canonical fixture seed, shells to the
  `test-support` binary (spec §3.2).
- **T2** flake `seed_db()` (both backends) delegates to `devtool seed-e2e`; add
  `devtoolBin` to the two e2e guests (spec §3.2 prerequisite).
- **T3** parameterize Playwright `baseURL` via `JAUNDER_E2E_BASE_URL`; reconcile
  the pre-existing warmup knob (spec §3.3).
- **T4** rewrite `e2e_local.rs`: harness owns `jaunder serve` on `:0`, per-run
  temp storage/DB, capture env, runtime.json discovery, `devtool seed-e2e`,
  baseURL feed, teardown-by-PID (spec §3.1, G1/G2/G3). Split into T4a (tested
  primitives) + T4b (orchestration rewrite).
- **T5** retire `end2end-cmd`/`end2end-dir` (spec §4).
- **T6** full AC sweep.

**Out (separate issues, per spec §4):** VM ephemeral `:0` (VM stays `:3000`);
host diag/panic gate → #269; full cargo-leptos removal + `watch` dev-loop →
#268; single-binary embed → #237; #155 Firefox data point.

**Admin (T0):** #268/#269 already filed (verified). Legacy cluster plan doc
disposition — **pending user confirmation at the plan-approval HALT**.

### Tasks at a glance

- **T0** — Admin: confirm follow-ups filed (done); resolve legacy plan doc.
- **T1** — `devtool seed-e2e` subcommand + unit test on the canonical arg list.
- **T2** — flake: `devtoolBin` on guests + both `seed_db()` call
  `devtool seed-e2e`; verify `cargo xtask e2e {sqlite,postgres} chromium` green.
- **T3** — `helpers.ts` `BASE_URL` reads `JAUNDER_E2E_BASE_URL`; warmup derives
  from it; VM combo stays green (default path).
- **T4a** — pure helpers in `e2e_local.rs`: `base_url_from_runtime` +
  `ServerChild` RAII teardown, each with a test (AC 2 mechanism).
- **T4b** — rewrite `e2e_local::run`: build → temp DB → serve `:0` + capture env
  → discover → `devtool seed-e2e` → Playwright(baseURL, filter preserved) →
  teardown.
- **T5** — drop `end2end-cmd`/`end2end-dir`; refresh the stale
  `:3000`/cargo-leptos doc comments.
- **T6** — AC sweep: `e2e-local` green from clean, no orphan, concurrent
  isolation, single-test, mail/websub specs pass, `cargo xtask validate` green.

### Key risks / decisions

- **Teardown (AC 2).** `ServerChild` RAII (`Drop` → `kill` + `wait`) reaps the
  server on every exit path incl. early return/panic; the per-run temp
  `JAUNDER_RUNTIME_FILE` + `TempDir` mean a `SIGKILL`-skipped `Drop` can't leave
  a stale file that misleads the next run.
- **Stale `devtool` on host PATH.** A new `devtool` subcommand isn't callable
  via the on-PATH `devtool` until the devShell rebuilds. The host driver invokes
  seed via `cargo run --manifest-path tools/Cargo.toml -- seed-e2e …`
  (source-run), never the PATH binary. Nix/CI rebuilds `devtoolBin` via crane —
  unaffected.
- **Fresh-DB simplification.** Both callers now guarantee a fresh/truncated DB,
  so `devtool seed-e2e`'s `create-user` steps are **fatal** (not the old
  non-fatal host behavior) — a create failure is a real error, not an expected
  re-run collision.
- **#9 concurrency.** #9 edits `test-support`/`storage`; this plan touches
  neither (`devtool` only _calls_ the `test-support` binary). If #9 lands first,
  re-confirm the `create-user`/`set-site-config`/`reset-mail` subcommand flags
  are unchanged.
- **Backend parity.** T2 exercises the shared seed on **both** SQLite and
  Postgres VMs (`CONTRIBUTING.md`).

---

## Global Constraints

- **`devtool` lives in the separate `tools/` workspace** (`tools/devtool/`),
  crane-built as `devtoolBin`, runs in-sandbox. New subcommands go there, NOT
  `xtask` (host-only). It may **call** the `test-support` binary but must NOT
  link the `test-support` crate (main workspace).
- Every commit passes `cargo xtask check` (pre-commit hook). Run it first.
- Storage-touching tests follow the dual-backend template (`CONTRIBUTING.md`
  backend parity); not in ADR-0019 per-backend dialect files.
- The bundle keeps the fixed `/pkg/jaunder.{js,wasm}` URLs + embedded
  `SPA_SHELL` (#181/#234 guards unchanged).
- `xtask` is **host-only** and Linux (the driver uses POSIX process/kill
  semantics).

---

## Task 0 — Admin

- [x] Confirm the spec's spun-out follow-ups exist (no new issues to file):
      **#268** (dev-loop replacement) and **#269** (host diag/panic gate) — both
      verified OPEN. `git`/`gh` read only, no code.
- [x] **Resolve the legacy cluster plan doc.** `git rm`
      `docs/superpowers/plans/2026-07-05-issue-249-csr-build-e2e-consolidation.md`
      (its Phase-A content is #236's, preserved in git history / PR #272;
      leaving it gives `jaunder-develop` a second, conflicting `issue-249` plan
      artifact). **Do this only after the user confirms at the plan-approval
      HALT.** Commit:
      `docs(issue-249): remove superseded cluster plan (standalone #249 plan replaces it)`.

---

## Task 1 — `devtool seed-e2e` (shared canonical fixture seed)

Single source of truth for the fixture arg-list that is triplicated today (spec
§3.2). Shells out to the `test-support` binary; the canonical list lives here.

**Files:**

- Create: `tools/devtool/src/seed_e2e.rs`
- Modify: `tools/devtool/src/main.rs` (add `SeedE2e(SeedE2eArgs)` variant +
  args + dispatch arm)
- Test: in-file `#[cfg(test)]` in `seed_e2e.rs` (matches `csr_bundle.rs`'s
  pure-logic test convention)

**Interfaces:**

- Consumes: the `test-support` binary's existing subcommands `create-user`,
  `set-site-config`, `reset-mail` (unchanged).
- Produces:
  `pub fn run(db: &str, mail_file: &str, test_support_bin: &std::path::Path) -> anyhow::Result<()>`
  and a CLI
  `devtool seed-e2e --db <url> --mail-file <path> --test-support-bin <path>`.
  Both the host driver (T4b) and the flake (T2) invoke the CLI.

- [ ] **Step 1: Write the failing test** (pins the canonical arg-list — AC 7)

In `tools/devtool/src/seed_e2e.rs`, test the pure arg-builder:

```rust
#[cfg(test)]
mod tests {
    use super::seed_invocations;

    #[test]
    fn canonical_fixture_invocations() {
        let inv = seed_invocations("/tmp/mail.jsonl");
        let as_vecs: Vec<Vec<&str>> = inv
            .iter()
            .map(|(args, fatal)| {
                assert!(*fatal, "all e2e seed steps are fatal against a fresh DB");
                args.iter().map(String::as_str).collect()
            })
            .collect();
        assert_eq!(
            as_vecs,
            vec![
                vec!["create-user", "--username", "testlogin", "--password", "testpassword123"],
                vec!["create-user", "--username", "testnoemail", "--password", "testpassword123"],
                vec!["create-user", "--username", "testoperator", "--password", "testpassword123", "--operator"],
                vec!["set-site-config", "--key", "site.registration_policy", "--value", "open"],
                vec!["set-site-config", "--key", "feeds.websub_hub_url", "--value", "https://hub.test.local/"],
                vec!["reset-mail", "--path", "/tmp/mail.jsonl"],
            ]
        );
    }
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run --manifest-path tools/Cargo.toml -p devtool seed_e2e`
Expected: FAIL — `seed_invocations` / module not defined.

- [ ] **Step 3: Implement `seed_e2e.rs` against the test**

Signature and behavior (the arg-list is pinned by Step 1; `run` shells each
invocation out to `test_support_bin` with `JAUNDER_DB=db`, all fatal):

```rust
//! `devtool seed-e2e` — the canonical e2e fixture seed (users + site-config +
//! mail-reset) applied by BOTH the host loop (`cargo xtask e2e-local`) and the
//! flake VM `seed_db()`. Was three literal copies kept in sync by comment; now
//! one list. Shells out to the `test-support` binary (devtool can't link the
//! main-workspace crate). Every step is fatal: both callers guarantee a fresh /
//! truncated DB, so a failure is a real error, not a re-run collision.
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context};

/// The canonical fixture invocations as `(args, fatal)`; `fatal` is currently
/// always true (kept in the shape so a future non-fatal step is a data change,
/// not a control-flow change). Pure — unit-tested.
fn seed_invocations(mail_file: &str) -> Vec<(Vec<String>, bool)> {
    let s = |xs: &[&str]| (xs.iter().map(|x| (*x).to_owned()).collect(), true);
    vec![
        s(&["create-user", "--username", "testlogin", "--password", "testpassword123"]),
        s(&["create-user", "--username", "testnoemail", "--password", "testpassword123"]),
        s(&["create-user", "--username", "testoperator", "--password", "testpassword123", "--operator"]),
        s(&["set-site-config", "--key", "site.registration_policy", "--value", "open"]),
        s(&["set-site-config", "--key", "feeds.websub_hub_url", "--value", "https://hub.test.local/"]),
        s(&["reset-mail", "--path", mail_file]),
    ]
}

pub fn run(db: &str, mail_file: &str, test_support_bin: &Path) -> anyhow::Result<()> {
    for (args, _fatal) in seed_invocations(mail_file) {
        let status = Command::new(test_support_bin)
            .args(&args)
            .env("JAUNDER_DB", db)
            .status()
            .with_context(|| format!("spawning {} {}", test_support_bin.display(), args[0]))?;
        if !status.success() {
            bail!("test-support {} failed ({status})", args[0]);
        }
    }
    Ok(())
}
```

Wire `main.rs`: add to the `Command` enum and dispatch —

```rust
/// Seed the canonical e2e fixtures (users + site-config + mail-reset) by
/// shelling out to `test-support`. Shared by the host loop and the flake VM.
SeedE2e(SeedE2eArgs),
```

```rust
#[derive(clap::Args)]
struct SeedE2eArgs {
    /// Target database URL (also read from JAUNDER_DB).
    #[arg(long, env = "JAUNDER_DB")]
    db: String,
    /// Mail-capture file to reset (also read from JAUNDER_MAIL_CAPTURE_FILE).
    #[arg(long, env = "JAUNDER_MAIL_CAPTURE_FILE")]
    mail_file: String,
    /// Path to the `test-support` binary (on-PATH name on the VM; the built
    /// `target/debug/test-support` on the host).
    #[arg(long)]
    test_support_bin: std::path::PathBuf,
}
```

```rust
Command::SeedE2e(args) => seed_e2e::run(&args.db, &args.mail_file, &args.test_support_bin),
```

Add `mod seed_e2e;` and update the module doc-comment (`main.rs:3`) to drop
`seed-e2e` from the "planned" list.

- [ ] **Step 4: Run it, verify it passes**

Run: `cargo nextest run --manifest-path tools/Cargo.toml -p devtool seed_e2e`
Expected: PASS. Then
`cargo run --manifest-path tools/Cargo.toml -- seed-e2e --help` prints the three
flags.

- [ ] **Step 5: Commit**

```bash
git add tools/devtool/src/seed_e2e.rs tools/devtool/src/main.rs
git commit -m "feat(devtool): seed-e2e — one shared e2e fixture seed for host and VM (#249)"
```

Run `cargo xtask check` first.

---

## Task 2 — Flake `seed_db()` delegates to `devtool seed-e2e` (both backends)

Collapse the two remaining literal seed copies onto T1's subcommand and put
`devtool` on the guest PATH (spec §3.2 prerequisite).

**Files:**

- Modify: `flake.nix` — sqlite guest `systemPackages` (~`722-726`) and postgres
  guest (~`820-824`): add `devtoolBin`.
- Modify: `flake.nix` — sqlite `seed_db()` (~`762-770`) and postgres `seed_db()`
  (~`899-907`): replace the six inline `test-support …` calls with one
  `devtool seed-e2e`.

**Interfaces:**

- Consumes: `devtool seed-e2e` (T1) — on the guest PATH once `devtoolBin` is in
  `systemPackages`; `test-support` is already on PATH there.

- [ ] **Step 1: Add `devtoolBin` to both e2e guests**

In each guest's `environment.systemPackages` list, add `devtoolBin` alongside
`testSupportBin`:

```nix
environment.systemPackages = [
  pkgs.sqlite                      # (pkgs.postgresql_16 in the postgres guest)
  pkgs.opentelemetry-collector-contrib
  testSupportBin
  devtoolBin
];
```

- [ ] **Step 2: Replace the sqlite `seed_db()` inline seed**

The `seed_db()` reset (stop/wipe/start/wait) is unchanged; only the seed block
changes. Replace the
`machine.succeed("export JAUNDER_DB=…; test-support create-user …")` call with:

```python
machine.succeed(
  "devtool seed-e2e"
  + " --db sqlite:/var/lib/jaunder/data/jaunder.db"
  + " --mail-file /var/lib/jaunder/mail.jsonl"
  + " --test-support-bin test-support"
)
```

- [ ] **Step 3: Replace the postgres `seed_db()` inline seed**

The TRUNCATE reset is unchanged; replace the `test-support …` block with:

```python
machine.succeed(
  "devtool seed-e2e"
  + " --db postgres://jaunder:testpassword@127.0.0.1/jaunder"
  + " --mail-file /var/lib/jaunder/mail.jsonl"
  + " --test-support-bin test-support"
)
```

- [ ] **Step 4: Verify both backends green** (this is the test — the VM run
      exercises the shared seed on SQLite and Postgres; AC 7 VM half + AC 8)

Run (background — slow Nix VM builds; use Bash background mode):
`devtool run -- cargo xtask e2e sqlite chromium` Expected: PASS (VM boots,
`seed_db()` runs `devtool seed-e2e`, suite green). Run:
`devtool run -- cargo xtask e2e postgres chromium` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add flake.nix
git commit -m "refactor(nix): flake seed_db uses devtool seed-e2e; devtoolBin on e2e guests (#249)"
```

Run `cargo xtask check` first (a flake edit still runs the host gate).

---

## Task 3 — Parameterize Playwright `baseURL`

`helpers.ts:46` `BASE_URL` is a hardcoded constant reading no env var; the
warmup URL (`fixtures.ts:119/154`) reads a _different_ knob. Make `BASE_URL`
env-driven and derive the warmup default from it (spec §3.3), default unchanged
so the VM path and `goto`/`page.request` call sites keep working.

**Files:**

- Modify: `end2end/tests/helpers.ts:46`
- Modify: `end2end/tests/fixtures.ts:119` (warmup default derives from
  `BASE_URL`)

**Interfaces:**

- Produces: `BASE_URL` now equals
  `process.env.JAUNDER_E2E_BASE_URL ?? "http://localhost:3000"`. T4b feeds
  `JAUNDER_E2E_BASE_URL=http://ip:port`; the VM feeds nothing (keeps `:3000`).

- [ ] **Step 1: Make `BASE_URL` env-driven** (`helpers.ts:46`)

```ts
export const BASE_URL =
  process.env.JAUNDER_E2E_BASE_URL ?? "http://localhost:3000";
```

- [ ] **Step 2: Derive the warmup default from `BASE_URL`** (`fixtures.ts`)

Import `BASE_URL` and replace the hardcoded `defaultWarmupUrl`
(`fixtures.ts:119`):

```ts
import { BASE_URL } from "./helpers";
// ...
const defaultWarmupUrl = `${BASE_URL}/`;
```

The explicit `JAUNDER_E2E_WARMUP_URL` override (`fixtures.ts:154`) still wins;
only its **default** now tracks `BASE_URL` instead of a second hardcoded
`:3000`.

- [ ] **Step 3: Verify the default path is unchanged**

No JS unit harness exists for these helpers, so verify behaviorally — the VM
combo feeds no `JAUNDER_E2E_BASE_URL`, so `BASE_URL` must still resolve to
`:3000`: Run (background): `devtool run -- cargo xtask e2e sqlite chromium`
Expected: PASS (proves the `?? "http://localhost:3000"` default is intact). The
override path is exercised by T4b/T6 on the host.

- [ ] **Step 4: Commit**

```bash
git add end2end/tests/helpers.ts end2end/tests/fixtures.ts
git commit -m "refactor(e2e): parameterize Playwright baseURL via JAUNDER_E2E_BASE_URL (#249)"
```

Run `cargo xtask check` first (prettier/eslint run in the gate).

---

## Task 4a — Tested primitives for the host driver

Two pure/RAII pieces the rewrite (T4b) depends on, each with its own test — the
teardown test is AC 2's mechanism.

**Files:**

- Modify: `xtask/src/steps/e2e_local.rs` (add the two helpers + their tests; the
  `run()` rewrite is T4b)

**Interfaces:**

- Produces: `fn base_url_from_runtime(json: &str) -> Option<String>` and
  `struct ServerChild(std::process::Child)` with a killing `Drop`. T4b consumes
  both.

- [ ] **Step 1: Write the failing tests**

```rust
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
        let child = Command::new("sleep").arg("60").spawn().expect("spawn sleep");
        let pid = child.id();
        let proc = std::path::PathBuf::from(format!("/proc/{pid}"));
        let guard = ServerChild(child);
        assert!(proc.exists(), "child should be alive before drop");
        drop(guard); // Drop kills AND waits (reaps the zombie so /proc/<pid> clears)
        // Linux-only (xtask is host-only Linux): once killed + reaped, /proc/<pid>
        // is gone. Zero-dependency liveness check — no external `kill` binary
        // (which isn't guaranteed on the devShell PATH).
        assert!(!proc.exists(), "child must be reaped after drop");
    }
}
```

- [ ] **Step 2: Run them, verify they fail**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml e2e_local` Expected:
FAIL — `base_url_from_runtime` / `ServerChild` not defined.

- [ ] **Step 3: Implement the two helpers**

```rust
use std::process::Child;

/// Parse the server's `runtime.json` (`{"ip","port"}`, ADR-0035) into a base URL.
/// `None` on malformed input or a missing field — the caller keeps polling.
fn base_url_from_runtime(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let ip = v.get("ip")?.as_str()?;
    let port = v.get("port")?.as_u64()?;
    Some(format!("http://{ip}:{port}"))
}

/// Owns the spawned `jaunder serve` child and reaps it on `Drop`, so no exit path
/// (early return, `?`, panic-unwind) leaks the server (AC 2). `SIGKILL` is fine —
/// the child holds no state we need flushed; the per-run temp storage dir is
/// dropped separately.
struct ServerChild(Child);

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
```

(`serde_json` is already an `xtask` dependency — `xtask/Cargo.toml:13` — so no
`Cargo.toml` change is needed.)

- [ ] **Step 4: Run them, verify they pass**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml e2e_local` Expected:
PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/e2e_local.rs
git commit -m "feat(xtask): runtime.json parse + ServerChild teardown guard for e2e-local (#249)"
```

Run `cargo xtask check` first.

---

## Task 4b — Rewrite `e2e_local::run` to own the server

The orchestration: build → per-run temp storage/DB → serve `:0` + capture env →
discover port → seed → Playwright(baseURL) → teardown (spec §3.1).

**Files:**

- Modify: `xtask/src/steps/e2e_local.rs` (rewrite `run`; refresh the module doc;
  the canonical env-var-set doc block)
- Modify: `xtask/src/lib.rs:113-122` (refresh the `E2eLocal` doc comment — no
  longer "against an ALREADY-RUNNING server")
- Modify: `flake.nix` (~48-56 `mailCaptureEnv` — a cross-reference comment only)

**Interfaces:**

- Consumes: `base_url_from_runtime`, `ServerChild` (T4a); `devtool seed-e2e`
  (T1); `cargo xtask build-csr` (#236); `JAUNDER_E2E_BASE_URL` (T3).
- Produces: `pub fn run(sh, result, test_filter: Option<&str>)` — signature
  unchanged, so `lib.rs` dispatch is untouched; the single-test filter still
  threads to Playwright (AC 6).

- [ ] **Step 1: Rewrite `run` — build + own the server lifecycle**

Replace the body of `run` (keep the signature). Sequence, each step
`StepResult::fail` + early return on error (the `ServerChild`/`TempDir` guards
fire on the return):

1. Resolve `root` (`git rev-parse --show-toplevel`), as today. **Keep cwd at
   `{root}`** through build → serve → discover → seed (absolute paths
   throughout); `sh.change_dir(format!("{root}/end2end"))` **only** just before
   the Playwright run (as today) — Playwright needs `end2end/` for its
   config/node_modules.
2. **Build:** call `crate::steps::build_csr::run(sh, result, false)`
   **directly** (bundle → `target/site/pkg/`) — not `cargo xtask build-csr`,
   which would recompile the running xtask and inherit the `cargo xtask` alias's
   relative-path fragility. Then `cargo build -p jaunder` (server bin →
   `{root}/target/debug/jaunder`) and `cargo build -p test-support` (seed impl →
   `{root}/target/debug/test-support`), run with cwd = `{root}`.
3. **Per-run temp storage dir:** `let storage = tempfile::tempdir()?;` →
   `let db = format!("sqlite:{}/jaunder.db", storage.path().display());` and
   `let runtime = storage.path().join("runtime.json");` and per-run mail/websub
   capture files under `storage`.
4. **Spawn `jaunder serve`** (dev env — omit `--environment prod` — so schema
   auto-inits on start), wrapped in `ServerChild`:
   ```rust
   let child = std::process::Command::new(format!("{root}/target/debug/jaunder"))
       .arg("serve")
       .env("JAUNDER_BIND", "127.0.0.1:0")
       .env("JAUNDER_STORAGE_PATH", storage.path())
       .env("JAUNDER_DB", &db)
       .env("JAUNDER_RUNTIME_FILE", &runtime)
       .env("JAUNDER_MAIL_CAPTURE_FILE", &mail)
       .env("JAUNDER_WEBSUB_CAPTURE_FILE", &websub)
       .env("JAUNDER_DIAG_LOG_FILE", &diag)
       .spawn()?;
   let _server = ServerChild(child);   // reaped on every return below
   ```
5. **Discover the port:** poll `runtime` (~15s, 30×0.5s) →
   `base_url_from_runtime`; then poll `curl -sf {base_url}/` ready. Fail
   `e2e-local-server` on timeout.
6. **Seed:**
   `cargo run --manifest-path {root}/tools/Cargo.toml -- seed-e2e --db {db} --mail-file {mail} --test-support-bin {root}/target/debug/test-support`
   (absolute manifest; source-run avoids the stale-PATH `devtool`). Fatal on
   error.
7. **Playwright:** as today (`--project chromium --project chromium-admin`,
   `--reporter=html,line`, `JAUNDER_E2E_WORKERS` default 1,
   `PLAYWRIGHT_HTML_OPEN=never`, `test_filter` passthrough) **plus**
   `.env("JAUNDER_E2E_BASE_URL", &base_url)`.
8. On scope exit the `ServerChild` `Drop` kills the server and `TempDir` `Drop`
   removes the storage — no orphan, no dev-DB pollution.

The concrete body is determined by the sequence above plus the existing
`StepResult` pattern; write it out in `run` (this is orchestration the unit
tests in T4a can't pin end-to-end — its verification is behavioral, T6).

- [ ] **Step 2: Refresh the doc comments**

Update the `e2e_local.rs` module doc (`:1-10`) and `lib.rs` `E2eLocal` doc
(`:113-118`) to describe the owned-server loop (ephemeral port, temp DB, capture
env, teardown) — drop "ALREADY-RUNNING server on :3000" and the
`cargo leptos end-to-end` framing.

**Also (spec §3.3, second deliverable — the single documented list):** add a
doc-comment block in `e2e_local.rs` enumerating the **canonical e2e-server
env-var set** the host driver and the flake systemd unit both source —
`JAUNDER_BIND`, `JAUNDER_DB`, `JAUNDER_STORAGE_PATH`, `JAUNDER_RUNTIME_FILE`,
`JAUNDER_MAIL_CAPTURE_FILE`, `JAUNDER_WEBSUB_CAPTURE_FILE`,
`JAUNDER_DIAG_LOG_FILE` — and a one-line cross-reference comment at `flake.nix`
`mailCaptureEnv` (~48-56) pointing back to it, so the two consumers reference
one documented list (names shared; only values differ per environment).

- [ ] **Step 3: Verify the T4a tests still pass + it builds**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml e2e_local` Expected:
PASS (T4a's 3 tests; `run`'s behavior is verified in T6). Run:
`cargo build --manifest-path xtask/Cargo.toml` → builds clean.

- [ ] **Step 4: Smoke-run the loop** (behavioral verification of `run`)

Ensure nothing is bound to `:3000`/dev server, then: Run (background):
`devtool run -- cargo xtask e2e-local` Expected: builds, starts its own server
on an ephemeral port, seeds, runs chromium + chromium-admin, exits 0; afterwards
`ss -tlnp` shows no leftover `jaunder`. (Note the stale-`devtool` caveat: run
from a devShell whose `devtool` post-dates nothing here — the driver source-runs
devtool, so the host PATH binary is irrelevant.)

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/e2e_local.rs xtask/src/lib.rs flake.nix
git commit -m "feat(xtask): e2e-local owns an ephemeral-port jaunder serve (VM parity) (#249)"
```

Run `cargo xtask check` first.

---

## Task 5 — Retire `cargo leptos end-to-end` as the loop entry

Dropping `end2end-cmd`/`end2end-dir` prevents double-serving now that
`e2e-local` owns its server (spec §4). cargo-leptos stays installed for `watch`
(full removal → #268).

**Files:**

- Modify: `Cargo.toml:134-135` (remove `end2end-cmd` + `end2end-dir` from
  `[package.metadata.leptos]`)

- [ ] **Step 1: Remove the two lines**

Delete:

```toml
end2end-cmd = "cargo run --manifest-path ../xtask/Cargo.toml -- e2e-local"
end2end-dir = "end2end"
```

- [ ] **Step 2: Verify the leptos config still parses + the loop entry works**

Run: `devtool run -- cargo leptos build` → succeeds (config valid without
`end2end-*`). Run (background): `devtool run -- cargo xtask e2e-local` → green
(the entry point is now the xtask, not `cargo leptos end-to-end`).

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore(build): retire cargo leptos end-to-end (e2e-local owns the loop) (#249)"
```

Run `cargo xtask check` first.

---

## Task 6 — Acceptance-criteria sweep

No new code — assemble the pieces and confirm every spec §5 AC. Fix regressions
in the owning task if any fail.

- [ ] **AC 1/6/9 — self-contained loop + single-test + no leptos end-to-end.**
      From a clean tree with `:3000` free: `cargo xtask e2e-local` → green; then
      `cargo xtask e2e-local auth.spec.ts` → runs only that spec; confirm no
      `cargo leptos end-to-end` was needed.
- [ ] **AC 2 — no orphan after failure.** Run `cargo xtask e2e-local` against a
      spec forced to fail (or `SIGINT` mid-run); afterwards `ss -tlnp` /
      `pgrep jaunder` shows no surviving server. (T4a's
      `server_child_kills_on_drop` covers the mechanism; this confirms it
      end-to-end.)
- [ ] **AC 3 — concurrent server/DB isolation.** Launch two
      `cargo xtask e2e-local` at once; confirm distinct ephemeral ports (two
      `runtime.json` under two temp dirs) and distinct DBs; neither touches
      `data/jaunder.db`. (Shared Playwright output dirs are out of scope — spec
      §5 AC 3.)
- [ ] **AC 4/5 — capture-env parity + fresh DB.** Confirm `email.spec.ts`,
      `password_reset.spec.ts`, `feeds.spec.ts` pass on the host (they fail on
      `main`); run `e2e-local` twice back-to-back — no
      `username is already taken`.
- [ ] **AC 7 — seed dedup.** `rg "create-user" xtask/ flake.nix` returns
      **nothing** (the literal list is gone from both), while
      `rg "create-user" tools/devtool/src/seed_e2e.rs` shows it now lives only
      there; host + both flake `seed_db()` call `devtool seed-e2e`.
- [ ] **AC 8 — matrix unchanged.** `cargo xtask validate` green (all four
      `{sqlite,postgres}×{chromium,firefox}` combos + coverage); VM systemd
      unit + `:3000` bind untouched.
- [ ] No commit unless a fix was needed; if so, one clean commit referencing
      `#249`.

---

## Self-review (author checklist — done before HALT)

- **Spec coverage:** §3.1→T4a/T4b; §3.2→T1/T2; §3.3→T3 (baseURL) + T4b
  (documented canonical env-var list); §4 end2end-cmd→T5; ACs→T6; spun-outs→T0
  (already filed). No gap.
- **Placeholders:** none — every code step carries the actual
  test/signature/body or is a marked behavioral-verification step (T4b run, T3,
  T2 — config/orchestration with no unit harness, verified by the e2e suite).
- **Type consistency:** `seed_invocations`/`run` (T1), `base_url_from_runtime` +
  `ServerChild` (T4a→T4b), `JAUNDER_E2E_BASE_URL` (T3→T4b) names match across
  tasks.
