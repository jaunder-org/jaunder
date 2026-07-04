# `xtask traces analyze` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate `scripts/analyze-otel-traces` into a Rust
`cargo xtask traces analyze <files…>` subcommand — a faithful, unit-tested port
of the twelve OTel trace report sections.

**Architecture:** A new `traces` module in `xtask` with a reusable in-crate seam
— `parse` (JSONL → `Vec<Span>`), `analyze` (`Vec<Span>` → typed `Analysis`), and
`render` (`Analysis` → `tabled` tables). The CLI handler is thin; #33's
`traces run` will call `analyze`/`render` in-process. Mirrors
`xtask/src/audit_wasm.rs`.

**Tech Stack:** Rust, `clap` (CLI), `serde_json` (already a dep; JSONL parse),
`tabled` (new dep; table rendering), `anyhow`.

**Spec:** `docs/superpowers/specs/2026-07-04-issue-32-traces-analyze-xtask.md` —
the plan is "how"; consult the spec for "what/why" and the exact per-section
semantics. Node source of truth: `scripts/analyze-otel-traces` (cited by line).

## Global Constraints

- **Home = `xtask`** (ADR-0028), not `devtool`. No new ADR.
- **Faithful port**: same twelve sections, same statistics, same skip-when-empty
  behavior. Output need **not** be byte-identical (rendered via `tabled`).
- **No hand-rolled column code** (`padLeft`/`padRight`/`truncate`) — use
  `tabled`.
- **`--top` bounds only the ranked tables**; the cache-warmth,
  per-project-duration, and long-task-by-project tables print **all** rows.
- **Two malformed-JSON policies**: malformed top-level JSONL line → hard error;
  malformed embedded `e2e.*_json` attribute → silent fallback.
- **Exit codes** flow through xtask's `Err`→exit-2 path (1→2 delta vs Node;
  noted, harmless).
- **Dead-code gate → command-first commits.** xtask's `-D dead-code` gate
  rejects a `pub` item in the private `traces` module until a production path
  (the `traces analyze` command) reaches it. Every commit lands each new `pub`
  fn together with its production caller — never an unconsumed helper.
- xtask tests run via `cargo test --manifest-path xtask/Cargo.toml`. xtask is
  excluded from the Nix coverage check — host unit suite + clippy only.
- Commit gate: run `cargo xtask check` before each commit (jaunder-commit). **No
  `Co-Authored-By` trailer.**

---

## Review header

**Scope (in):** the `traces analyze` analyzer (parse + analyze + render + CLI),
the `--json`-rejection policy, doc repointing, one committed fixture.

**Scope (out):** #33's `traces run` orchestration; deleting either script (both
stay on disk until #33); pruning OBE hydration sections (Task 1 files that as a
follow-on).

**Tasks:**

1. File the follow-on issue (audit/remove CSR-OBE hydration sections).
2. Command-first vertical slice — parse + analyze(slowest-spans) + render +
   CLI + `--json` policy + fixture. Runnable, gate-clean.
3. The three remaining always/simple sections.
4. JSON-attribute hotspot sections (+ `to_url_path`/`parse_json_attr` helpers).
5. Cache-warmth, hydration-vs-API, nav-phase-components, hydration-runtime.
6. Docs repoint + one-time Node-vs-Rust equivalence diff.

**Key risks/decisions:**

- **Command-first** for the dead-code gate (Global Constraints): the CLI is
  wired in Task 2 so every later helper is reachable in the commit that adds it.
- `analyze` returns **fully-sorted** row vectors; `render` applies `top` slicing
  (and skips the three all-row tables). `top` is a pure display concern.
- `Span` keeps `raw: serde_json::Value` so `analyze` reads `e2e.*_json` on
  demand (only e2e.test spans carry them), as Node keeps `raw`.
- Parse splits file-read from string-parse (`parse_spans(content, …)`) so the
  fixture drives tests via `include_str!` with no temp file and `analyze_spans`
  runs without a process (crit. 4).

---

### Task 1: File the follow-on issue (separable concern)

**Files:** none (tracker only).

**Interfaces:** none.

