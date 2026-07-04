# Spec — migrate `analyze-otel-traces` into `xtask traces analyze` (#32)

**Issue:** [#32](https://github.com/jaunder-org/jaunder/issues/32) — _tooling:
migrate scripts/analyze-otel-traces into devtool_ (milestone: Devtool
migration).

> The issue title says "into devtool," but **ADR-0028** (the devtool/xtask
> boundary) supersedes that wording by its litmus — _where must this code run?_
> `analyze-otel-traces` is host-side analysis of already-exfiltrated trace
> artifacts (it never runs inside a Nix sandbox), so its home is **`xtask`**,
> not `devtool`. ADR-0028's classification table already records
> `#32 → xtask (confirm in its cycle)`; this cycle confirms it. No new ADR is
> warranted — 0028 governs.

## Problem

`scripts/analyze-otel-traces` is a ~1150-line Node script that parses the
OpenTelemetry JSONL exported by the e2e VM collector and prints twelve report
tables (slowest spans, slowest `e2e.test` spans, action/navigation/
long-task/resource hotspots, cache-warmth, hydration-vs-API, navigation-phase
and hydration-runtime components, per-project durations, per-trace totals). It
is the analysis half of the trace-perf tooling used by the #152/#155 hydration/
wall-clock investigations. Per the coverage-pipeline Rust migration
(`docs/archive/2026-06-24-coverage-pipeline-rust-migration-design.md`), the
remaining `scripts/*` helpers are being replaced with maintainable, testable
Rust in the workspace tooling crates. This issue does the analyzer.

`scripts/run-e2e-trace-analysis` (issue #33) is the orchestration half: it
`nix build`s the four `{sqlite,postgres}×{chromium,firefox}` e2e checks,
collects their trace artifacts, and shells out to `analyze-otel-traces`. #33
will become `xtask traces run` and will call the analyzer **in-process**.

## Goal

`cargo xtask traces analyze <otel-traces.jsonl>...` reproduces the Node script's
analysis faithfully — same twelve sections, same computed statistics — as
maintainable, unit-tested Rust, rendered with a table library rather than
hand-rolled column padding. The analysis core is exposed as a reusable in-crate
API so #33's `traces run` layers nix-orchestration on top with no subprocess and
no duplicated logic.

## Design

### 1. Placement and command surface

A new **`traces`** subcommand group in `xtask` (mirroring the existing `adr` and
`coverage` groups), with one subcommand this cycle:

```
cargo xtask traces analyze [--top N] [--trace TRACE_ID] [--project NAME] <file>...
```

- `--top N` (default 25, positive integer — clap value parser) — rows per table
  for the ranked tables. **Three outputs deliberately ignore `--top` and print
  every row**, matching Node: the cache-warmth table
  (`printE2eNavigationCacheWarmth`), the per-project E2E-duration table
  (`printE2eByProject`), and the long-task **by-project** sub-table
  (`projectRows`). Preserve that.
- `--trace TRACE_ID` — restrict to one trace id.
- `--project NAME` — restrict to one e2e project; **filters only spans whose
  `name` starts with `e2e.`** (HTTP/server spans always pass), exactly as Node.
  When set and spans remain, a **`Project filter: <name>` header line + blank
  line** is printed before the tables (`scripts/analyze-otel-traces:1134-1137`).
- One or more positional trace files.

`command_name()` returns `"traces-analyze"`. #33 adds `traces run` to this
group.

### 2. Reusable in-crate seam (the payoff of the #32/#33 consolidation)

The analyzer is **not** buried in the CLI handler. `xtask/src/traces/` exposes:

- `analyze(inputs: &[PathBuf], filters: Filters) -> Result<Analysis>` — read +
  parse the JSONL, compute every section. `Analysis` holds the typed per-section
  row models (counts, `f64` ms, ids, names) — the values the unit tests assert
  on.
- `render(analysis: &Analysis, top: usize) -> String` — the `tabled` rendering
  of those models into the report tables.

`traces analyze` (this cycle) = parse args → `analyze` → `render` → print.
Because `traces analyze` rejects `--json` (§4), `run()` prints `render()`
straight to stdout and pushes an ok `StepResult` — it does not route the tables
through the `CommandResult` JSON envelope the way `audit_wasm` serializes
`result.audit`. `traces run` (#33) = `nix build` → collect files → `analyze`
**in-process** → `render` → print. This mirrors how the `coverage` module
exposes `reanchor`/`refresh_crap` for `run()` to call. Module shape mirrors
`xtask/src/audit_wasm.rs`: pure parse/compute helpers split out and unit-tested;
filesystem I/O confined to the entry points.

### 3. Output: `tabled`, not byte-identical

Rendering uses the **`tabled`** crate (new `xtask` dependency; host-only,
cache-excluded, so the build cost is cheap). Output is **not** required to be
byte-identical to the Node script — it should be clean, aligned tables carrying
the same columns and rows. The typed row models are the test surface; the
`tabled` render is a thin display layer (numeric columns formatted to the Node
script's decimal precision, e.g. ms to three decimals). Because the port is
faithful, a one-time Rust-vs-Node diff on the committed fixture validates
equivalence before the script is retired (in #33).

### 4. `--json` rejection policy

`traces analyze` produces human-facing tables, not a structured payload. Rather
than emit a hollow `--json` envelope, `xtask --json traces analyze …`
**errors**. This is encoded as a minimal general policy, not a one-off:
`Command` gains `produces_json_payload() -> bool` (defaults `true`;
`traces analyze` returns `false`), and `run()` rejects `--json` for any command
that answers `false`. Only `traces analyze` opts out today; every future command
answers the question explicitly.

### 5. Sections: faithful port of all twelve

All twelve report functions are ported with their exact statistics and
skip-when-empty behavior:

1. Top-N slowest spans (all spans) — always printed when spans exist.
2. Top-N slowest `e2e.test` spans.
3. `e2e` action hotspots (`e2e.action_top_json`).
4. `e2e` navigation hotspots: phase totals + slow navigation targets
   (`e2e.navigation_top_json`).
5. Navigation commit→hydration by cache warmth.
6. Long-task hotspots + per-project totals (`e2e.long_tasks_json`).
7. Resource hotspots: initiator + asset (`e2e.resource_summary_json`).
8. Hydration budget vs API budget (`e2e.navigation_top_json` +
   `e2e.request_top_slow_json`).
9. Navigation phase component hotspots: samples + targets + by-project
   (`e2e.navigation_top_json`).
10. Hydration runtime components: samples + by-project
    (`e2e.hydration_runtime_json`).
11. E2E test duration by project — always printed when `e2e.test` spans exist.
12. Per-trace duration totals — always printed when spans exist.

Sections 3–10 are omitted when their driving attribute is absent/empty, exactly
as Node's early-returns do.

**Separable concern (deferred, not dropped here):** several hydration-focused
sections (candidates: #5, #8, #9, #10, and the hydration span attributes they
read) are likely OBE after the CSR re-architecture. Auditing and removing the
obsolete set needs CSR context and is **filed as a follow-on issue by the plan's
first task** — this cycle ports them faithfully so nothing is silently lost.

### 6. Parsing and error/edge semantics (faithful to Node)

- Record nesting is `resourceSpans[].scopeSpans[].spans[]`
  (`scripts/analyze-otel-traces:119-130`); parsed with `serde_json` (already an
  `xtask` dep). Span durations from `(endTimeUnixNano − startTimeUnixNano)` as
  nanos → ms (`u64`/`i128`, matching Node's `BigInt`).
  `busy_ns`/`idle_ns`/`method`/`uri`/`e2e.*` read from the `attributes[]` list
  (`stringValue`, else `intValue`).
- **URL normalization (`toUrlPath`, `:306`).** Navigation targets, resource
  asset names, and navigation-phase-component targets are normalized: a
  parseable URL → `host + pathname`; an unparseable value → the raw string;
  empty/non-string → `""`. Preserve this shaping.
- **Two different malformed-JSON policies** — do not conflate them:
  - A malformed **top-level JSONL line** → **hard error** naming the file
    (`:113-117`).
  - A malformed **embedded `e2e.*_json` attribute** → **silently falls back** to
    the empty default (`parseJsonAttr`, `:85-95`); it is not an error. The port
    must not hard-error here.
- **Numeric guards.** Per-section negative/non-finite drops are preserved (e.g.
  `value < 0` in phase/cache-warmth/long-task/resource accumulation, `value > 0`
  in hydration-vs-API). Sort keys differ per table (`maxMs`, `avgMs`, raw
  `value`, or `hydrationMs` as the script uses).
- **No input files / file-not-found / bad `--top`** → error via xtask's existing
  `Err`→**exit 2** path (clap parse errors also exit 2). The Node script used
  exit 1; this 1→2 shift is a documented, harmless delta — the only consumer
  (`run-e2e-trace-analysis`, itself soon retired) checks `status !== 0`, and
  #33's `traces run` calls the analyzer in-process (no exit code inspected at
  all).
- **No spans found** → print `No spans found in the provided trace files.` and
  exit **0**.
- Missing attributes → empty/empty-fallback; sections skip when empty (§5).

### 7. Retirement boundary (#32 additive; #33 retires both scripts)

#32 is **additive**: it adds `traces analyze`, repoints the
`analyze-otel-traces` usage in `CONTRIBUTING.md` and `docs/observability.md` to
the new command, and **leaves both scripts on disk** so `run-e2e-trace-analysis`
keeps working untouched. #33 deletes **both** `scripts/analyze-otel-traces` and
`scripts/run-e2e-trace-analysis` atomically when it lands `traces run`. The
transient state (new command + old scripts still present) is harmless for the
short window until #33.

### 8. Testing

- **One committed synthetic fixture** —
  `xtask/src/traces/testdata/otel-traces-sample.jsonl` (pulled in via
  `include_str!` from the in-module tests), hand-crafted to populate all twelve
  sections: a couple of traces, several `e2e.test` spans each carrying the
  `e2e.*_json` attributes, and HTTP spans with
  `method`/`uri`/`busy_ns`/`idle_ns`. Tests assert the computed per-section
  statistics (sorted order, counts, avg/max/total). The fixture must also
  exercise the shaping behaviors the review flagged, or the Rust-vs-Node diff
  won't cover them: at least one **negative/non-finite** metric (to hit the
  guard drops), navigation/resource entries with **URLs** (to hit `toUrlPath`),
  a malformed **embedded** `e2e.*_json` attribute (silent fallback), and a run
  with **`--project`** set (to hit the `Project filter:` header and the
  `e2e.`-only filter). This fixture is also the input for the one-time
  Rust-vs-Node equivalence diff.
- **Inline edge-case unit tests** — tiny in-code spans for: empty file,
  malformed line (hard error), missing attribute (empty fallback), `--trace`
  filter, `--project` filter (HTTP spans still pass), no-spans message.
- **CLI parse tests** — mirror the existing `cli_tests`: `traces analyze` parses
  positionals + `--top`/`--trace`/`--project`; `produces_json_payload()` is
  `false` for `traces analyze` and `--json` is rejected.
- New `xtask` code must satisfy the coverage gate; the fixture exercises every
  `analyze` path and a `render` smoke test covers the display layer.

## Acceptance criteria

1. `cargo xtask traces analyze <fixture>` prints the twelve sections (those
   whose driving data is present) with the same statistics the Node script
   computes for the same input. The **durable** guard is unit tests asserting on
   the returned `Analysis` (per-section sorted order, counts, avg/max/total)
   over the fixture; the Rust-vs-Node output comparison is a one-time dev-time
   cross-check (the script is deleted in #33, so it cannot be an enduring
   regression test).
2. `--top N`, `--trace TRACE_ID`, and `--project NAME` behave as the Node script
   does, verified by unit tests: `--top` bounds the ranked tables but the
   cache-warmth, per-project-duration, and long-task-by-project tables print all
   rows regardless; `--project` filters only `e2e.*`-named spans and emits the
   `Project filter: <name>` header; `--trace` restricts to one trace id.
3. Output is rendered via `tabled` (no hand-rolled
   `padLeft`/`padRight`/`truncate` column code in the port).
4. The analysis core is a reusable in-crate API —
   `analyze(...) -> Result<Analysis>` and `render(&Analysis, top)` — callable
   without the CLI, so #33 can invoke it in-process. (Observable: a unit test
   calls `analyze` on the fixture and asserts on the returned `Analysis` without
   spawning a process.)
5. `xtask --json traces analyze <file>` exits non-zero with a message stating
   `--json` is unsupported for a command with no structured output;
   `Command::produces_json_payload()` returns `false` for `traces analyze` and
   `true` for at least one other command (regression-locked by a unit test).
6. A malformed JSON line causes a hard error naming the offending file; a
   file-not-found causes an error; both exit non-zero. An input with zero
   matching spans prints the no-spans message and exits 0.
7. `CONTRIBUTING.md` and `docs/observability.md` reference
   `cargo xtask traces analyze` in place of `scripts/analyze-otel-traces`. Both
   scripts remain on disk (retired in #33).
8. The plan's first task files a follow-on issue to audit/remove the CSR-OBE
   hydration-focused sections.
9. `cargo xtask check` is green (static + clippy + host xtask unit suite +
   coverage), including the new `traces` tests.
