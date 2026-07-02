# Observability

This project emits OpenTelemetry traces from both the backend and end-to-end
test runner.

## Backend

- Backend spans are produced via `tracing` + OpenTelemetry in the `server`
  crate.
- In e2e VM checks, traces are exported to the in-VM collector and written to:
  - `/var/lib/jaunder/otel-traces.jsonl` (inside VM)
  - `otel-traces-sqlite.jsonl/otel-traces.jsonl` (copied artifact)
  - `otel-traces-postgres.jsonl/otel-traces.jsonl` (copied artifact)

## End-to-End Tracing Layers

E2E tracing currently has two complementary layers:

- `e2e.test` (automatic, from `end2end/tests/fixtures.ts`)
  - one span per test
  - request timing summary
  - navigation lifecycle summary (`e2e.navigation_top_json`)
  - each navigation record includes `cacheWarmth` (`cold` for first document
    navigation in the test, `warm` for subsequent ones)
  - includes `commit -> hydration` timing when hydration is observed
  - resource summary
  - timed action summary (`e2e.action_top_json`)
- `e2e.flow.*` (manual semantic phases, from `end2end/tests/perf.ts`)
  - opt-in for selected scenarios
  - mark-to-mark phase timing for domain-specific flow analysis

Both layers use the same trace context (`JAUNDER_E2E_TRACEPARENT`) so browser
and backend spans are correlated in a single trace.

## Per-test timing report

Each e2e VM check also runs Playwright's `json` reporter and copies the result
out as a flat artifact (alongside the OTEL traces above):

- `playwright-report-sqlite.json` / `playwright-report-postgres.json`

