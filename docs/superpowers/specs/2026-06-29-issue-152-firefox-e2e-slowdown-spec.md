# Issue #152 — Firefox e2e combos run ~4 min slower than Chromium

**Date:** 2026-06-29
**Issue:** [#152](https://github.com/jaunder-org/jaunder/issues/152) (surfaced by #129's matrix)
**Status:** spec — approved design

## Problem

After #129 fanned e2e into a `{backend}×{browser}` matrix, per-combo CI timings show **Firefox
runs ~76% slower than Chromium** (sqlite: 578s vs 329s Playwright; postgres similar), and since the
combos run in parallel, Firefox alone sets the e2e wall-clock (~12m vs a ~7m40s Chromium/validate
floor). VM boot is identical (~22s) — the entire delta is *inside* the Playwright run.

We cannot yet tell whether the delta is a **uniform per-navigation hydration tax** or
**concentrated in a few heavy tests**, because the CI Playwright config emits no per-test timing.

## Investigation findings (grounding, 2026-06-29)

- **CI uses the inline `nixPlaywrightConfig`** (flake.nix:430), *not* `end2end/playwright.config.ts`
  (local only). The CI config: `reporter: 'line'`, `workers: 1` (serial), flat `timeout: 30s`, **no
  `retries`** (default 0), no `trace`. So **retries and config-level timeouts are ruled out** as
  causes — the run passed with 0 retries.
- **Per-test timeouts are set by the tests themselves** via `hydrationHeavyTimeoutMs(testInfo, N)`
  and `test.slow()` (pervasive in `posts/auth/feeds/visibility/...`), which scale Firefox's budget.
  So Firefox tests are *designed* to run longer and don't fail — they just take the time.
- The suite is explicitly built around "Firefox hydrates the Leptos WASM bundle slower than
  Chromium": `waitForHydration` gates every nav; `goto/click/waitForSelector` wrap actions in
  `withTimedAction`, which records **per-action OTEL spans**.
- **Visibility gap:** `reporter: 'line'` prints no per-test durations; `build.log` (the uploaded
  artifact) has no per-test/retry data. `otel-traces.jsonl` is `copy_from_vm`'d to the nix `$out`
  (flake.nix:642/774) but was **not** present in the downloaded `e2e-diagnostics-*` CI artifact.

## Guiding principle

**Measurement-first.** Do not attempt a speed fix until per-test (and per-action) timing is a
durable, retrievable artifact and the delta is attributed.

## Phases

### Phase 1 — Visibility (the durable deliverable)

- In `nixPlaywrightConfig` (flake.nix:430), set `reporter: [['line'], ['json', { outputFile:
  '/tmp/e2e/playwright-report.json' }]]` (the suite runs in `/tmp/e2e`; exact path finalized in the
  plan) — keep `line` for live progress, add `json` for structured per-test data (title, project,
  status, duration, retries).
- `copy_from_vm` the JSON report into the diagnostics that get uploaded, mirroring the existing
  `otel-traces.jsonl` copy-out (flake.nix:642 sqlite / :774 postgres) and however `build.log` /
  `jaunder-journal` reach the `e2e-diagnostics-*` artifact.
- Ensure `otel-traces-<backend>.jsonl` is actually included in the uploaded CI artifact (it reaches
  `$out` but was absent from the downloaded artifact) so per-action analysis is reproducible.
- **Exit criterion:** a `cargo xtask e2e <backend> <browser>` run (and CI) produces a per-test JSON
  report retrievable from diagnostics, for both browsers.

### Phase 2 — Measure & root-cause

- Pull the JSON reports for `sqlite/firefox` vs `sqlite/chromium` (same backend isolates the
  browser). Compute per-test deltas; classify **uniform tax vs concentrated** in a few tests.
- Cross-reference the OTEL `withTimedAction` spans to attribute the delta: hydration/navigation
  (`page.goto` + `waitForHydration`) vs interaction (`ui.click`, `wait.selector`).
- Account for: per-test `hydrationHeavyTimeoutMs`/`test.slow()` budgets (these cap, not cause); and
  **fixed `setTimeout` sleeps** (websub.ts, feeds.spec.ts:56, visibility.spec.ts:341) which are
  browser-independent constants and should net out of the firefox−chromium delta.
- **Exit criterion:** a written attribution of the +250s (which tests / which action class).

### Phase 3 — Decide & act

- If a clear win emerges (a handful of tests dominate, or a fixable hydration/navigation wait or a
  removable hard sleep), apply it and re-measure.
- Otherwise, document the findings on #152 and file a **precise** follow-up (e.g. "speed Leptos
  Firefox WASM hydration" or "optimize the N slowest Firefox tests"), then close this cycle on the
  visibility + root-cause deliverable.

## Acceptance criteria

- Per-test e2e timing is a durable artifact in CI diagnostics + local runs (Phase 1), and
  `otel-traces-<backend>.jsonl` is uploaded.
- The Firefox/Chromium delta is attributed (uniform vs concentrated; action class) and written up
  on #152 (Phase 2).
- Either a clear-win speedup is landed, or a precise follow-up issue is filed (Phase 3).
- `cargo xtask validate` is green.

## Out of scope / separable concerns

- **#153** (filed): dedupe the two diverging Playwright configs (local TS vs nix inline).
- Deep Leptos/Firefox WASM-hydration perf work — the likely Phase-3 follow-up, not this cycle.
- webkit (excluded from the VM matrix: WPE SIGABRT, flake.nix:428).

## Affected files (initial)

- `flake.nix` (`nixPlaywrightConfig` reporter + the two `testScript` copy-outs)
- possibly `xtask/src/steps/nix.rs` (if diagnostics collection needs the new file wired in)
- analysis only (no change): `end2end/tests/*` , `end2end/tests/helpers.ts`, otel/json reports
