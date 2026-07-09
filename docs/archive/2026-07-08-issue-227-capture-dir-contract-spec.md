# Spec — Consolidate capture-to-file env vars into a `JAUNDER_CAPTURE_DIR` contract (#227)

**Status:** approved **Issue:** jaunder-org/jaunder#227 **Date:** 2026-07-08

## Problem

The server has three independent "write this stream to a file" env vars, each
read directly and each wired separately into every e2e surface:

| Env var                       | Server read site                                     | Written by | Read/consumed by                                                         |
| ----------------------------- | ---------------------------------------------------- | ---------- | ------------------------------------------------------------------------ |
| `JAUNDER_MAIL_CAPTURE_FILE`   | `server/src/mailer/mod.rs` `build_mailer`            | server     | Playwright `mail.ts`; host seed `--mail-file`; `test-support reset-mail` |
| `JAUNDER_WEBSUB_CAPTURE_FILE` | `server/src/websub/mod.rs` `default_client_from_env` | server     | Playwright `websub.ts`                                                   |
| `JAUNDER_DIAG_LOG_FILE`       | `server/src/observability.rs` `diag_log_file`        | server     | server-only; lifted as an artifact for the e2e panic gate                |

Every new capture stream adds another `_FILE` var, another
`systemd.services.jaunder.environment` entry, another line in `flake.nix`'s
`e2eRunAndCapture`, and (for diag) another copy-out + a filename-prefix match in
the xtask artifact-lift filter. The sprawl surfaced in #144's design review and
is flagged as future work in ADR-0049.

## Goal

Replace the three per-stream `_FILE` vars with a single **output-dir contract**:
one env var `JAUNDER_CAPTURE_DIR` naming a dedicated directory into which each
capture stream writes a well-known filename by convention. Setting one var per
environment configures all streams; the e2e harness copies one directory out per
combo; adding a new capture stream needs **zero** new env-var or copy-out
plumbing.

This realizes the direction ADR-0049 flagged and matches #153's
convention-over-env ethos (delete env plumbing rather than add it).

## Decisions (settled in design)

1. **Clean break, no back-compat shim.** The three `_FILE` env vars are removed
   everywhere. They are e2e-only with no production or external consumers, so
   nothing outside this tree depends on them.
2. **Whole-directory copy-out (as a tarball).** The e2e harness tars
   `$JAUNDER_CAPTURE_DIR` out per combo as a per-backend
   `capture-<backend>.tar.gz`; the xtask lift matches that file. Adding a stream
   needs no lift change. This newly lifts `mail.jsonl`/`websub.jsonl` too
   (previously consumed only in-VM) — harmless and useful for post-mortem. (A
   tarball, not a raw directory-source `copy_from_vm` — see the e2e-wiring note;
   refined at plan stage to use the proven `playwright-artifacts` file-copy
   pattern.)
3. **Dedicated subdirectory, not the state root.** `JAUNDER_CAPTURE_DIR` points
   at a dedicated directory (`/var/lib/jaunder/capture` in the VM; a `capture/`
   subdir of the per-run temp dir on the host), **not** `/var/lib/jaunder`
   itself — otherwise the whole-dir copy would sweep up the DB, `runtime.json`,
   and otel traces. Forced by decision 2.
4. **Filenames by convention.** `mail.jsonl`, `websub.jsonl`, `diag.log`. The
   diag file drops the now-redundant `jaunder-` prefix (it lives inside a
   jaunder capture dir, and the lift no longer keys on a filename prefix).
