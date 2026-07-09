# Capture-Dir Contract (#227) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Replace the three per-stream e2e capture env vars
(`JAUNDER_MAIL_CAPTURE_FILE` / `JAUNDER_WEBSUB_CAPTURE_FILE` /
`JAUNDER_DIAG_LOG_FILE`) with a single `JAUNDER_CAPTURE_DIR` output-dir
contract, backed by a new host-only `host` crate.

**Architecture:** A new `host` workspace crate owns the one Rust helper (dir-var
name + filename constants + resolve/mkdir). `server` and `test-support` depend
on it; the Playwright readers delegate to a new `test-support capture-path`
subcommand so no filename literal lives in TypeScript. The e2e harness sets one
env var per environment and copies one `capture/` directory out per combo.

**Tech Stack:** Rust (cargo workspace), clap, Nix flake VM e2e,
Playwright/TypeScript, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-07-08-issue-227-capture-dir-contract.md`](../specs/2026-07-08-issue-227-capture-dir-contract.md)
— the "what/why". This plan is the "how"; it references spec sections rather
than restating them.

## Global Constraints

- **Clean break:** by the end,
  `rg 'JAUNDER_(MAIL|WEBSUB)_CAPTURE_FILE|JAUNDER_DIAG_LOG_FILE'` over the live
  surfaces (`server/`, `test-support/`, `xtask/`, `tools/`, `end2end/`,
  `flake.nix`, `docs/observability.md`, `CONTRIBUTING.md`) returns nothing (spec
  AC1). Historical docs under `docs/archive/` and prior `docs/superpowers/` are
  out of scope.
- **Filenames** (single-source in `host`): `mail.jsonl`, `websub.jsonl`,
  `diag.log`. The diag file drops its old `jaunder-` prefix.
- **Capture dir is a dedicated subdir:** `/var/lib/jaunder/capture` (VM) / a
  `capture/` subdir of the per-run temp dir (host) — never the state root.
- **Per-commit gate:** run `cargo xtask check` clean before each commit
  (**jaunder-commit**). It runs host static + clippy + Nix coverage/unit tests,
  **not** e2e — so intermediate commits stay green while the e2e wiring is
  mid-migration; the full e2e matrix is the final task.
- **No `Co-Authored-By` trailer.** Commit subjects reference `(#227)`.
- Follow `CONTRIBUTING.md` (coverage policy ADR-0050, backend parity,
  dialect-file rules). No storage/dual-backend tests are added by this plan.
- **Inject the path; don't read the env in a unit (spec Decision 5).**
  `build_mailer` and `default_client` take an `Option<PathBuf>`; the `serve`
  root (`commands.rs:422/432`) resolves it via `host::capture_path(Stream::…)`
  and passes it in. Their tests inject a `TempDir` path via a shared `rstest`
  fixture — **no `set_var`, no lock** — so they're race-free under both
  `nextest` and a plain threaded `cargo test --test <suite>`. The diag stream is
  the exception: it stays resolved inside `init_tracing_impl` (which already
  reads a family of env vars and installs the process-global subscriber), and
  its tests keep the existing module-wide `lock_env()`. Because mailer/websub no
  longer touch the env, **only** the observability tests set
  `JAUNDER_CAPTURE_DIR`, under that existing lock — **no new cross-module lock
  is added.** The `host` crate's own `ENV_LOCK` (Task 2) is independent
  (separate test binary).

---

## Task 1: File the `otel-traces` consolidation follow-up issue

Separable concern surfaced during the spec (spec "Out of scope"): the
collector-written `otel-traces.jsonl` is _not_ migrated here. File it so it
isn't lost.

**Files:** none (tracker only).

- [x] **Step 1:** Using **jaunder-issues**, create an issue in
      `jaunder-org/jaunder`, milestone "E2E test suite", type Task, label `dx`.
      Title:
      `tooling(e2e): extend the capture-dir contract to the collector-written otel trace file`.
      Body: the `otel-collector` writes `/var/lib/jaunder/otel-traces.jsonl` via
      its YAML `file` exporter (`flake.nix:526`), outside `JAUNDER_CAPTURE_DIR`;
      folding it in requires templating the collector config path, ensuring
      `capture/` exists before the collector (which starts before jaunder), and
      reworking the `otel-traces-<backend>.jsonl/otel-traces.jsonl` copy-out
      layout that `cargo xtask traces run` consumes. Reference #227 as the
      origin.
- [x] **Step 2:** Record the new issue number in this plan (below) so the ship
      step can cross-reference it. Follow-up issue: **#332** (added to Jaunder
      Backlog project #1).

---

## Task 2: Create the `host` crate + capture helper

The foundation every later task depends on.

**Files:**

- Create: `host/Cargo.toml`, `host/src/lib.rs`
- Modify: `Cargo.toml` (root workspace: `members` + `[workspace.dependencies]`)
- Test: in-file `#[cfg(test)]` in `host/src/lib.rs`

**Interfaces:**

- Produces (the whole crate's public API — later tasks depend on these exact
  names/types):
  - `host::CAPTURE_DIR_ENV: &str` (`= "JAUNDER_CAPTURE_DIR"`)
  - `host::Stream` —
    `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Stream { Mail, WebSub, Diag }`
  - `Stream::filename(self) -> &'static str` → `"mail.jsonl"` / `"websub.jsonl"`
    / `"diag.log"`
  - `Stream::parse(key: &str) -> Option<Stream>` → `"mail"`/`"websub"`/`"diag"`
    else `None`
  - `host::capture_dir() -> Option<std::path::PathBuf>` — trimmed
    `JAUNDER_CAPTURE_DIR`, `None` if unset/blank
  - `host::capture_path(stream: Stream) -> Option<std::path::PathBuf>` —
    `capture_dir()` joined with `stream.filename()`, `create_dir_all`-ing the
    dir; `None` when capture is off

- [x] **Step 1: Write `host/Cargo.toml`** — mirror an existing leaf crate's
      package stanza:

```toml
[package]
name = "host"
version = "0.1.0"
edition = "2021"

[lints]
workspace = true

[dependencies]
```

(No deps beyond `std`. Match the `edition` and any `[lints] workspace = true`
used by `common/Cargo.toml` — copy them verbatim if they differ from the above.)

- [x] **Step 2: Register the crate in the root `Cargo.toml`** — add `"host"` to
      `members` (keep alphabetical: after `csr`, before `server`) and add
      `host = { path = "host" }` to `[workspace.dependencies]` (next to
      `common`/`storage`).

- [x] **Step 3: Write the failing tests** in `host/src/lib.rs`. Env-mutation
      tests serialize on a module `Mutex` (nextest gives process isolation, but
      a plain `cargo test` runs threads in one process — guard it):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn stream_filenames_are_the_convention() {
        assert_eq!(Stream::Mail.filename(), "mail.jsonl");
        assert_eq!(Stream::WebSub.filename(), "websub.jsonl");
        assert_eq!(Stream::Diag.filename(), "diag.log");
    }

    #[test]
    fn stream_parse_accepts_keys_and_rejects_unknown() {
        assert_eq!(Stream::parse("mail"), Some(Stream::Mail));
        assert_eq!(Stream::parse("websub"), Some(Stream::WebSub));
        assert_eq!(Stream::parse("diag"), Some(Stream::Diag));
        assert_eq!(Stream::parse("bogus"), None);
        assert_eq!(Stream::parse(""), None);
    }

    #[test]
    fn capture_path_joins_and_creates_dir_when_set() {
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("capture"); // does not exist yet
        std::env::set_var(CAPTURE_DIR_ENV, &dir);
        let p = capture_path(Stream::Mail).expect("some when set");
        assert_eq!(p, dir.join("mail.jsonl"));
        assert!(dir.is_dir(), "capture_path must create the dir");
        std::env::remove_var(CAPTURE_DIR_ENV);
    }

    #[test]
    fn capture_path_is_none_when_unset_or_blank() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::remove_var(CAPTURE_DIR_ENV);
        assert_eq!(capture_path(Stream::Diag), None);
        std::env::set_var(CAPTURE_DIR_ENV, "   ");
        assert_eq!(capture_path(Stream::Diag), None, "blank ⇒ None");
        std::env::remove_var(CAPTURE_DIR_ENV);
    }
}
```

Add `tempfile` as a `[dev-dependencies]` entry in `host/Cargo.toml`
(`tempfile = { workspace = true }` if the workspace pins it, else the version
`common`/`server` use).

- [x] **Step 4: Run the tests, verify they fail**

Run: `cargo nextest run -p host` Expected: FAIL — `host` items not yet defined.

- [x] **Step 5: Implement the library** in `host/src/lib.rs` to the Interfaces
      above. Every branch is pinned by Step 3 (filename mapping, parse
      accept/reject, set→join+mkdir, unset/blank→None), so the body follows from
      the tests. The only non-test-pinned choice is discarding the
      `create_dir_all` error (`let _ = std::fs::create_dir_all(&dir);`) — the
      writer's file open surfaces any real failure, matching today's behavior.
      Crate-level doc comment: state it is the strictly-host-focused shared
      crate, sibling to `common`, and that `capture_dir`/`capture_path` back the
      `JAUNDER_CAPTURE_DIR` contract (spec §"the new `host` crate").

- [x] **Step 6: Run the tests, verify they pass**

Run: `cargo nextest run -p host` Expected: PASS.

- [x] **Step 7: Commit**

```bash
git add host/ Cargo.toml Cargo.lock
git commit -m "feat(host): add host crate with JAUNDER_CAPTURE_DIR capture helper (#227)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

