# `xtask traces run` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate `scripts/run-e2e-trace-analysis` into `cargo xtask traces run`
— the nix-orchestration that builds the e2e checks and analyzes their traces via
the in-process `traces::analyze`/`render` seam #32 built — and retire both trace
scripts.

**Architecture:** A shared `xtask/src/nix_build.rs` (extracted from
`audit_wasm`) does `nix build … --print-out-paths` → store path (stdout-only,
fixing #224). A new `xtask/src/traces/run.rs` orchestrates the
`{sqlite,postgres}×{chromium,firefox}` builds and hands the collected trace
files to `traces::analyze`/`render`. Thin CLI arm.

**Tech Stack:** Rust, `clap`, `anyhow`, the existing `traces` module.

**Spec:** `docs/superpowers/specs/2026-07-04-issue-33-traces-run-xtask.md`. Node
source of truth: `scripts/run-e2e-trace-analysis` (cited by line).

## Global Constraints

- **Home = `xtask`** (ADR-0028). No new ADR.
- **#224 parser fix by construction:** the store path is selected from **stdout
  only** (`store_path_from_streams`); stderr is never parsed.
- **Single shared** `nix_build` module for both `audit-wasm` and `traces run`;
  `audit-wasm` behavior unchanged.
- **`--json` rejected** for `traces run` (`produces_json_payload()` false);
  errors (nix-build failure / missing trace file) propagate as `Err` → exit 2.
- **Command-first / dead-code gate:** xtask's `-D dead-code` rejects an
  unconsumed `pub` item in a private module. `nix_build` stays reachable via
  `audit-wasm` throughout Task 1; `traces::run`'s pure helpers land in the same
  commit as the command that calls them (Task 2). No commit leaves a `pub` fn
  unconsumed by production code.
- xtask tests: `cargo test --manifest-path xtask/Cargo.toml`. xtask is excluded
  from the Nix coverage check (host suite + clippy only). Commit gate:
  `cargo xtask check` before each commit (jaunder-commit). **No `Co-Authored-By`
  trailer.**

---

## Review header

**Scope (in):** `traces run` (nix orchestration + in-process analysis), the
shared `nix_build` extraction (with the #224 stdout-only fix), retiring both
scripts, doc repointing, the #224 coordination comment.

**Scope (out):** #224's `hydrationHeavy*` rename; any change to the
`traces analyze` analysis logic (#32, merged, reused as-is).

**Tasks:**

1. Extract `nix_build.rs` from `audit_wasm` (+ `store_path_from_streams`, the
   #224 guard).
2. `traces run` — CLI arm + `run.rs` helpers/orchestration + tests + manual e2e
   verify.
3. Retire both scripts, repoint instructional docs, comment on #224.

**Key risks/decisions:**

- The #224 fix is a _stream-selection_ fix, not a parser fix — isolated in the
  pure `store_path_from_streams(stdout, stderr)` so it's unit-testable without
  nix.
- `traces run`'s nix I/O is manual/integration; only the pure helpers
  (`e2e_attr`, `trace_file_path`, browser enumeration) + CLI parse are
  unit-tested (mirrors how `audit_wasm`'s nix path is left to manual use).
- Retire (Task 3) lands _after_ the command works (Task 2), so no commit breaks
  tooling.

---

### Task 1: Extract `nix_build.rs` (shared) + the #224 stdout-only guard

**Files:**

- Create: `xtask/src/nix_build.rs`
- Modify: `xtask/src/audit_wasm.rs` — remove `parse_store_path` (+ its 2 tests,
  moved); `resolve_site_path` becomes a wrapper over
  `nix_build::build_out_path`.
- Modify: `xtask/src/lib.rs` — add `mod nix_build;`

**Interfaces (produced):**

```rust
// xtask/src/nix_build.rs
use anyhow::{bail, Context, Result};
use std::process::Command;

/// Last `/nix/store/` line of `text` (never `.drv`). Verbatim move of
/// `audit_wasm::parse_store_path`.
pub fn parse_store_path(text: &str) -> Option<String>;

/// Select the store path from a completed build's streams — parses `stdout` ONLY;
/// `stderr` is used solely for the error message, never parsed. This is the #224
/// fix (the Node bug joined stderr into the parsed text).
pub fn store_path_from_streams(stdout: &str, stderr: &str) -> Result<String> {
    parse_store_path(stdout).with_context(|| {
        format!("could not parse a /nix/store path from nix stdout; stderr:\n{stderr}")
    })
}

/// `nix build .#<attr> --no-link --print-out-paths`; captures both streams, bails
/// with stderr on non-zero status, else `store_path_from_streams`.
pub fn build_out_path(attr: &str) -> Result<String>;
```

`audit_wasm::resolve_site_path(explicit)` = `explicit` verbatim when set, else
`nix_build::build_out_path("site")`.

- [x] **Step 1: Write the failing tests** (`#[cfg(test)]` in `nix_build.rs`)

  ```
  // moved verbatim from audit_wasm:
  test parse_store_path_takes_last_store_line:
      parse_store_path("warning: x\n/nix/store/aaa-x\n  /nix/store/bbb-site  \n")
        == Some("/nix/store/bbb-site")
  test parse_store_path_none_when_no_store_line:
      parse_store_path("no paths here\n") == None
  // new — the #224 guard:
  test store_path_from_streams_ignores_stderr:
      // stdout has NO store line; stderr carries a -user-environment store path.
      store_path_from_streams("no result here\n",
                              "/nix/store/zzz-user-environment\n").is_err()
  test store_path_from_streams_takes_stdout:
      store_path_from_streams("/nix/store/aaa-e2e\n", "junk").unwrap() == "/nix/store/aaa-e2e"
  ```

- [x] **Step 2: Run, verify fail.** Run:
      `cargo test --manifest-path xtask/Cargo.toml nix_build` Expected: FAIL
      (module absent).

- [x] **Step 3: Implement** `nix_build.rs`: move `parse_store_path` (+ body)
      from `audit_wasm`; add `store_path_from_streams` and `build_out_path` (the
      latter is `audit_wasm::resolve_site_path`'s non-explicit body generalized
      to `attr`). Add `mod nix_build;` to `lib.rs`. Rewrite
      `audit_wasm::resolve_site_path` as the wrapper and drop its now-moved
      `parse_store_path` + the two `parse_store_path_*` tests. **Prune the
      now-orphaned imports** in `audit_wasm.rs`: `use std::process::Command;`
      and `bail` (from `use anyhow::{bail, Context, Result};`) were used only
      inside the old `resolve_site_path` body and are now dead under
      `-D warnings` — remove them (keep `Context`/`Result`, still used by
      `run`). Both `build_out_path` and `parse_store_path` move to `nix_build`,
      which imports `Command`/`bail`/`Context`.

- [x] **Step 4: Run, verify pass.** Run:
      `cargo test --manifest-path xtask/Cargo.toml` Expected: PASS (audit_wasm
      tests still green in their new shape).

- [x] **Step 5: Commit**
  ```bash
  git add xtask/src/nix_build.rs xtask/src/audit_wasm.rs xtask/src/lib.rs
  git commit -m "refactor(xtask): extract nix_build helper; stdout-only store-path selection (#33)"
  ```
  Run `cargo xtask check` first (jaunder-commit).

---

### Task 2: `traces run` — CLI arm, orchestration, helpers, tests

**Files:**

- Create: `xtask/src/traces/run.rs` (+ `pub mod run;` in
  `xtask/src/traces/mod.rs`)
- Modify: `xtask/src/lib.rs` — `TracesCommand::Run`, `command_name`,
  `produces_json_payload`, `run()` dispatch arm, `cli_tests`

**Interfaces:**

- Consumes: `nix_build::build_out_path`, `traces::analyze::analyze`,
  `traces::render::render`, `traces::parse::Filters`, `E2eBackend`/`E2eBrowser`.
- Produces:

  ```rust
  // xtask/src/traces/run.rs
  use std::path::PathBuf;
  use anyhow::{Context, Result};
  use crate::nix_build::build_out_path;
  use crate::{E2eBackend, E2eBrowser};

  /// The flake attr for one e2e combo. cold → `packages…-cold`, warm → `checks…`.
  /// e.g. `checks.x86_64-linux.e2e-sqlite-chromium`,
  ///      `packages.x86_64-linux.e2e-postgres-firefox-cold`.
  pub fn e2e_attr(backend: E2eBackend, browser: E2eBrowser, cold: bool) -> String;

  /// `<out>/otel-traces-<backend>.jsonl/otel-traces.jsonl`.
  pub fn trace_file_path(out: &str, backend: E2eBackend) -> PathBuf;

  /// One browser if `--browser` given, else both (chromium, firefox).
  pub fn browsers(browser: Option<E2eBrowser>) -> Vec<E2eBrowser>;

  /// Build every combo (both backends × the selected browsers), collect each
  /// trace file, erroring (naming the path) if one is absent. nix I/O — manual.
  pub fn collect_trace_files(cold: bool, browser: Option<E2eBrowser>) -> Result<Vec<PathBuf>>;
  ```

  `collect_trace_files`: for `backend in [Sqlite, Postgres]`,
  `br in browsers(browser)`:
  `let out = build_out_path(&e2e_attr(backend, br, cold))?;`
  `let f = trace_file_path(&out, backend);`
  `ensure!(f.exists(), "trace file not found: {}", f.display());` push `f`.
  (Backends outer, browsers inner — matches the script's iteration/tie-order.)

  ```rust
  // xtask/src/lib.rs — TracesCommand gains:
  Run {
      #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
      top: u64,
      #[arg(long)] trace: Option<String>,
      #[arg(long)] cold: bool,
      #[arg(long, value_enum)] browser: Option<E2eBrowser>,
  }
  ```

  `command_name`: `Traces(TracesCommand::Run { .. }) => "traces-run"`.
  `produces_json_payload`: `false` for `Traces(Analyze | Run)` (widen the
  existing `matches!`; update its doc comment). `run()` dispatch arm:

  ```rust
  Command::Traces(TracesCommand::Run { top, trace, cold, browser }) => {
      let start = std::time::Instant::now();
      let mut result = CommandResult::new("traces-run");
      let files = traces::run::collect_trace_files(cold, browser)?;      // Err → exit 2
      let filters = traces::parse::Filters { trace, project: None };
      let analysis = traces::analyze::analyze(&files, filters)?;
      result.traces = Some(traces::render::render(&analysis, top as usize));
      result.push(StepResult::ok("traces-run").detail(format!("{} trace file(s)", files.len())));
      finalize(&mut result, start);
      Ok(result)
  }
  ```

- [x] **Step 1: Write the failing tests**

  ```
  // run.rs #[cfg(test)]
  test e2e_attr_warm_and_cold:
      e2e_attr(Sqlite, Chromium, false) == "checks.x86_64-linux.e2e-sqlite-chromium"
      e2e_attr(Postgres, Firefox, true) == "packages.x86_64-linux.e2e-postgres-firefox-cold"
  test trace_file_path_shape:
      trace_file_path("/nix/store/x", Sqlite)
        == PathBuf::from("/nix/store/x/otel-traces-sqlite.jsonl/otel-traces.jsonl")
  test browsers_one_or_both:
      browsers(Some(Firefox)) == [Firefox]
      browsers(None) == [Chromium, Firefox]

  // lib.rs cli_tests
  test traces_run_parses_flags:
      parse ["xtask","traces","run","--top","40","--cold","--browser","firefox","--trace","aa"]
      → top==40, cold==true, browser==Some(Firefox), trace==Some("aa"); command_name()=="traces-run"
  test traces_run_defaults:
      parse ["xtask","traces","run"] → top==25, cold==false, browser==None, trace==None
  // UPDATE the existing `produces_json_payload_false_only_for_traces_analyze`
  // (lib.rs) — widen it to also assert Run is false and rename to
  // `produces_json_payload_false_for_traces_commands` (it would otherwise lie):
  test produces_json_payload_false_for_traces_commands:
      Traces(Analyze{..}).produces_json_payload() == false
      Traces(Run{..}).produces_json_payload() == false
      Check{..}.produces_json_payload() == true; AuditWasm{..}.produces_json_payload() == true
  test run_rejects_json_for_traces_run:
      run(Cli{json:true, command:Traces(Run{..})}).is_err()  // and message mentions --json
  ```

  (`E2eBackend`/`E2eBrowser` may need `PartialEq` derive for `==` in tests —
  they already derive it per lib.rs. `browsers` returns `Vec<E2eBrowser>`.)

- [x] **Step 2: Run, verify fail.** Run:
      `cargo test --manifest-path xtask/Cargo.toml` Expected: FAIL.

- [x] **Step 3: Implement** `run.rs` (helpers + `collect_trace_files`) and the
      `lib.rs` wiring (variant, `command_name`, widened
      `produces_json_payload` + doc, dispatch arm, `pub mod run;`).
      `E2eBackend`/`E2eBrowser` `as_str` is crate-private and reachable from
      `traces::run` (same crate).

- [x] **Step 4: Run, verify pass.** Run:
      `cargo test --manifest-path xtask/Cargo.toml` Expected: PASS.

- [x] **Step 5: Commit**

  ```bash
  git add xtask/src/traces/run.rs xtask/src/traces/mod.rs xtask/src/lib.rs
  git commit -m "feat(xtask): traces run — nix-orchestrate e2e checks + in-process analysis (#33)"
  ```

- [ ] **Step 6: Manual end-to-end verification** (AC1 — nix I/O, not
      unit-tested)

  Against a warm build (limit to one browser to halve build time):

  ```bash
  cargo run --manifest-path xtask/Cargo.toml -- traces run --browser chromium --top 10
  ```

  Expected: it builds `e2e-sqlite-chromium` + `e2e-postgres-chromium`, finds
  both trace files, and prints the report. Cross-check a couple of section rows
  against `cargo xtask traces analyze <the two otel-traces.jsonl paths>` — must
  be identical. If the e2e VM build is prohibitively cold, note it and verify on
  the ship gate's build instead. (No commit — verification only.)

---

### Task 3: Retire both scripts, repoint instructional docs, coordinate #224

**Files:**

- Delete: `scripts/analyze-otel-traces`, `scripts/run-e2e-trace-analysis`
- Modify: `CONTRIBUTING.md`, `docs/observability.md`, `docs/ARCHITECTURE.md`,
  `flake.nix`

**Interfaces:** none.

- [ ] **Step 1: Delete the scripts**

  ```bash
  git rm scripts/analyze-otel-traces scripts/run-e2e-trace-analysis
  ```

- [ ] **Step 2: Repoint the instructional references** (spec §6 set)
  - `CONTRIBUTING.md` "Run & Analyze" bullet → `cargo xtask traces run` (with
    `--cold`/`--browser` notes preserved).
  - `docs/observability.md`: the usage block (~:73-98) — the two invocation
    examples and the filter notes → `cargo xtask traces run`; drop/reword the
    trailing `(--project … underlying scripts/analyze-otel-traces …)`
    parenthetical (the script is gone); and the ~:305 harness mention →
    `cargo xtask traces run`.
  - `docs/ARCHITECTURE.md` (~:128) tools-list entry
    `scripts/analyze-otel-traces` → `cargo xtask traces analyze` (add a
    `cargo xtask traces run` sibling entry).
  - `flake.nix` comments at ~:759 and ~:1075 → `cargo xtask traces run`. Leave
    untouched: the `#155` findings narratives, ADR-0028's table, and the
    `//! Port of …` comments in `xtask/src/traces/*.rs`.

- [ ] **Step 3: Verify no stray instructional references** Run:
      `rg -n 'scripts/(analyze-otel-traces|run-e2e-trace-analysis)'` Expected:
      only `docs/archive/**`, `docs/adr/0028-*`, the `#155` findings prose in
      `docs/observability.md`, and the `traces/*.rs` lineage comments remain —
      nothing instructional.

- [ ] **Step 4: Commit**

  ```bash
  git add scripts/ CONTRIBUTING.md docs/observability.md docs/ARCHITECTURE.md flake.nix
  git commit -m "test-infra(e2e): retire trace scripts for cargo xtask traces run/analyze (#33)"
  ```

- [ ] **Step 5: Coordinate #224** (tracker — no commit) Comment on
      [#224](https://github.com/jaunder-org/jaunder/issues/224): the
      `run-e2e-trace-analysis` out-path parser fix is handled by #33 (the script
      is retired and its Rust replacement `cargo xtask traces run` parses stdout
      only via `nix_build::store_path_from_streams`), leaving #224 the
      `hydrationHeavy*` rename.