5. **One shared helper, in a new `host` crate.** A single Rust helper owns the
   dir-var name, the filename constants, the trimmed-non-empty guard, and the
   defensive `create_dir_all`. It lives in a **new `host` workspace crate** —
   the strictly-host-focused sibling of `common` (which is deliberately
   target-agnostic and compiles to wasm; a future strictly-client crate would be
   its peer). `host` is chosen over `common` because this is host-only, e2e-only
   plumbing (`std::fs`/`std::env`) that has no business in the wasm frontend
   bundle — and `common` today has **zero** host-only
   `#[cfg(not(target_arch = "wasm32"))]` carve-outs, an invariant worth keeping.
   It is chosen over `server` because both `server` (which writes the streams)
   and `test-support` (which deletes the mail file in `reset-mail`) can depend
   on `host`, so the filename convention lives in exactly **one** Rust place.
   `host` never compiles to wasm, so the helper needs no cfg gate. The helper is
   invoked only at **composition roots** — where the env is read once and the
   result handed to a consumer — never deep inside a unit under test (see the
   injection design below): the roots are the `serve` command (mailer + websub),
   the observability bootstrap (diag), and the `test-support`
   `reset-mail`/`capture-path` entrypoints. The Playwright readers do **not**
   restate the filenames — they delegate to `test-support capture-path` (see the
   TS section), so the convention has no second home in TypeScript.

   **Inject the resolved path; don't read the env in a unit.** `build_mailer`
   and `default_client` (renamed from `default_client_from_env`) take an
   `Option<PathBuf>` — `Some(path)` selects the file-capturing transport, `None`
   (the production default, `JAUNDER_CAPTURE_DIR` unset) selects SMTP/noop and
   the live HTTP client. The `serve` composition root resolves each via
   `host::capture_path(Stream::…)` and passes it in. This keeps the
   process-global env out of these units entirely, so their tests inject a
   `TempDir` path as a value (a shared `rstest` fixture) — no `set_var`, no
   lock, race-free under both `cargo nextest` (process-per-test) **and** plain
   `cargo test --test <suite>` (in-process, threaded — a workflow
   `CONTRIBUTING.md` sanctions). The diag stream is the exception: it is
   resolved inside `init_tracing_impl` (the observability bootstrap) via
   `host::capture_path(Stream::Diag)`, alongside that function's sibling
   env-driven config (log-filter, slow-op, log-format, otel) — DI-ing only the
   diag path would be an inconsistent half-measure, and its tests must serialize
   on the existing module-wide `lock_env()` anyway because `init_tracing_impl`
   installs the process-global tracing subscriber + panic hook. Because
   mailer/websub no longer touch the env, **only** the observability tests set
   `JAUNDER_CAPTURE_DIR`, under that existing `lock_env()` — so no new
   cross-module lock is needed.

6. **No free-path CLI args — reset derives from the dir.**
   `test-support reset-mail` and `devtool seed-e2e` **drop their mail-file path
   argument entirely**. `reset-mail` derives `<JAUNDER_CAPTURE_DIR>/mail.jsonl`
   from the shared `host` helper; `seed-e2e` propagates `JAUNDER_CAPTURE_DIR` to
   the `test-support` subprocess it spawns (inherited env — `devtool` is a
   separate `tools/` workspace and cannot link `host`, but it never needs the
   filename, only to pass the dir through). Callers (flake, host driver)
   therefore set **only `JAUNDER_CAPTURE_DIR`** — never a filename — on the seed
   command, mirroring the server env. **Rationale:** a free
   `--path`/`--mail-file` arg is a "put the mail file anywhere" capability that
   no flow uses; keeping it would force each caller to re-hardcode `mail.jsonl`,
   recreating the exact per-file second source of truth #227 exists to delete.
   `reset-mail`'s subprocess test (`test-support/tests/cli.rs`) sets
   `JAUNDER_CAPTURE_DIR` on the spawned child's env (a `TempDir`) — child-only,
   no process-global `set_var` — while the `host` helper's own in-process test
   drives both branches directly.

## Design

### Rust: the new `host` crate + capture-path helper

Create a new `host` workspace crate — a home for **strictly-host-focused**
shared code (the first tenant is this capture-dir contract; a future
strictly-client crate would be its symmetric peer). This means: a
`host/Cargo.toml` added as a workspace member in the root `Cargo.toml`, a
`workspace.dependencies` entry, and `server` and `test-support` taking a
`host = { workspace = true }` dependency. **No explicit gate wiring is needed:**
the Nix coverage source filter auto-admits any new top-level crate, and
coverage/clippy run workspace-wide (no per-crate enumeration), so `host`'s tests
are picked up simply by its being a workspace member. Because `host` never
targets wasm, its code needs no `#[cfg(not(target_arch = "wasm32"))]` gating.