## Task 3: Migrate the mailer read site to injected path

Convert `build_mailer` from reading the env to **receiving** the capture path;
the `serve` root resolves it via `host`.

**Files:**

- Modify: `server/Cargo.toml` (add `host = { workspace = true }`),
  `server/src/mailer/mod.rs:40-43` (`build_mailer` signature + capture branch),
  `:32` + `server/src/mailer/file.rs:7` (doc comments),
  `server/src/commands.rs:432` (root passes the resolved path),
  `server/tests/storage/storage.rs:1481` (test caller passes `None`)
- Test: in-file `#[cfg(test)]` in `server/src/mailer/mod.rs`

**Interfaces:**

- Consumes: `host::{capture_path, Stream}` (Task 2).
- Produces:
  `build_mailer(site_config: &dyn SiteConfigStorage, mail_capture: Option<PathBuf>) -> Arc<dyn MailSender>`
  — `Some` ⇒ `FileMailSender`, `None` ⇒ SMTP/noop.

- [x] **Step 1: Add the dep** — `host = { workspace = true }` in
      `server/Cargo.toml` `[dependencies]`.

- [x] **Step 2: Write the failing test** — the capture-file branch of
      `build_mailer` (spec AC2) is currently untested. Inject the path via an
      `rstest` fixture — **no env, no lock**:

```rust
#[fixture]
fn capture_dir() -> tempfile::TempDir { tempfile::tempdir().unwrap() }

#[rstest]
#[tokio::test]
async fn build_mailer_selects_file_sender_when_path_given(capture_dir: tempfile::TempDir) {
    let path = capture_dir.path().join("mail.jsonl");
    let site = /* the no-config SiteConfigStorage test double this module already uses */;
    let mailer = build_mailer(&site, Some(path.clone())).await;
    // Send one message via the returned sender and assert `path` now exists
    // (use the module's existing mail-send/capture helper if present).
    assert!(path.exists());
}
```

(The module's test block currently uses plain `#[tokio::test]` + `use super::*`
— **add `use rstest::*;`** for the `#[fixture]`/`#[rstest]` macros, `rstest`
being a dev-dep already. The config double is `MapConfigStore`; build the
`EmailMessage` inline as the existing tests do — there is no reusable
mail-send/capture helper. The existing no-SMTP / SMTP-present tests (`:79`,
`:97`) just gain a `None` second arg.)

- [x] **Step 3: Run it, verify it fails**

Run:
`cargo nextest run -p jaunder build_mailer_selects_file_sender_when_path_given`
Expected: FAIL — signature has no second param yet.

- [x] **Step 4: Implement** — change the signature to
      `build_mailer(site_config: &dyn SiteConfigStorage, mail_capture: Option<std::path::PathBuf>)`
      and replace the `std::env::var(...)` branch with:

```rust
if let Some(path) = mail_capture {
    return Arc::new(FileMailSender::new(path)) as Arc<dyn MailSender>;
}
```

Update the `serve` root at `commands.rs:432` to
`build_mailer(db.site_config.as_ref(), host::capture_path(host::Stream::Mail)).await`;
update the dual-backend test at `storage.rs:1481` to pass `None`. Update the
`build_mailer` doc (mod.rs:32) and `FileMailSender` doc (file.rs:7) to the
capture-dir contract (the _seam_ no longer names any env var — the root does).

- [x] **Step 5: Run the tests, verify they pass**

Run: `cargo nextest run -p jaunder mailer` Expected: PASS (new test + existing
mailer tests).

- [x] **Step 6: Commit**

```bash
git add server/Cargo.toml Cargo.lock server/src/mailer/ server/src/commands.rs server/tests/storage/storage.rs
git commit -m "refactor(mailer): inject capture path, resolved from JAUNDER_CAPTURE_DIR (#227)"
```

---

## Task 4: Migrate the websub read site to injected path

Convert `default_client_from_env` → `default_client(Option<PathBuf>)`; the
`serve` root resolves the path.

**Files:**

- Modify: `server/src/websub/mod.rs:34-40` (`default_client_from_env` →
  `default_client`, signature + body), `:29/:31` +
  `server/src/websub/file_capture.rs:8` (doc comments),
  `server/src/websub/mod.rs:46-70` (rewrite the test),
  `server/src/commands.rs:422` (root passes the resolved path)
- Test: in-file `#[cfg(test)]` in `server/src/websub/mod.rs`

**Interfaces:**

- Consumes: `host::{capture_path, Stream}`.
- Produces:
  `default_client(websub_capture: Option<PathBuf>) -> Arc<dyn WebSubClient>` —
  `Some` ⇒ `FileCapturingWebSubClient`, `None` ⇒ `HttpWebSubClient`.

- [x] **Step 1: Rewrite the failing test** — the existing
      `selects_file_capture_when_env_set_else_http` (mod.rs:51) keys on
      `const ENV_KEY` + `set_var`. Replace it with an injection test — **no env,
      no lock**: `default_client(Some(dir.join("websub.jsonl")))` yields
      `FileCapturingWebSubClient`; `default_client(None)` yields
      `HttpWebSubClient`. Drop the `ENV_KEY` const and the
      `set_var`/`remove_var`. The module currently uses plain `#[tokio::test]` +
      `use super::*` — **add `use rstest::*;`** and reuse the temp-dir
      `#[fixture]` shape from Task 3.

