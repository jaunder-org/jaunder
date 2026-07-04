# Spec — migrate `run-e2e-trace-analysis` into `xtask traces run` (#33)

**Issue:** [#33](https://github.com/jaunder-org/jaunder/issues/33) — _tooling:
migrate scripts/run-e2e-trace-analysis into devtool_ (milestone: Devtool
migration).

> As with #32, the issue title says "devtool," but **ADR-0028**'s litmus places
> this in **`xtask`**: `run-e2e-trace-analysis` is a **host** tool that _invokes
> `nix build`_ and analyzes the exfiltrated outputs — it can't run inside a Nix
> sandbox. ADR-0028's table already reads `#33 → xtask (confirm in its cycle)`;
> this cycle confirms it. No new ADR.

## Problem

`scripts/run-e2e-trace-analysis` is a ~170-line Node orchestrator: it
`nix build`s the `{sqlite,postgres}×{chromium,firefox}` e2e VM checks (or the
`-cold` package variants), collects each
`otel-traces-<backend>.jsonl/otel-traces.jsonl` artifact, and shells out to
`scripts/analyze-otel-traces` to report on them. It is the orchestration half of
the trace-perf tooling; #32 already migrated the analysis half to
`cargo xtask traces analyze` and left both scripts on disk for this cycle to
retire.

Two defects in the current script:

- **Out-path parser bug
  ([#224](https://github.com/jaunder-org/jaunder/issues/224)).** `runAndCapture`
  joins **stderr** into the text it parses for the `/nix/store` output path, so
  a stray `…-user-environment` line from nix's stderr can be picked as the
  result. The #155 measurement had to bypass the script because of this. (#224
  also covers an unrelated `hydrationHeavy*` rename — **out of scope here**; #33
  owns only the parser fix.)

## Goal

`cargo xtask traces run [--top N] [--trace ID] [--cold] [--browser chromium|firefox]`
reproduces the orchestrator in maintainable Rust: it builds the e2e checks,
finds their trace artifacts, and analyzes them by calling the **in-process**
`traces::analyze`/`render` seam #32 built — no subprocess to an analyzer script.
The out-path parser bug is fixed by construction (stdout-only parsing). Both
scripts are retired and docs point at the new command.

## Design

### 1. Command surface

A second subcommand under the existing `traces` group (added in #32):

```
cargo xtask traces run [--top N] [--trace TRACE_ID] [--cold] [--browser chromium|firefox]
```

- `--top N` (default 25, positive integer) — forwarded to `render`.
- `--trace TRACE_ID` — forwarded as the analysis trace filter.
- `--cold` — build the cold-cache **package** variants instead of the warm
  **check** variants.
- `--browser chromium|firefox` — restrict to one browser (default: both). Reuses
  the existing `E2eBrowser` clap `ValueEnum`; backends are always both
  (`sqlite`, `postgres`), matching the script.

`command_name()` → `"traces-run"`. Like `traces analyze`, it is a manual tool
(not part of `check`/`validate`) and **rejects `--json`** via
`Command::produces_json_payload()` (human report only, no structured payload).

### 2. Shared nix helper (`xtask/src/nix_build.rs`)

`audit_wasm` already contains the exact "run `nix build … --print-out-paths` →
store path" logic `traces run` needs. Extract it to a neutral module so one
tested implementation serves both call sites (the #224 bug _was_ a
store-path-parse bug; a single stdout-only parser is the guard against it
recurring):

```rust
// xtask/src/nix_build.rs
/// The realized store path from `nix build … --print-out-paths` output — the last
/// `/nix/store/` line of the given text (never `.drv`). Unchanged from
/// `audit_wasm::parse_store_path`.
pub fn parse_store_path(text: &str) -> Option<String>;

/// Select the store path from a completed `nix build`'s streams, parsing **stdout
/// only** — `stderr` is deliberately ignored so a `…-user-environment` (or any
/// other) line nix writes to stderr can never be selected (this is the #224 fix;
/// the Node bug was joining stderr into the parsed text). Pure over the two
/// strings, so the stdout-only contract is unit-testable without nix.
pub fn store_path_from_streams(stdout: &str, stderr: &str) -> Result<String>;

/// `nix build .#<attr> --no-link --print-out-paths`; captures both streams, bails
/// with the captured stderr on non-zero status, else `store_path_from_streams`.
/// Mirrors the previous `resolve_site_path` I/O exactly.
pub fn build_out_path(attr: &str) -> Result<String>;
```

`store_path_from_streams` is where the #224 fix lives and is verifiable: it
calls `parse_store_path(stdout)` and never inspects `stderr`. `build_out_path`
is the thin I/O boundary (run nix → hand its `Output` streams to
`store_path_from_streams`).

`audit_wasm::resolve_site_path(explicit)` becomes a thin wrapper — `explicit`
verbatim when set, else `build_out_path("site")` — so `audit-wasm`'s behavior is
**unchanged** (still captures stderr into its error). `parse_store_path`'s
existing `audit_wasm` unit tests move with it.

### 3. Orchestration (`xtask/src/traces/run.rs`)

`traces::run(top, trace, cold, browser)`:

- Enumerate `backends = [sqlite, postgres]` ×
  `browsers = browser.map(one).unwrap_or([chromium, firefox])`.
- For each pair, build the attr and realize it:
  - `ns = if cold { "packages" } else { "checks" }`,
    `suffix = if cold { "-cold" } else { "" }`.
  - `attr = format!("{ns}.x86_64-linux.e2e-{backend}-{browser}{suffix}")`.
  - `out = nix_build::build_out_path(&attr)?`.
  - `trace_file = <out>/otel-traces-<backend>.jsonl/otel-traces.jsonl`; error if
    it doesn't exist (naming the path).
- Collect the trace files, then
  `traces::analyze::analyze(&files, Filters { trace, project: None })?` and
  `traces::render::render(&analysis, top)`.

Pure, unit-testable helpers are split out from the nix/filesystem I/O:
`e2e_attr(backend, browser, cold) -> String`,
`trace_file_path(out, backend) -> PathBuf`, and the browser enumeration.

### 4. CLI wiring & output

- `TracesCommand::Run { top, trace, cold, browser }` added beside `Analyze`.
- `run()` dispatch: build `Filters`, call `traces::run`, put the rendered report
  in `result.traces` (printed by `print_human` before the verdict, as
  `traces analyze` does), push `StepResult::ok("traces-run")`.
- No live progress logging (the tool is used non-interactively;
  straightforwardness over interactive notice). nix's own output is captured,
  not streamed.

### 5. Error handling

A nix-build failure (`build_out_path` bails) or a missing trace file propagates
as `Err` from the dispatch arm → the exit-2 path in `main.rs` (consistent with
the `traces analyze` behavior). A bad `--top`/`--browser` is a clap parse error
(exit 2). Both are a documented, harmless delta from the Node script, whose
arg-parse failure exited **1** with a usage dump — the 1→2 shift matches the
rest of xtask.

### 6. Retirement & docs

#33 completes the retirement #32 deferred:

- **Delete** `scripts/analyze-otel-traces` **and**
  `scripts/run-e2e-trace-analysis`.
- Repoint the **instructional/usage** references off both scripts (the sites
  that tell a reader to invoke them). The full set found by grep, outside
  `docs/archive/`:
  - `CONTRIBUTING.md` — the "Run & Analyze" bullet → `cargo xtask traces run`.
  - `docs/observability.md` — the `run-e2e-trace-analysis` usage block
    (~:76-98): the two invocation examples, the `--cold`/`--browser`/`--trace`
    filter notes, and the trailing
    `(--project … is a flag of the underlying scripts/analyze-otel-traces …)`
    parenthetical (the script is gone) → the new command; **and** the
    present-tense harness mention at ~:305 ("driven … by the `#152`
    `run-e2e-trace-analysis` harness") → `cargo xtask traces run`.
  - `docs/ARCHITECTURE.md` (~:128) — the "specialized tools" list entry
    `scripts/analyze-otel-traces` → `cargo xtask traces analyze` (+
    `traces run`).
  - `flake.nix` comments at `:759` and `:1075` that name
    `run-e2e-trace-analysis` → `cargo xtask traces run` (comments only; no
    functional wiring references either script).
- **Not touched** (deliberately): frozen historical narratives that recount how
  a _past_ measurement was run (the `#155` findings sections in
  `docs/observability.md` citing the scripts by name), ADR-0028's classification
  table row (historical record), and the
  `//! Port of scripts/analyze-otel-traces` code-lineage comments #32 left in
  `xtask/src/traces/*.rs` (provenance, not instruction).

## Acceptance criteria

1. `cargo xtask traces run` builds the `{sqlite,postgres}×{chromium,firefox}`
   e2e checks (warm) or their `-cold` package variants (`--cold`), collects each
   `otel-traces-<backend>.jsonl/otel-traces.jsonl`, and prints the **same report
   `traces analyze` produces** (identical because it calls the same
   `traces::render` on the `analyze` result — no subprocess to a script).
   **Verification: manual** — one run against a warm build, output compared to
   `traces analyze` on the same artifacts.
2. `--browser chromium|firefox` restricts to one browser (both backends still
   built); `--top`/`--trace` are forwarded to the analysis. Verified by unit
   tests on the attr-construction and enumeration helpers.
3. The out-path selection reads **stdout only** — #224's stderr-join bug cannot
   recur. Regression-locked by a `store_path_from_streams` unit test:
   `store_path_from_streams(stdout_without_a_store_line, "/nix/store/…-user-environment")`
   returns `Err` (the stderr line is not selected), and with a real store line
   on stdout it returns that. (A `parse_store_path` test alone can't lock this —
   a `-user-environment` path _is_ a valid `/nix/store` path; the fix is which
   stream is parsed, which is exactly what `store_path_from_streams` isolates.)
4. The nix-build/parse-store-path logic is a **single shared** `nix_build`
   module used by both `audit-wasm` and `traces run`; `audit-wasm`'s behavior is
   unchanged (its `parse_store_path` tests pass in the new location).
5. `cargo xtask --json traces run` exits non-zero with the "no structured
   output" message; `produces_json_payload()` is `false` for `traces run`
   (regression-locked by a unit test).
6. A nix-build failure or a missing trace file exits non-zero (Err → exit 2),
   naming the failing attr / missing path. **Verification:**
   `trace_file_path(out, backend)` is a pure helper with a unit test (the
   `<out>/otel-traces-<backend>.jsonl/otel-traces.jsonl` shape); the runtime
   "file absent → Err naming the path" and nix-failure branches are I/O-bound
   and covered by the manual run (AC1) — not unit-tested.
7. `scripts/analyze-otel-traces` and `scripts/run-e2e-trace-analysis` are
   deleted; every **instructional/usage** reference is repointed to
   `cargo xtask traces run` / `cargo xtask traces analyze` — the §6 set:
   `CONTRIBUTING.md`, `docs/observability.md` (usage block + the ~:305 harness
   mention), `docs/ARCHITECTURE.md` (~:128 tools list), and the two `flake.nix`
   comments. Explicitly **excluded** (not violations): the `#155` historical
   findings narratives, ADR-0028's table, and the `//! Port of …` lineage
   comments in `xtask/src/traces/*.rs`.
8. #224 is updated to note its parser half is handled by #33, leaving it the
   `hydrationHeavy*` rename.
9. `cargo xtask validate --no-e2e` is green (static + clippy + host xtask unit
   suite
   - coverage), including the new `traces run` and relocated `nix_build` tests.