The crate's capture module exposes:

- The env-var name `JAUNDER_CAPTURE_DIR` in exactly one place.
- Filename constants for the three streams (`mail.jsonl`, `websub.jsonl`,
  `diag.log`).
- A function that, given a well-known filename, returns `Some(dir.join(name))`
  when `JAUNDER_CAPTURE_DIR` is set to a non-blank value (trimmed), else `None`;
  it `create_dir_all`s the capture directory so writers can open the file.
  Unset/blank ⇒ `None` ⇒ the whole capture feature is inert (production
  default), preserving today's behavior.

The call sites:

- `serve` composition root: resolves `host::capture_path(Stream::Mail)` and
  `host::capture_path(Stream::WebSub)` and passes each into the seam below.
- `build_mailer(site_config, mail_capture: Option<PathBuf>)`: `Some` →
  `FileMailSender`; `None` → SMTP/noop. (Injected — no env read inside.)
- `default_client(websub_capture: Option<PathBuf>)` (renamed from
  `default_client_from_env`): `Some` → `FileCapturingWebSubClient`; `None` →
  `HttpWebSubClient`. (Injected.)
- `init_tracing_impl` (observability bootstrap): resolves
  `host::capture_path(Stream::Diag)` internally, alongside its sibling
  env-driven config; `Some` → install the diag layer + panic hook; `None` →
  inert. (Not injected — see the injection design note above.)
- `test-support reset-mail`: derives the mail file via
  `host::capture_path(Stream::Mail)`. It **errors (non-zero exit) when
  `JAUNDER_CAPTURE_DIR` is unset/blank** (`None`) — see the loud-failure risk
  below; a validly-derived-but-absent file stays a no-op (nothing to reset).
  This differs from the server read sites, where `None` means "capture off"
  (inert): `reset-mail` is an e2e-only tool, so an unset dir is a
  misconfiguration to surface, not a feature toggle.
- `test-support capture-path <stream>` (new subcommand): prints the resolved
  absolute path for a stream (`mail`/`websub`/`diag`) via `host::capture_path`;
  a thin sibling of `reset-mail`, erroring the same way on an unset dir. This is
  what lets the TS readers ask for a path instead of restating filenames (see
  below).

Two stale doc-comments in server source name the old vars and must be updated to
the dir contract (AC1 greps `server/` and would otherwise fail on them):
`server/src/mailer/file.rs` and `server/src/websub/file_capture.rs`.

### TS: delegate path resolution to `test-support` (no filename literals)

Rather than restating `mail.jsonl`/`websub.jsonl` in TypeScript (a second,
cross-language source of truth for the filenames), the readers **ask
`test-support` for the path**. A small `capturePathViaTool(stream)` helper
shells out to `test-support capture-path <stream>` — mirroring the existing
`seedPostsViaTool` pattern (`end2end/tests/seed.ts:30` already does
`execFileSync("test-support", …, { env: process.env })`, so `test-support` is on
PATH and inherits `JAUNDER_CAPTURE_DIR` in both host and VM runs). `mail.ts` and
`websub.ts` replace their hardcoded `_FILE`-var read + `/tmp` fallback with this
call (resolved once and memoized). Net effect: the `mail.jsonl`/`websub.jsonl`
filenames live in **exactly one place** — the `host` crate — and TS shares only
a stable logical _stream key_ (`"mail"`/`"websub"`), immune to a future file
rename. The `/tmp` fallback disappears (an unset dir now surfaces as a
`capture-path` error, consistent with `reset-mail`).

### e2e wiring