- [x] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p jaunder websub` Expected: FAIL — signature has no
param yet / `default_client` undefined.

- [x] **Step 3: Implement** — rename to
      `default_client(websub_capture: Option<std::path::PathBuf>)` and:

```rust
if let Some(path) = websub_capture {
    std::sync::Arc::new(FileCapturingWebSubClient::new(path))
} else {
    std::sync::Arc::new(HttpWebSubClient::new())
}
```

Update the `serve` root at `commands.rs:422` to
`default_client(host::capture_path(host::Stream::WebSub))`. Update the
`default_client` doc (mod.rs:29-32) and `FileCapturingWebSubClient` doc
(file_capture.rs:8) to the capture-dir contract.

- [x] **Step 4: Run, verify pass**

Run: `cargo nextest run -p jaunder websub` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add server/src/websub/ server/src/commands.rs
git commit -m "refactor(websub): inject capture path, resolved from JAUNDER_CAPTURE_DIR (#227)"
```

---

## Task 5: Migrate the diag-log read site to `host` (filename `diag.log`)

Diag is **not** injected — it is resolved inside `init_tracing_impl` (the
observability bootstrap that already reads a family of env vars and installs the
process-global subscriber). Its tests keep the module's existing `lock_env()`
(required for the global subscriber regardless of capture). See spec Decision 5.

**Files:**

- Modify: `server/src/observability.rs:65` (`diag_log_file`), doc comments
  (`:59-63`, `:89`, `:150`, `:334`), diag env-tests (`:624-627`, `:849-872`)
- Test: in-file `#[cfg(test)]` in `server/src/observability.rs`

**Interfaces:**

- Consumes: `host::{capture_path, Stream}`. Note `trimmed_non_empty` (`:43`)
  stays — it still backs the OTLP-endpoint read at `:52`; only the diag read at
  `:65` migrates.

- [x] **Step 1: Update the failing tests** — the observability tests that
      `set_var`/`remove_var("JAUNDER_DIAG_LOG_FILE")` and assert the diag file
      is written (around `:849-872`) switch to setting `host::CAPTURE_DIR_ENV`
      at a `TempDir`, and assert the diag records land at `<dir>/diag.log` (was
      `jaunder-diag.log`). **Keep the module's existing `lock_env()`** — it
      already serializes these tests and is needed for the global-subscriber
      install regardless; just swap the var and filename (do NOT introduce any
      cross-module lock). Also update the stale `jaunder-diag.log` literal in
      the explicit-path test at `observability.rs:666` to `diag.log` for
      consistency (it passes a path directly, so it doesn't fail AC1, but leave
      no stale filename behind).

  **Note the one non-mechanical conversion:**
  `init_tracing_impl_survives_unopenable_diag_path` (`:863`) currently makes the
  path unopenable by pointing `JAUNDER_DIAG_LOG_FILE` at a _directory_. Under
  the new scheme `host::capture_path(Diag)` `create_dir_all`s `<dir>` and joins
  `diag.log`, so that no longer fails. Reconstruct "unopenable" a different way
  — e.g. set `JAUNDER_CAPTURE_DIR` to a path that is an existing **regular
  file** (so `create_dir_all` / the `<file>/diag.log` open fails). The test's
  intent (open failure disables the sink, startup survives) is unchanged.

- [x] **Step 2: Run, verify fail**

Run: `cargo nextest run -p jaunder observability` Expected: FAIL — reads old var
/ expects old filename.

- [x] **Step 3: Implement** — replace `diag_log_file`'s body:

```rust
fn diag_log_file() -> Option<std::path::PathBuf> {
    host::capture_path(host::Stream::Diag)
}
```

Update the surrounding doc comments (`:59-63` etc.) to name
`JAUNDER_CAPTURE_DIR` and the `diag.log` filename.

- [x] **Step 4: Run, verify pass**

Run: `cargo nextest run -p jaunder observability` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add server/src/observability.rs
git commit -m "refactor(observability): scoped diag log via JAUNDER_CAPTURE_DIR/diag.log (#227)"
```

---

## Task 6: `test-support` — derive `reset-mail` + add `capture-path`

**Files:**

- Modify: `test-support/Cargo.toml` (add `host = { workspace = true }`),
  `test-support/src/main.rs` (`ResetMail` arm `:72-76`, new `CapturePath`
  variant, dispatch)
- Test: `test-support/tests/cli.rs`

**Interfaces:**

- Consumes: `host::{capture_path, Stream, CAPTURE_DIR_ENV}`.
- Produces (CLI other tasks depend on): `test-support reset-mail` (no args);
  `test-support capture-path <mail|websub|diag>` → prints the absolute path on
  stdout, non-zero on unset dir / unknown stream.

- [x] **Step 1: Add the dep** — `host = { workspace = true }` in
      `test-support/Cargo.toml`.

- [x] **Step 2: Update/add the failing subprocess tests** in
      `test-support/tests/cli.rs` (spawns the built binary, mutates only the
      child's env):

```rust
// reset-mail: derives <dir>/mail.jsonl and deletes it
#[test]
fn reset_mail_deletes_derived_capture_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("capture");
    std::fs::create_dir_all(&dir).unwrap();
    let mail = dir.join("mail.jsonl");
    std::fs::write(&mail, "x\n").unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .arg("reset-mail")
        .env("JAUNDER_CAPTURE_DIR", &dir)
        .status().unwrap();
    assert!(status.success());
    assert!(!mail.exists());
}