- [x] **Step 1: File the issue** via jaunder-issues, in `jaunder-org/jaunder`.

  Title: `tooling: remove CSR-OBE hydration sections from xtask traces analyze`

  Body (substance):

  > Follow-on from #32 (`xtask traces analyze`). #32 ported all twelve trace
  > report sections faithfully. Several **hydration-focused** sections are
  > likely OBE after the CSR re-architecture and should be audited and removed,
  > along with any now-unused hydration span attributes:
  >
  > - `printE2eNavigationCacheWarmth` (commit→hydration by warmth)
  > - `printE2eHydrationVsApi` (hydration budget vs API budget)
  > - `printNavigationPhaseComponentHotspots` (commit_to_hydration/wasm_init/
  >   leptos_hydrate/post_hydrate_effects)
  > - `printHydrationRuntimeComponents` (`e2e.hydration_runtime_json`)
  >
  > Decide the exact cut with CSR context in hand (which attributes are still
  > emitted post-CSR). Milestone: Devtool migration (or E2E test suite).

  Label `tooling`.

- [x] **Step 2: Record the issue number** here: **#228**.

_No commit — tracker action only._

---

### Task 2: Command-first vertical slice (parse + analyze + render + CLI)

The runnable spine: `cargo xtask traces analyze <file>` parses the JSONL and
prints the **slowest-spans** section (plus the no-spans / project-filter
framing), rejects `--json`, and parses every flag. Everything is reachable from
the command, so the commit is dead-code-clean. Later tasks grow
`Analysis`/`render` additively.

**Files:**

- Create: `xtask/src/traces/mod.rs`
  (`pub mod parse; pub mod analyze; pub mod render;`)
- Create: `xtask/src/traces/parse.rs`, `xtask/src/traces/analyze.rs`,
  `xtask/src/traces/render.rs`
- Create: `xtask/src/traces/testdata/otel-traces-sample.jsonl` (the fixture)
- Modify: `xtask/src/lib.rs` — `mod traces;` (by line 5); `Command::Traces`;
  `TracesCommand`; `command_name`; `produces_json_payload`; `run()` `--json`
  guard + dispatch arm; `cli_tests`
- Modify: `xtask/Cargo.toml` — add `tabled` (pin to the resolved `Cargo.lock`
  version after `cargo add tabled --manifest-path xtask/Cargo.toml`)

**Interfaces (produced — later tasks depend on these exact names/types):**

```rust
// xtask/src/traces/parse.rs
use std::path::Path;
use anyhow::Result;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Span {
    pub trace_id: String, pub span_id: String, pub parent_span_id: String,
    pub name: String, pub method: String, pub uri: String,
    pub project: String,  // e2e.project
    pub busy_ns: String, pub idle_ns: String,
    pub duration_ms: f64,
    pub source: String,   // input file path this span came from (Node span.source)
    pub raw: Value,       // raw span object, for on-demand e2e.*_json reads
}

#[derive(Debug, Default, Clone)]
pub struct Filters { pub trace: Option<String>, pub project: Option<String> }

/// `attributes[]` string read: stringValue, else stringified intValue, else "".
pub fn get_attr(span: &Value, key: &str) -> String;
/// ms from (endTimeUnixNano - startTimeUnixNano).
pub fn parse_duration_ms(span: &Value) -> f64;
/// Parse JSONL content → spans, applying filters. `source` labels errors and
/// each Span. A malformed line is a hard Err (Node :113-117).
pub fn parse_spans(content: &str, filters: &Filters, source: &str) -> Result<Vec<Span>>;
/// Read a file then `parse_spans`; errors name the path.
pub fn read_spans(path: &Path, filters: &Filters) -> Result<Vec<Span>>;

// xtask/src/traces/analyze.rs
use std::path::PathBuf;
use crate::traces::parse::{Filters, Span};

#[derive(Debug, Default)]
pub struct Analysis {
    pub span_count: usize,
    pub project_filter: Option<String>,
    pub slowest_spans: Vec<SlowSpanRow>,   // ALL spans, duration_ms desc (not sliced)
    // Task 3 adds: slowest_e2e_tests, by_project, trace_totals
    // Task 4 adds: action/nav/long-task/resource vecs
    // Task 5 adds: cache-warmth/hydration/component vecs
}
#[derive(Debug, Clone)]
pub struct SlowSpanRow {
    pub duration_ms: f64, pub trace_id: String, pub name: String,
    pub method: String, pub uri: String, pub busy_ns: String,
    pub idle_ns: String, pub source: String,
}
/// Compute Analysis from parsed spans — no I/O, no process (crit. 4). Rows are
/// FULLY sorted; `top` slicing is render's job.
pub fn analyze_spans(spans: Vec<Span>, project_filter: Option<String>) -> Analysis;
/// Read + parse every input, then `analyze_spans`. Copies `filters.project` into
/// `Analysis.project_filter`.
pub fn analyze(inputs: &[PathBuf], filters: Filters) -> Result<Analysis>;

// xtask/src/traces/render.rs
/// Full report text. span_count==0 → the no-spans line. project_filter Some and
/// span_count>0 → prefix "Project filter: <n>\n\n". Ranked sections sliced to
/// `top`; rendered with `tabled`.
pub fn render(analysis: &Analysis, top: usize) -> String;

// xtask/src/lib.rs
#[derive(Subcommand)]
pub enum TracesCommand {
    Analyze {
        #[arg(long, default_value_t = 25, value_parser = clap::value_parser!(u64).range(1..))]
        top: u64,
        #[arg(long)] trace: Option<String>,
        #[arg(long)] project: Option<String>,
        #[arg(required = true)] files: Vec<std::path::PathBuf>,
    },
}
// Command gains: Traces(TracesCommand)
impl Command { pub fn produces_json_payload(&self) -> bool; } // false only for Traces(Analyze)
```