- **`flake.nix`**
  - `mailCaptureEnv` collapses to a single
    `JAUNDER_CAPTURE_DIR = "/var/lib/jaunder/capture";` entry (renamed to
    reflect its new single-var role).
  - `e2eRunAndCapture`'s Playwright command sets one `JAUNDER_CAPTURE_DIR=...`
    instead of the two `_CAPTURE_FILE=...` lines.
  - Copy-out: replace the per-file diag rename+`_grab` with a single guarded
    **tarball** of the capture dir (`tar czf … capture` →
    `capture-${backend}.tar.gz`, then `_grab`), mirroring the existing
    `playwright-artifacts` pattern. (A tarball, not a raw directory-source
    `copy_from_vm` — every existing `copy_from_vm` in the flake copies a _file_
    source, so a whole-directory-source copy would be an unproven new pattern
    first exercised only at the full matrix. Refined from a raw dir at plan
    stage.)
  - `e2ePanicGate` reads `/var/lib/jaunder/capture/diag.log`.
  - The `devtool seed-e2e` invocations (both backend blocks) drop `--mail-file`
    and instead set `JAUNDER_CAPTURE_DIR=/var/lib/jaunder/capture` in the seed
    command's env, so the `reset-mail` step it spawns derives the mail path
    itself.
- **`xtask/src/steps/e2e_local.rs`** (host driver): one `capture/` subdir under
  the per-run temp dir; set `JAUNDER_CAPTURE_DIR` on the server, the Playwright
  process, **and the `seed-e2e` subprocess** (replacing the computed
  `--mail-file` arg). Remove the three `_FILE` envs. Update the module-doc
  canonical env list.
- **`tools/devtool` `seed-e2e`**: drop the `--mail-file` CLI arg, the
  `mail_file` field on its args struct (`main.rs`), and the `mail_file`
  parameter threaded through `run` / `seed_invocations` (`seed_e2e.rs`); the
  `reset-mail` step becomes argument-less. Update its `seed_invocations` unit
  test.
- **`test-support/src/main.rs`**: `reset-mail` loses its `--path` arg; it now
  derives the target from `JAUNDER_CAPTURE_DIR` via the `host` helper and errors
  when unset. **Add a `capture-path <stream>` subcommand** that prints the
  `host`-resolved absolute path for a stream and errors the same way on an unset
  dir.
- **`test-support/tests/cli.rs`**: the out-of-process `reset-mail` smoke test
  currently spawns `test-support reset-mail --path <tmpfile>`. It switches to
  setting `JAUNDER_CAPTURE_DIR=<TempDir>` on the child `Command` and
  writing/asserting `<dir>/mail.jsonl` — a subprocess test, so it mutates only
  the child's env, no process-global `set_var`. (This is the coverage-merge test
  for `main`'s `ResetMail` arm, per #232.) Extend it to cover the `capture-path`
  arm the same subprocess way.
- **`end2end/tests/mail.ts` + `websub.ts`**: replace the `_FILE`-var read +
  `/tmp` fallback with `capturePathViaTool("mail"|"websub")` (a new small helper
  shelling `test-support capture-path`, mirroring `seed.ts`'s
  `seedPostsViaTool`). No filename literals remain in TS.
- **`xtask/src/steps/nix.rs`** (artifact lift): drop the `jaunder-diag-`
  filename match; add a `capture-*.tar.gz` match, lifted as a flat file (like
  `playwright-artifacts-*.tar.gz`). Update the doc comment and the unit test.

### Docs & decision record

- `docs/observability.md` and `CONTRIBUTING.md` updated to the new var/paths. In
  `CONTRIBUTING.md` this is **both** the prod-warning note (~line 742, the mail
  var) **and** the diag-artifact documentation (~lines 301-302, 306): the lift
  target changes from
  `.xtask/diagnostics/e2e-<backend>-<browser>/jaunder-diag-<backend>.log` to the
  `capture-<backend>.tar.gz` tarball (which contains `diag.log`), and the VM
  path from `/var/lib/jaunder/jaunder-diag.log` to
  `/var/lib/jaunder/capture/diag.log`.
- A new ADR records the capture-dir contract (realizing ADR-0049's flagged
  future work). It states explicitly that the diag/panic-hook trigger var
  changes from `JAUNDER_DIAG_LOG_FILE` to `JAUNDER_CAPTURE_DIR`.
- A decision record captures the **`host` crate-layering convention** — a
  strictly-host shared crate as sibling to target-agnostic `common`, with a
  symmetric strictly-client crate as a future peer. This is a structural
  convention broader than #227 (it governs where future host-only shared code
  lands), so it warrants its own ADR (or a clearly separable section of the
  capture-dir ADR). Decide granularity at authoring time via the `jaunder-adr`
  flow.