// reset-mail: loud failure when the dir is unset
#[test]
fn reset_mail_errors_without_capture_dir() {
    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .arg("reset-mail").env_remove("JAUNDER_CAPTURE_DIR").output().unwrap();
    assert!(!out.status.success());
}

// capture-path: prints the derived absolute path
#[test]
fn capture_path_prints_derived_path() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("capture");
    let out = Command::new(env!("CARGO_BIN_EXE_test-support"))
        .args(["capture-path", "mail"]).env("JAUNDER_CAPTURE_DIR", &dir).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), dir.join("mail.jsonl").to_string_lossy());
}
```

(Match `cli.rs`'s existing spawn idiom — use its binary-path constant if it
differs from `CARGO_BIN_EXE_test-support`.)

- [x] **Step 3: Run, verify fail**

Run: `cargo nextest run -p test-support` Expected: FAIL — `--path` still
required / `capture-path` unknown.

- [x] **Step 4: Implement** in `test-support/src/main.rs`:
  - `ResetMail` loses its `#[arg(long, env = "JAUNDER_MAIL_CAPTURE_FILE")] path`
    field (becomes a unit variant). Its arm:
    `let path = host::capture_path(host::Stream::Mail).context("JAUNDER_CAPTURE_DIR is not set")?; if path.exists() { std::fs::remove_file(&path)?; } Ok(())`.
  - Add `CapturePath { stream: String }`. Arm:
    `let s = host::Stream::parse(&stream).with_context(|| format!("unknown capture stream {stream:?}"))?; let p = host::capture_path(s).context("JAUNDER_CAPTURE_DIR is not set")?; println!("{}", p.display()); Ok(())`.
  - Note the `cov:ignore`/`crap:allow` context around `main` (`:81`) — keep
    `capture-path`/`reset-mail` covered by `cli.rs` (they take no DB, so they
    stay outside the DB-only `cov:ignore` block).

- [x] **Step 5: Run, verify pass**

Run: `cargo nextest run -p test-support` Expected: PASS.

- [x] **Step 6: Commit**

```bash
git add test-support/Cargo.toml Cargo.lock test-support/src/main.rs test-support/tests/cli.rs
git commit -m "feat(test-support): derive reset-mail + add capture-path via JAUNDER_CAPTURE_DIR (#227)"
```

---

## Task 7: `devtool seed-e2e` — drop the `--mail-file` plumbing

**Files:**

- Modify: `tools/devtool/src/main.rs:74` (drop `mail_file` field), `:122`
  (call), `tools/devtool/src/seed_e2e.rs:17/58/64-65` (drop `mail_file` param;
  `reset-mail` step arg-less), `:131` (unit test)
- Test: in-file `#[cfg(test)]` in `tools/devtool/src/seed_e2e.rs`

**Interfaces:**

- Consumes: `test-support reset-mail` now argument-less (Task 6). devtool is in
  the separate `tools/` workspace (no `host` link); it relies on
  `JAUNDER_CAPTURE_DIR` being in its inherited env (set by callers, Tasks 8/9),
  which the spawned `test-support` inherits (`Command` without `env_clear`).

- [x] **Step 1: Update the failing unit test** — `seed_invocations`'s test
      (`:131`) currently asserts a `reset-mail --path /tmp/mail.jsonl` step.
      Change the expected last step to `["reset-mail"]` and drop the `mail_file`
      argument from the `seed_invocations(...)` call in the test.

- [x] **Step 2: Run, verify fail**

Run: `cargo nextest run --manifest-path tools/Cargo.toml` Expected: FAIL —
signature/args mismatch.

- [x] **Step 3: Implement**:
  - `seed_e2e.rs`: `fn seed_invocations() -> Vec<(Vec<String>, bool)>` (drop
    `mail_file`); last step `step(&["reset-mail"])`.
    `pub fn run(db: &str, test_support_bin: &Path)` (drop `mail_file`), call
    `seed_invocations()`.
  - `main.rs`: drop the `mail_file: String` field from the `SeedE2e` args struct
    (`:74`) and drop it from the
    `seed_e2e::run(&args.db, &args.test_support_bin)` call (`:122`).