Filter semantics (Node `readSpans` :131-142): skip spans whose `traceId !=`
`filters.trace` when set; when `filters.project` is set, skip a span **only if**
its `name` starts with `"e2e."` and its `e2e.project != project` (HTTP spans
pass).

- [x] **Step 1: Author the fixture** `otel-traces-sample.jsonl`

  Hand-craft JSONL (one `resourceSpans` record per line) exercising, per spec
  §8: ≥2 traces; several `e2e.test` spans carrying `e2e.project`, `e2e.test`,
  `e2e.action_count`, `e2e.request_count`, and **each** `e2e.*_json` attribute
  (`action_top_json`, `navigation_top_json`, `long_tasks_json`,
  `resource_summary_json`, `hydration_runtime_json`, `request_top_slow_json`);
  some HTTP spans with `method`/`uri`/`busy_ns`/`idle_ns`; at least one
  **negative/non-finite** metric inside a `*_json` blob; navigation/resource
  entries with real **URLs**; one **malformed embedded** `e2e.*_json` attribute.
  Small but total — every section must have data. (Only slowest-spans is
  asserted in this task; Tasks 3–5 assert the rest against this same fixture.)

- [x] **Step 2: Write the failing tests**

  ```
  // parse.rs #[cfg(test)]
  const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");

  test get_attr_string_then_int_then_empty:
      get_attr(span,"method")=="GET"; get_attr(span,"n")=="42"; get_attr(span,"x")==""
  test parse_duration_ms_from_unix_nanos:
      span start "1000000" end "2500000" → 1.5
  test parse_spans_walks_resource_scope_spans:
      one line with spans=[A,B] → len()==2; each Span.source=="sample"
  test parse_spans_malformed_line_is_hard_error:
      parse_spans("{bad\n", &Filters::default(), "t").is_err()  // err mentions "t"
  test parse_spans_trace_filter:
      Filters{trace:Some("aa"),..} keeps only the "aa" span
  test parse_spans_project_filter_only_affects_e2e_spans:
      spans {e2e.test/firefox},{e2e.test/chromium},{GET http}; project=firefox
      → keeps firefox e2e.test AND the http span, drops chromium (len==2)
  test read_spans_file_not_found_errors:
      read_spans(Path::new("/no/such.jsonl"), &Filters::default()).is_err()
  test parse_spans_empty_content_is_empty_vec:
      parse_spans("", &Filters::default(), "t").unwrap().is_empty()

  // analyze.rs #[cfg(test)]
  const FIXTURE: &str = include_str!("testdata/otel-traces-sample.jsonl");
  fn spans() -> Vec<Span> { parse_spans(FIXTURE,&Filters::default(),"sample").unwrap() }
  test slowest_spans_sorted_desc_and_complete:
      a=analyze_spans(spans(),None); a.slowest_spans sorted by duration_ms desc;
      a.slowest_spans.len()==spans().len();  a.span_count==spans().len()
  test analyze_reads_files:  // I/O path
      write FIXTURE to a temp file; analyze(&[temp],Filters::default()).slowest_spans
      == analyze_spans(spans(),None).slowest_spans

  // render.rs #[cfg(test)]
  test render_empty_is_no_spans_message:
      render(&Analysis::default(),25).contains("No spans found in the provided trace files.")
  test render_project_filter_header:
      Analysis{project_filter:Some("firefox".into()),span_count:1, one slowest row}
      → render(..).starts_with("Project filter: firefox")
  test render_slices_ranked_section:
      Analysis with 30 slowest_spans → render(..,top=5) shows 5 data rows in that section
  test render_uses_tabled_not_manual_padding:
      render output for the slowest-spans section contains tabled's border glyphs

  // lib.rs cli_tests (extend)
  test traces_analyze_parses_flags_and_files:
      parse ["xtask","traces","analyze","--top","40","--project","firefox","a.jsonl","b.jsonl"]
      → top==40, project==Some("firefox"), files==[a,b]; command_name()=="traces-analyze"
  test traces_analyze_requires_a_file:
      parse ["xtask","traces","analyze"].is_err()
  test traces_analyze_top_must_be_positive:
      parse ["xtask","traces","analyze","--top","0","a.jsonl"].is_err()
  test produces_json_payload_false_only_for_traces_analyze:
      Traces(Analyze{..}).produces_json_payload()==false;
      Check{..}.produces_json_payload()==true; AuditWasm{..}.produces_json_payload()==true
  test run_rejects_json_for_traces_analyze:
      run(Cli{json:true, command:Traces(Analyze{files:vec!["x".into()],..})}).is_err()
      // and the Err message mentions --json / structured output
  ```