- **ADR-0049** is marked _superseded-in-part_ by the new ADR (a status note +
  forward cross-reference at its top), because its body asserts present-tense
  behavior that this change falsifies — notably "the panic hook is installed
  only when `JAUNDER_DIAG_LOG_FILE` is set" (now `JAUNDER_CAPTURE_DIR`). We
  annotate rather than rewrite the body (ADRs are point-in-time records; a
  reader must not be silently misled, but history stays intact). Follow the
  `jaunder-adr` skill's status-change flow.

## Acceptance criteria (observable)

1. **Single server var.**
   `rg 'JAUNDER_(MAIL|WEBSUB)_CAPTURE_FILE|JAUNDER_DIAG_LOG_FILE'` over the
   **live surfaces** — `server/`, `test-support/`, `xtask/`, `tools/`,
   `end2end/`, `flake.nix`, `docs/observability.md`, and `CONTRIBUTING.md` —
   returns no references. The server reads capture paths only via the single
   helper keyed on `JAUNDER_CAPTURE_DIR`. **Historical records are out of
   scope** and may retain mentions: everything under `docs/archive/`, and prior
   planning docs under `docs/superpowers/{specs,plans}/` that predate this issue
   (e.g. `2026-06-19-content-visibility-layer-c-design.md` references the old
   mail var as a record of a past design) — these document past state and are
   not migrated.
2. **Mail capture works via the dir.** `build_mailer(_, Some(<d>/mail.jsonl))`
   selects the file sender (test injects the path via an `rstest` fixture — no
   env); `None` → SMTP/noop. The `serve` root resolves the path from
   `JAUNDER_CAPTURE_DIR`.
3. **WebSub capture works via the dir.**
   `default_client(Some(<d>/websub.jsonl))` returns the file-capturing client;
   `None` → the live `HttpWebSubClient` (test injects the path, no env). The
   `serve` root resolves the path from `JAUNDER_CAPTURE_DIR`.
4. **Diag log works via the dir.** With `JAUNDER_CAPTURE_DIR=<d>` set, the
   scoped diag layer/panic hook writes `<d>/diag.log`; unset ⇒ the feature is
   inert (existing observability tests updated).