- [x] **Step 4: Run, verify pass**

Run: `cargo nextest run --manifest-path tools/Cargo.toml` Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add tools/devtool/src/main.rs tools/devtool/src/seed_e2e.rs tools/Cargo.lock
git commit -m "refactor(devtool): seed-e2e resets mail via JAUNDER_CAPTURE_DIR, drop --mail-file (#227)"
```

---

## Task 8: Host e2e driver — capture-dir wiring (`xtask/src/steps/e2e_local.rs`)

**Files:**

- Modify: `xtask/src/steps/e2e_local.rs` (`:14-22` module doc, `:93-95` path
  vars, `:102-108` server env, `:151-154` seed cmd, `:189-195` Playwright env)

**Interfaces:**

- Consumes: server reads `JAUNDER_CAPTURE_DIR` (Tasks 3-5); `devtool seed-e2e`
  no longer takes `--mail-file` (Task 7); `test-support capture-path` on PATH
  (Task 6).

- [x] **Step 1: Implement** (this task's deliverable is verified by running it,
      so implement then run):
  - Replace `let mail/websub/diag = format!("{sp}/…")` with
    `let capture = format!("{sp}/capture");`.
  - Server `Command`: drop the three `.env("JAUNDER_*_FILE", …)`; add
    `.env("JAUNDER_CAPTURE_DIR", &capture)`.
  - Seed `cmd!`: drop `--mail-file {mail}`; add
    `.env("JAUNDER_CAPTURE_DIR", &capture)` (the seed subprocess and the
    `test-support` it spawns inherit it).
  - Playwright `cmd!`: drop the two `.env("JAUNDER_*_CAPTURE_FILE", …)`; add
    `.env("JAUNDER_CAPTURE_DIR", &capture)`.
  - Update the module doc (`:11-22`) canonical env list: `JAUNDER_CAPTURE_DIR`
    replaces the three `_FILE` vars.

- [x] **Step 2: Build the driver, verify it compiles**

Run: `cargo build -p xtask` (or `devtool run -- cargo xtask --help`) Expected:
builds clean.

- [x] **Step 3: Behavioral check (defer heavy run):** the end-to-end exercise of
      this task happens together with the TS delegation in Task 9 and the full
      matrix in Task 13. Note here that a single-spec host run
      (`cargo xtask e2e-local`, mail spec) is the targeted check once Task 9
      lands.

- [x] **Step 4: Commit**

```bash
git add xtask/src/steps/e2e_local.rs
git commit -m "refactor(xtask): host e2e driver sets JAUNDER_CAPTURE_DIR (#227)"
```

---

## Task 9: TS readers delegate to `test-support capture-path`

**Files:**

- Create: `end2end/tests/capture.ts`
- Modify: `end2end/tests/mail.ts:26-27` + usages,
  `end2end/tests/websub.ts:24-25` + usages

**Interfaces:**

- Consumes: `test-support capture-path <stream>` (Task 6), `JAUNDER_CAPTURE_DIR`
  in the Playwright env (Task 8 host / Task 10 VM).
- Produces: `capturePathViaTool(stream: "mail" | "websub"): string`.

- [x] **Step 1: Implement `end2end/tests/capture.ts`** — mirror `seed.ts:30`'s
      `execFileSync("test-support", …, { env: process.env })`:

```ts
import { execFileSync } from "node:child_process";

/** Resolve a capture-file absolute path by asking the `test-support` binary,
 *  so the filename convention lives only in the Rust `host` crate. */