- [x] **Step 3: Run the tests, verify they fail**

  Run: `cargo test --manifest-path xtask/Cargo.toml` Expected: FAIL —
  module/types/variants not defined.

- [x] **Step 4: Implement the slice**
  - `parse.rs`: `get_attr` (:70-83), `parse_duration_ms`, the
    `resourceSpans[].scopeSpans[].spans[]` walk building each `Span` (scalars
    via `get_attr`/`parse_duration_ms`, `source` from the arg,
    `raw = span.clone()`), filters per the semantics above, hard-error on a bad
    line, file-not-found error in `read_spans`.
  - `analyze.rs`: `analyze_spans` computes `span_count` + `slowest_spans`
    (`printSlowest` :189-214 — all spans, duration desc, **not** sliced) and
    stores `project_filter`; `analyze` loops `read_spans` then `analyze_spans`.
  - `render.rs`: `render` emits the no-spans line when empty, the
    `Project filter:` header when set, and the slowest-spans section as a
    `tabled` table sliced to `top`. Use a `#[derive(Tabled)]` display struct
    with ms formatted to 3 decimals (`#[tabled(display_with=…)]` or a mapped
    `String`).
  - `lib.rs`: add `Traces(TracesCommand)`; `command_name` arm →
    `"traces-analyze"`; `produces_json_payload` → `false` only for that variant;
    in `run()` **before** the match:
    `if cli.json && !cli.command.produces_json_payload() { anyhow::bail!("--json is not supported for `{}` (produces no structured output)", cli.command_name()); }`;
    a dispatch arm building `Filters`, calling `analyze`, printing
    `render(&analysis, top as usize)` to stdout, pushing
    `StepResult::ok("traces-analyze")`, `finalize`.

- [x] **Step 5: Run, verify pass.** Run:
      `cargo test --manifest-path xtask/Cargo.toml` → PASS.

- [x] **Step 6: Commit**
  ```bash
  git add xtask/src/traces/ xtask/src/lib.rs xtask/Cargo.toml
  git commit -m "feat(xtask): traces analyze — parse + CLI + slowest-spans; --json rejection (#32)"
  ```
  Run `cargo xtask check` first — must be green (static + clippy + host xtask
  suite).

---

### Task 3: The three remaining always/simple sections

**Files:** Modify `xtask/src/traces/analyze.rs`, `xtask/src/traces/render.rs`.

**Interfaces:** added to `Analysis`:

```rust
pub slowest_e2e_tests: Vec<E2eTestRow>,   // name=="e2e.test", duration desc
pub by_project: Vec<ByProjectRow>,        // e2e.test grouped by project, avg desc — ALL rows
pub trace_totals: Vec<TraceTotalRow>,     // per-trace sum, desc

#[derive(Debug, Clone)] pub struct E2eTestRow {
    pub duration_ms: f64, pub project: String, pub actions: u64,
    pub requests: u64, pub trace_id: String, pub test: String,
}
#[derive(Debug, Clone)] pub struct ByProjectRow {
    pub project: String, pub tests: usize, pub avg_ms: f64, pub max_ms: f64,
    pub avg_actions: f64, pub avg_requests: f64,
}
#[derive(Debug, Clone)] pub struct TraceTotalRow {
    pub trace_id: String, pub total_ms: f64, pub spans: usize,
}
```

- [x] **Step 1: Write the failing tests** (extend `analyze.rs` tests over the
      fixture)

  ```
  test slowest_e2e_tests_only_e2e_test_spans:
      every row ↔ a name=="e2e.test" span; sorted duration desc;
      actions/requests from e2e.action_count/e2e.request_count
  test by_project_groups_and_averages:
      one row per distinct e2e.project among e2e.test spans; tests/avg_ms/max_ms/
      avg_actions/avg_requests match a hand-computed fixture expectation; avg_ms desc
  test trace_totals_sum_per_trace:
      total_ms==sum(member durations); spans==member count; total_ms desc
  ```

  Pin exact numbers where feasible.

- [x] **Step 2: Run, verify fail.**
      `cargo test --manifest-path xtask/Cargo.toml traces::analyze` → FAIL.

- [x] **Step 3: Implement** faithfully: `printSlowestE2eTests` (:216-249),
      `printE2eByProject` (:1017-1067, all rows), `printTraceTotals`
      (:1070-1096). Extend `render` to emit these three sections (by-project
      prints all rows). A section whose vec is empty is skipped, except
      slowest/by-project/trace-totals which always print when their spans exist.

- [x] **Step 4: Run, verify pass.** → PASS.

- [x] **Step 5: Commit**
  ```bash
  git add xtask/src/traces/analyze.rs xtask/src/traces/render.rs
  git commit -m "feat(xtask): traces analyze — e2e-tests, by-project, trace-totals (#32)"
  ```

---

### Task 4: JSON-attribute hotspot sections

**Files:** Modify `xtask/src/traces/analyze.rs`, `render.rs`, `parse.rs` (add
the two helpers here, each with a caller in this commit), `xtask/Cargo.toml`
(add `url`).