5. **Helper is the single source; no free-path args.** The three **filenames**
   (`mail.jsonl`/`websub.jsonl`/`diag.log`) appear in exactly one place in the
   whole codebase — the `host` helper — and nowhere in TS (the readers delegate
   via `capture-path`) or Nix. The dir-var **name** `JAUNDER_CAPTURE_DIR` is
   _read_ in one place (the helper) but is legitimately _named_ wherever the
   harness sets it (the flake, `xtask`'s `e2e_local.rs`, the `cli.rs` test) —
   that is inherent to configuring an env var, not the duplication this issue
   targets. A test exercises both branches of the helper (set ⇒ `Some(join)` and
   directory created; unset/blank ⇒ `None`). Neither `test-support reset-mail`
   nor `devtool seed-e2e` accepts a mail-file path argument (`--help`/`rg` shows
   the flags gone); `reset-mail` derives its target from `JAUNDER_CAPTURE_DIR`
   via the helper, and a test verifies it deletes `<dir>/mail.jsonl`. A test
   also asserts `reset-mail` **exits non-zero when `JAUNDER_CAPTURE_DIR` is
   unset** (loud misconfiguration, not a silent no-op).
6. **e2e green, both backends, both browsers.** `cargo xtask validate` passes —
   the full `{sqlite,postgres}×{chromium,firefox}` matrix runs against the
   capture-dir wiring (mail, websub, and diag/panic-gate specs all exercised).
7. **Whole-dir artifact lift.** On an e2e run, the capture directory is lifted
   per combo as a `capture-<backend>.tar.gz` tarball (of `diag.log` and any
   written `mail.jsonl`/ `websub.jsonl`); `nix.rs`'s
   `copy_e2e_diagnostics_between` unit test asserts a `capture-<backend>.tar.gz`
   file is copied and the old bare-`jaunder-diag.log` lock is replaced
   accordingly.
8. **Panic gate unchanged in behavior.** The e2e zero-panic gate still reads the
   scoped diag records (now at `capture/diag.log`) and the journal, and still
   fails on any unexpected `panicked at`.
9. **Host driver parity.** `cargo xtask e2e-local` (or a single-spec host run)
   exercises the capture-dir wiring end-to-end: server writes under the temp
   `capture/` dir, Playwright reads the same, seed resets
   `<capture>/mail.jsonl`.
10. **Production inert.** With `JAUNDER_CAPTURE_DIR` unset, mailer falls back to
    SMTP/noop, websub uses the live HTTP client, and no diag layer/panic hook is
    installed — identical to today.

## Out of scope

- Changing the _format_ of any captured stream (still JSONL / the diag log
  format).
- Any capture stream not already present (this is consolidation, not new
  capture).
- **The `otel-traces.jsonl` artifact.** It _is_ an exfiltrated server-side
  diagnostic, but it is written by the **otel-collector** service
  (`otelcol-contrib`) via its own YAML `file` exporter (`flake.nix:526`), not by
  the jaunder app via a `JAUNDER_*` env var — a different mechanism from the
  capture-to-file family this issue consolidates. Folding it in would
  additionally mean reworking the special copy-out layout
  (`otel-traces-<backend>.jsonl/otel-traces.jsonl`) that
  `cargo xtask traces run` consumes and ensuring `capture/` exists before the
  collector (which starts _before_ jaunder). This is a separable concern →
  **filed as a follow-up issue** (the plan's first task), not folded here.
- The app/system journals (`journalctl` output → journald, not app-written
  files) and the Playwright report/artifacts (`test-results/`, governed by the
  Playwright config per #153) — not app capture streams; lifted as today.

**Scope confirmed complete for the capture-env-var family:** an enumeration of
all `JAUNDER_*` vars and every production file-writer in
`server`/`storage`/`common` found the three consolidated here to be the _only_
"app writes a diagnostic stream to a file via env var." Other file-ish vars are
different in kind and deliberately untouched: `JAUNDER_RUNTIME_FILE` (a
port-discovery _handshake_, ADR-0035, consumed live, never copied out),
`JAUNDER_DB_PASSWORD_FILE` (an _input_ secret), and `JAUNDER_STORAGE_PATH`
(product data: DB, media, backups).

## Risks / notes

- **Loud→silent regression on the reset path (mitigated).** Today
  `devtool seed-e2e` takes `--mail-file` as a _required_ arg, so forgetting it
  is a hard clap error. Once `reset-mail` derives the path from
  `JAUNDER_CAPTURE_DIR`, a caller that forgets to set the dir could _silently
  skip_ the mail reset — stale captured mail from a prior run would leak into
  the next and cause confusing e2e flakes. **Mitigation:** `reset-mail` exits
  non-zero when `JAUNDER_CAPTURE_DIR` is unset/blank (AC5), preserving the loud
  failure. (Server read sites keep the opposite convention — unset ⇒ inert —
  because they are the production path; `reset-mail` is e2e-only.)
- **Test isolation (resolved by injection).** The consolidation could have made
  the mailer, websub, and observability tests all mutate the _same_
  `JAUNDER_CAPTURE_DIR`, cross-contaminating under a threaded `cargo test`. The
  injection design avoids it: mailer/websub tests receive a `TempDir` path as a
  value (no env), so **only** the observability tests set `JAUNDER_CAPTURE_DIR`,
  under that module's existing `lock_env()` — no new lock, and correct under
  both `nextest` and plain `cargo test`. The one env-read test lives in the
  `host` crate (its own binary; a local lock guards its two env cases).
- The whole-dir copy must be **guarded** (the dir may not exist if a run
  captured nothing); mirror the existing `test -e`/best-effort copy pattern.
- The server-side `create_dir_all` runs at startup (via the first helper call),
  so the capture dir exists for the harness even before the first byte is
  written; the copy-out guard is belt-and-suspenders.
- Keep the `issue-227` token in the branch, worktree, spec, and plan filenames
  (cycle state is derived from artifacts).