export function capturePathViaTool(stream: "mail" | "websub"): string {
  return execFileSync("test-support", ["capture-path", stream], {
    stdio: "pipe",
    env: process.env,
  })
    .toString()
    .trim();
}
```

- [x] **Step 2: Update `mail.ts`** — replace the `MAIL_CAPTURE_FILE` module
      const (`:26-27`) with a memoized accessor, and swap the four usages
      (`:38,40,66`) to call it:

```ts
import { capturePathViaTool } from "./capture";
let _mailFile: string | undefined;
function mailCaptureFile(): string {
  return (_mailFile ??= capturePathViaTool("mail"));
}
```

(Resolving lazily avoids a subprocess at import time. Update the file's header
comment that referenced `JAUNDER_MAIL_CAPTURE_FILE`.)

- [x] **Step 3: Update `websub.ts`** — same treatment: memoized
      `websubCaptureFile()` via `capturePathViaTool("websub")`, swap usages
      (`:35,37,64,91`), update the header comment (`:24-25`, `:4`).

- [x] **Step 4: Typecheck**

Run: from `end2end/`, `npx tsc --noEmit -p tsconfig.json` (there is no npm
typecheck script — `package.json` `scripts` is empty; `typescript` is a devDep
and `tsconfig.json` is present, so this is the real command. Playwright
transpiles TS at runtime, so this is a separate manual gate.) Expected: PASS (no
type errors; no remaining `JAUNDER_*_CAPTURE_FILE` refs).

- [x] **Step 5: Behavioral check** — with Tasks 6 + 8 landed, a targeted host
      run exercises the delegation:

Run: `cargo xtask e2e-local` filtered to a mail spec (per the driver's
`test_filter`, e.g. the email-verification spec). Expected: PASS — server writes
`<tmp>/capture/mail.jsonl`, `mail.ts` reads the same path via `capture-path`.

- [x] **Step 6: Commit**

```bash
git add end2end/tests/capture.ts end2end/tests/mail.ts end2end/tests/websub.ts
git commit -m "refactor(e2e): TS readers resolve capture paths via test-support capture-path (#227)"
```

---

## Task 10: `flake.nix` — single var, whole-dir copy-out, panic-gate path, seed

**Files:**

- Modify: `flake.nix` — `mailCaptureEnv` (`:52-60`), `e2eRunAndCapture`
  Playwright cmd (`:641-642`), diag copy-out (`:684-688`), `e2ePanicGate`
  (`:595`), seed calls (`:768-772`, `:902-907`)

**Interfaces:**

- Consumes: server reads `JAUNDER_CAPTURE_DIR`; `capture/diag.log`;
  `devtool seed-e2e` argument-less mail reset.

- [x] **Step 1: Implement**:
  - `mailCaptureEnv` → a single
    `JAUNDER_CAPTURE_DIR = "/var/lib/jaunder/capture";`. **Keep the binding name
    `mailCaptureEnv`** (only change its value + the comment noting #227 is done)
    — renaming it would force touching its three consumers at
    `flake.nix:181/:749/:859`, needless churn.
  - Playwright cmd: replace the two `+ " JAUNDER_*_CAPTURE_FILE=…"` lines with
    `+ " JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture"`.
  - Copy-out: delete the per-file diag rename+`_grab` (`:684-688`); replace with
    a **tarball** of the capture dir, mirroring the existing
    `playwright-artifacts` pattern (`:678`) — a _file_ copy, the proven
    `copy_from_vm` shape (a whole-directory-source copy is not used anywhere in
    this flake, so don't introduce one):
    ```python
    machine.execute("test -d /var/lib/jaunder/capture && tar czf /tmp/capture-${backend}.tar.gz -C /var/lib/jaunder capture 2>/dev/null || true")
    _grab("/tmp/capture-${backend}.tar.gz")
    ```
  - `e2ePanicGate`: change the diag read (`:595`) to
    `cat /var/lib/jaunder/capture/diag.log`.
  - Both `devtool seed-e2e` calls: drop
    `+ " --mail-file /var/lib/jaunder/mail.jsonl"`; prefix the command with
    `JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture ` so the seed's `reset-mail`
    derives the path.

- [x] **Step 2: Evaluate the flake, verify it parses**

Run: `devtool run -- nix flake check --no-build` (or
`nix eval .#nixosConfigurations` as the repo does for flake syntax). Note: full
VM build happens in Task 13. Expected: evaluates without error.

- [x] **Step 3: Commit**

```bash
git add flake.nix
git commit -m "refactor(e2e): flake sets one JAUNDER_CAPTURE_DIR, copies capture/ out per combo (#227)"
```

---

## Task 11: `xtask/src/steps/nix.rs` — lift the `capture-<backend>.tar.gz` tarball

**Files:**

- Modify: `xtask/src/steps/nix.rs:128-145` (`copy_e2e_diagnostics_between`
  filter + doc), `:597-646` (unit test)
- Test: in-file `#[cfg(test)]` in `xtask/src/steps/nix.rs`

**Interfaces:**

- Consumes: the flake now emits a `capture-<backend>.tar.gz` file (Task 10)
  instead of `jaunder-diag-<backend>.log`.

- [x] **Step 1: Update the failing unit test** —
      `copy_e2e_diagnostics_between_copies_journal_otel_and_playwright`
      (`:598`): drop the `jaunder-diag-sqlite.log` file + the
      bare-`jaunder-diag.log` "must not lift" assertion; instead create a
      `capture-sqlite.tar.gz` file and assert it is copied (a flat-file copy,
      like `playwright-artifacts-*.tar.gz`). Update the expected count.