**Deps:** `to_url_path` uses the **`url`** crate
(`cargo add url --manifest-path xtask/Cargo.toml`) — `Url::parse` → `host_str` +
`:port` (default port omitted, as Node's `URL.host`) + `path()` (always ≥ `/`);
parse error → the raw string, empty input → `""`. No hand-rolled URL parsing.

**Interfaces:** add to `parse.rs`:

```rust
/// Parse a JSON-string attribute; Value::Null on missing/unparseable — the silent
/// fallback (Node parseJsonAttr :85-95). Callers treat Null as empty.
pub fn parse_json_attr(span: &Value, key: &str) -> Value;
/// URL → host+pathname; unparseable → raw; empty/non-string → "" (toUrlPath :306-316).
pub fn to_url_path(value: &str) -> String;
```

add to `Analysis`:

```rust
pub action_hotspots: Vec<HotspotRow>,           // e2e.action_top_json, max desc
pub navigation_phase_hotspots: Vec<HotspotRow>, // nav phase totals
pub navigation_targets: Vec<TargetRow>,         // slow nav targets (to_url_path)
pub long_task_hotspots: Vec<HotspotRow>,        // e2e.long_tasks_json by name
pub long_task_by_project: Vec<LongTaskProjectRow>, // ALL rows (top ignored)
pub resource_initiators: Vec<HotspotRow>,
pub resource_assets: Vec<AssetRow>,

#[derive(Debug, Clone)] pub struct HotspotRow {
    pub name: String, pub count: usize, pub avg_ms: f64, pub max_ms: f64, pub total_ms: f64,
}
#[derive(Debug, Clone)] pub struct TargetRow {
    pub target: String, pub count: usize, pub avg_ms: f64, pub max_ms: f64, pub total_ms: f64,
}
#[derive(Debug, Clone)] pub struct LongTaskProjectRow {
    pub project: String, pub tests: usize, pub task_count: usize,
    pub avg_per_test_ms: f64, pub max_ms: f64,
}
#[derive(Debug, Clone)] pub struct AssetRow {
    pub name: String, pub initiator: String, pub count: usize,
    pub avg_ms: f64, pub max_ms: f64, pub total_ms: f64,
}
```

- [x] **Step 1: Write the failing tests**

  ```
  // parse.rs
  test parse_json_attr_null_on_missing_or_bad:
      missing → Null; attr stringValue "{bad" → Null; stringValue "[1,2]" → json!([1,2])
  test to_url_path_cases:
      "https://h:8080/a/b?q=1"→"h:8080/a/b"; "not a url"→"not a url"; ""→""
  // analyze.rs (over the fixture)
  test action_hotspots_from_action_top_json:
      aggregates {name,durationMs} across e2e.action_top_json; count/total/avg
      match; max_ms desc
  test navigation_phase_and_targets:
      phase totals cover the addPhase set (:373-387); targets keyed by
      to_url_path(navigation.url), dropping totalMs null/<0; both max desc
  test long_tasks_hotspots_and_by_project:
      hotspots by task name (default "longtask"), negatives dropped; by_project
      one row per project (tests/task_count/avg_per_test_ms/max_ms); by_project NOT sliced
  test resource_initiators_and_assets:
      from e2e.resource_summary_json.topSlow; initiator default "unknown";
      asset name via to_url_path; negatives dropped; max desc
  ```

- [x] **Step 2: Run, verify fail.** → FAIL.

- [x] **Step 3: Implement** `parse_json_attr`/`to_url_path`, then
      `printE2eActionHotspots` (:252-304), `printE2eNavigationHotspots`
      (:322-448), `printE2eLongTaskHotspots` (:513-602),
      `printE2eResourceHotspots` (:604-703). Keep every `isFinite`/`<0` guard
      and each section's sort key; fully sort. Extend `render` for these
      sections (long-task-by-project prints all rows).

- [x] **Step 4: Run, verify pass.** → PASS.

- [x] **Step 5: Commit**
  ```bash
  git add xtask/src/traces/parse.rs xtask/src/traces/analyze.rs xtask/src/traces/render.rs
  git commit -m "feat(xtask): traces analyze — action/nav/long-task/resource hotspots (#32)"
  ```

---

### Task 5: Cache-warmth, hydration-vs-API, phase/runtime components

**Files:** Modify `xtask/src/traces/analyze.rs`, `render.rs`.

**Interfaces:** add to `Analysis`:

```rust
pub cache_warmth: Vec<CacheWarmthRow>,                    // ALL rows (top ignored)
pub hydration_vs_api: Vec<HydrationVsApiRow>,
pub nav_phase_component_samples: Vec<PhaseSampleRow>,
pub nav_phase_component_targets: Vec<PhaseTargetRow>,
pub nav_phase_component_by_project: Vec<PhaseProjectRow>,
pub hydration_runtime_samples: Vec<RuntimeSampleRow>,
pub hydration_runtime_by_project: Vec<RuntimeProjectRow>,

#[derive(Debug, Clone)] pub struct CacheWarmthRow {
    pub cache_warmth: String, pub project: String, pub count: usize,
    pub avg_ms: f64, pub max_ms: f64,
}
#[derive(Debug, Clone)] pub struct HydrationVsApiRow {
    pub hydration_ms: f64, pub api_ms: f64, pub ratio: Option<f64>,
    pub project: String, pub trace_id: String, pub test: String,
}
#[derive(Debug, Clone)] pub struct PhaseSampleRow {
    pub phase: String, pub project: String, pub ms: f64, pub trace_id: String, pub target: String,
}
#[derive(Debug, Clone)] pub struct PhaseTargetRow {
    pub phase: String, pub project: String, pub target: String,
    pub count: usize, pub avg_ms: f64, pub max_ms: f64,
}
#[derive(Debug, Clone)] pub struct PhaseProjectRow {
    pub phase: String, pub project: String, pub count: usize, pub avg_ms: f64, pub max_ms: f64,
}
#[derive(Debug, Clone)] pub struct RuntimeSampleRow {
    pub component: String, pub project: String, pub ms: f64, pub trace_id: String, pub test: String,
}
#[derive(Debug, Clone)] pub struct RuntimeProjectRow {
    pub component: String, pub project: String, pub count: usize, pub avg_ms: f64, pub max_ms: f64,
}
```

- [x] **Step 1: Write the failing tests** (over the fixture)

  ```
  test cache_warmth_by_warmth_and_project:
      groups commitToHydrationMs (>=0) by (warm|cold, project); avg/max/count;
      avg_ms desc; NOT sliced
  test hydration_vs_api_budget:
      hydration_ms=Σ nav commitToHydrationMs(>0); api_ms=Σ request_top_slow_json
      durations whose url contains "/api/" (>0); ratio=hyd/api or None when api==0;
      hydration_ms desc
  test navigation_phase_components:
      phases [commit_to_hydration,wasm_init,leptos_hydrate,post_hydrate_effects];
      samples/targets/by-project from e2e.navigation_top_json; <0 dropped
  test hydration_runtime_components:
      components [hydration,wasm_resource,wasm_init,leptos_hydrate,post_hydrate_effects];
      samples + by-project from e2e.hydration_runtime_json; <0 dropped
  ```

- [x] **Step 2: Run, verify fail.** → FAIL.

- [x] **Step 3: Implement** `printE2eNavigationCacheWarmth` (:450-511, all
      rows), `printE2eHydrationVsApi` (:705-774),
      `printNavigationPhaseComponentHotspots` (:776-922),
      `printHydrationRuntimeComponents` (:924-1015). Preserve field lists and
      guards; fully sort. Extend `render` for these sections (cache-warmth
      prints all rows).

- [x] **Step 4: Run, verify pass.** → PASS.

- [x] **Step 5: Add a render section-order test** (cheap ordering lock)

  ```
  test render_emits_sections_in_canonical_order:
      render over the full-fixture Analysis lists the section headers in Node
      main()'s order (:1139-1150): slowest → e2e-tests → action → nav → cache-warmth
      → long-task → resource → hydration-vs-api → nav-phase-components →
      hydration-runtime → by-project → trace-totals
  ```

  Implement/verify → PASS.

- [x] **Step 6: Commit**
  ```bash
  git add xtask/src/traces/analyze.rs xtask/src/traces/render.rs
  git commit -m "feat(xtask): traces analyze — cache-warmth, hydration, components (#32)"
  ```

---

### Task 6: Docs repoint + Node-vs-Rust equivalence check

**Files:** Modify `CONTRIBUTING.md`, `docs/observability.md`.

**Interfaces:** none (docs + a one-time dev check).

- [x] **Step 1: Repoint docs**

  Replace `scripts/analyze-otel-traces` invocation examples in `CONTRIBUTING.md`
  (:256-273) and `docs/observability.md` (:51-89) with
  `cargo xtask traces analyze <files…>`, preserving the flag descriptions. Leave
  all `scripts/run-e2e-trace-analysis` references intact (retired in #33); both
  scripts remain on disk.

- [x] **Step 2: One-time Rust-vs-Node equivalence diff** (dev-time validation) —
      confirmed identical statistics across all twelve sections on the fixture.

  ```bash
  node scripts/analyze-otel-traces xtask/src/traces/testdata/otel-traces-sample.jsonl > /tmp/node.out
  cargo run --manifest-path xtask/Cargo.toml -- traces analyze xtask/src/traces/testdata/otel-traces-sample.jsonl > /tmp/rust.out
  ```

  Confirm same sections, same rows, same statistics (borders/formatting differ
  by design). Reconcile any **statistical** difference by fixing `analyze` (+
  its test). Not an enduring test — the script is deleted in #33.

- [x] **Step 3: Full gate**

  Run `cargo xtask check` — must be green (static + clippy + host xtask unit
  suite
  - coverage for the rest of the tree).

- [x] **Step 4: Commit**
  ```bash
  git add CONTRIBUTING.md docs/observability.md
  git commit -m "docs(observability): point trace analysis at cargo xtask traces analyze (#32)"
  ```