It records every test's title, project (browser), status, retries, and duration.
This is the primary source for per-test timing comparisons across browsers (e.g.
the Firefox-vs-Chromium analysis in #152). On the
`cargo xtask e2e <backend> <browser>` path it lands per combo at
`.xtask/diagnostics/e2e-<backend>-<browser>/playwright-report-<backend>.json`
and is uploaded as the `e2e-diagnostics-<backend>-<browser>` CI artifact.

## Analysis

Use `scripts/analyze-otel-traces` on one or more artifact files, for example:

```bash
scripts/analyze-otel-traces \
  /nix/store/...-vm-test-run-jaunder-e2e-sqlite-chromium/otel-traces-sqlite.jsonl/otel-traces.jsonl \
  /nix/store/...-vm-test-run-jaunder-e2e-postgres-firefox/otel-traces-postgres.jsonl/otel-traces.jsonl
```

The analyzer reports:

- slowest spans overall
- slowest `e2e.test` spans
- top e2e action hotspots
- top navigation phase hotspots and slow targets
- navigation `commit -> hydration` split by `cacheWarmth`
- per-project/browser e2e duration breakdown
- per-navigation hydration component hotspots (`wasm_init`, `leptos_hydrate`,
  `post_hydrate_effects`, `commit_to_hydration`) by sample, target, and project
- hydration runtime component hotspots from `e2e.hydration_runtime_json` (per
  test and per project)
- per-trace duration totals

To run both e2e VM checks and immediately analyze the produced traces, use:

```bash
scripts/run-e2e-trace-analysis --top 25
```

For cold-cache diagnostics (without `JAUNDER_E2E_WARMUP=1` in the VM checks),
use:

```bash
scripts/run-e2e-trace-analysis --cold --top 25
```

Optional filters:

- `--top N` controls how many rows each section prints.
- `--trace TRACE_ID` restricts analysis to one trace id.
- `--cold` runs the per-browser cold packages
  (`e2e-{sqlite,postgres}-{chromium,firefox}-cold`) instead of the default
  warmup e2e checks.
- `--browser chromium|firefox` restricts the run to one browser (default: both).
  Use this (not `--project`) to focus one browser, e.g. when debugging Firefox
  timeout pressure: `scripts/run-e2e-trace-analysis --browser firefox`.

(`--project NAME` is a flag of the underlying `scripts/analyze-otel-traces`, not
of `run-e2e-trace-analysis`.)

## #155 — post-CSR Firefox e2e tax (findings, 2026-07-02)

Re-measurement of the #152 Firefox-vs-Chromium tax on the **leptos-CSR** build
(post-#180; no SSR, no hydration reconciliation). Method: the four warm
`e2e-{sqlite,postgres}-{chromium,firefox}` checks, per-test durations paired
from `playwright-report-<backend>.json`, attribution from
`scripts/analyze-otel-traces`.

**The tax barely moved after the CSR cutover.** Median per-test Firefox/Chromium
ratio:

| backend  | median ratio | mean | tests ≥1.4× | suite total (ff / ch)   |
| -------- | ------------ | ---- | ----------- | ----------------------- |
| sqlite   | **1.83×**    | 1.80 | 61/66       | 585.8s / 336.8s (1.74×) |
| postgres | **1.69×**    | 1.69 | 62/66       | 623.5s / 376.0s (1.66×) |

Compare #152's SSR-era median **1.90×**. Removing hydration did **not** collapse
the gap — strong evidence the cost was never hydration-specific but ongoing
WASM/JS execution + rendering, which CSR still runs in Firefox.

**The delta is uniform and client-side, not server-side.** Distribution peaks in
the 1.7–2.2× bucket (45/66 sqlite) with only 1–4 tests <1.4× — uniform, not a
few hot tests. Attribution (sqlite chromium vs firefox traces):

- `e2e.test` avg: firefox 6813ms vs chromium 3802ms (1.79×), with **identical**
  avg actions (13.78) and firefox making **fewer** requests (31 vs 37) — so it
  is not doing more server work.
- The delta lives in **`navigation.commit_to_hydration`** (post-CSR this phase
  measures commit → CSR mount-ready, _not_ hydration): firefox 1123ms vs
  chromium 559ms = **2.01×**. The `wait.hydration` action (the mount-ready wait)
  is the single largest action bucket (655ms avg × 302 = 198s).
- Server-side phases are browser-invariant and small: `navigation.request` ~88ms
  avg; API fetches (`/api/current_user` 27ms, etc.) are browser-independent.

**Verdict (AC2): the per-test Firefox tax is irreducible at the per-test level**
— inherent SpiderMonkey-vs-V8 WASM-execution cost, uniform across the suite,
with no hot test to optimize and no hydration left to remove. Therefore **worker
parallelism is the only lever on Firefox e2e wall-clock** (see #182, folded into
#155); per-test tuning is not pursued.

**Per-browser floor (for the Task-6 timeout reconciliation):** at workers:1,
firefox `e2e.test` avg 6.8s / max 21.2s, chromium avg 3.8s / max 11.9s; measured
ratio ~1.7–1.83× vs the current `hydrationHeavyTimeoutScale = 2.2` — the scale
is in the right ballpark (a modest trim, not removal), but the `hydrationHeavy*`
naming is a misnomer post-CSR (the phase is CSR mount, not hydration).

## Timeout Budgeting

E2E tests that are hydration-heavy should use
`hydrationHeavyTimeoutMs(testInfo, chromiumBudgetMs)` from
`end2end/tests/fixtures.ts` instead of hard-coded timeout numbers.

For first document navigation in a test (typically the coldest path), use
`hydrationHeavyFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)`.

This applies a project-aware multiplier derived from observed p90 hydration
latency so Firefox/WebKit runs get realistic budgets without increasing Chromium
timeouts unnecessarily.

For diagnostics, you can optionally warm each Playwright test page context
before instrumentation starts:

```bash
JAUNDER_E2E_WARMUP=1 playwright test
```

Optional controls:

- `JAUNDER_E2E_WARMUP_URL` (default `http://localhost:3000/`)
- `JAUNDER_E2E_WARMUP_TIMEOUT_MS` (default `10000`)

This warmup runs on the same test page/context and waits for
`body[data-hydrated]`, so subsequent navigations within that test are measured
as warm-cache behavior.

### Heavy timeline fixture seeding (#210)

The three heavy timeline tests (`posts.spec.ts` `:305`/`:349`/`:410`) seed their
paginated fixtures through the `test-support` binary (ADR-0046) — one in-process
storage write per post — rather than a sequential loop of
`POST /api/create_post` round-trips. That removes the setup cost `#155`
mitigated with worker-contention timeout headroom (`workerContentionScale` in
`end2end/tests/fixtures.ts`), so that headroom is now a candidate for reduction
once `workers>1` is unblocked (`#173`). The before/after measurement is driven
separately by the `#152` `run-e2e-trace-analysis` harness; the timeouts are not
re-tuned here.

## WASM Bundle Audit

Use `cargo xtask audit-wasm` to measure frontend bundle size from the
deterministic Nix `site` build output:

```bash
cargo xtask audit-wasm
```

This reports raw, gzip, and brotli sizes for:

- `pkg/jaunder_bg.wasm`
- `pkg/jaunder.js`

Useful options:

- `--json` for machine-readable output
- `--site-path /nix/store/...-jaunder-site` to reuse a previously built site
  output