- [x] **Step 2: Run, verify fail**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml copy_e2e_diagnostics_between`
Expected: FAIL — filter still keys on `jaunder-diag-`.

- [x] **Step 3: Implement** — in the `wanted` closure (`:138-145`), drop the
      `jaunder-diag-` line and add
      `|| (name.starts_with("capture-") && name.ends_with(".tar.gz"))`. Update
      the doc comment (`:128-134`) to describe the capture-dir tarball replacing
      the scoped diag log.

- [x] **Step 4: Run, verify pass**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml copy_e2e_diagnostics_between`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add xtask/src/steps/nix.rs
git commit -m "refactor(xtask): lift the capture-<backend> directory as an e2e artifact (#227)"
```

---

## Task 12: Docs + decision records

**Files:**

- Modify: `docs/observability.md` diag-log section (var at `:64`, plus the stale
  VM path `:56` and lifted-name `:57`), `CONTRIBUTING.md:742` (prod-warning) +
  `:301-302,306` (diag-artifact path),
  `docs/adr/0049-app-driven-scoped-server-diagnostics.md` (status)
- Create: ADR draft(s) under `docs/adr/drafts/` (numberless, per
  **jaunder-adr**)

- [x] **Step 1: Update `docs/observability.md`** — the diag-log section: var
      `JAUNDER_CAPTURE_DIR`, path `capture/diag.log`.
- [x] **Step 2: Update `CONTRIBUTING.md`** — the prod-warning (`:742`): don't
      set `JAUNDER_CAPTURE_DIR` in production (test-only). The diag-artifact doc
      (`:301-302,306`): lift target `capture-<backend>.tar.gz` (contains
      `diag.log`), VM path `/var/lib/jaunder/capture/diag.log`.
- [x] **Step 3: Author the capture-dir ADR draft** (**jaunder-adr**) in
      `docs/adr/drafts/`: the `JAUNDER_CAPTURE_DIR` output-dir contract,
      convention filenames, whole-dir lift; state the diag/panic-hook trigger
      var changed from `JAUNDER_DIAG_LOG_FILE` to `JAUNDER_CAPTURE_DIR`.
- [x] **Step 4: Author the `host`-crate-layering ADR draft**: `host` =
      strictly-host shared crate, sibling to target-agnostic `common`, with a
      future strictly-client crate as its peer. (Keep separate from the
      capture-dir ADR — this is the broader structural convention.)
- [x] **Step 5: Mark ADR-0049 superseded-in-part** — a status note + forward
      cross-reference at its top pointing to the capture-dir ADR (its body line
      ~79 "installed only when `JAUNDER_DIAG_LOG_FILE` is set" is now false).
      Annotate, don't rewrite the body (**jaunder-adr** status-change flow).
- [x] **Step 6: Prettier-format** the edited Markdown before staging (repo
      pre-commit runs `prettier -w`; format first to avoid the fail-restage
      double commit).
- [x] **Step 7: Commit**

```bash
git add docs/observability.md CONTRIBUTING.md docs/adr/
git commit -m "docs(e2e): document the JAUNDER_CAPTURE_DIR contract + host crate, supersede-in-part ADR-0049 (#227)"
```

---

## Task 13: Full-gate verification (acceptance)

The clean break + e2e wiring are only proven together. This task runs the
acceptance gate (spec AC1, AC6-AC10).

**Files:** none (verification only).

- [x] **Step 1: Clean-break sweep (AC1)**

Run:
`rg 'JAUNDER_(MAIL|WEBSUB)_CAPTURE_FILE|JAUNDER_DIAG_LOG_FILE' server/ test-support/ xtask/ tools/ end2end/ flake.nix docs/observability.md CONTRIBUTING.md`
Expected: no matches.

- [x] **Step 2: Full local gate + e2e matrix (AC6-AC10)**

Run: `devtool run -- cargo xtask validate` (background mode — long). Confirm the
`xtask-done: … ok=true` sentinel and green across all four
`{sqlite,postgres}×{chromium,firefox}` combos (mail, websub, and diag/panic-gate
specs exercised). Expected: PASS.

- [x] **Step 3:** If green, the cycle is ready for **jaunder-ship** (final
      review, archive spec+plan, promote ADR drafts, PR, merge, release the
      issue to Done). No commit — this task is a gate.

---

## Self-review notes

- **Spec coverage:** AC1→Task 13/Global Constraints; AC2→Task 3; AC3→Task 4;
  AC4→Task 5; AC5 (single-source filenames, no free-path args, loud-fail)→Tasks
  2/6/9; AC6→Task 13; AC7→Task 11; AC8→Tasks 5+10; AC9→Task 8; AC10→Tasks 3-5.
  Out-of-scope otel-traces→Task 1.
- **Type consistency:** `host::capture_path(host::Stream::{Mail,WebSub,Diag})`
  used at the composition roots (Tasks 3/4 pass its `Option<PathBuf>` into
  `build_mailer`/`default_client`; Task 5 calls it inside `init_tracing_impl`;
  Task 6 in `test-support`); `host::CAPTURE_DIR_ENV` used in Task 2/6.
  `build_mailer(_, Option<PathBuf>)` and `default_client(Option<PathBuf>)`
  signatures match their callers and the `rstest` fixture tests.
  `capturePathViaTool` (Task 9) matches the `capture-path` CLI (Task 6).
- **Ordering:** Task 2 (host) gates 3-6; 6 gates 7 and 9; 8 gates 9's behavioral
  check; 10 depends on 3-5 + 7; 11 depends on 10; 13 last. Intermediate commits
  stay green under `cargo xtask check` (no e2e); e2e consistency is proven only
  at Task 13.
