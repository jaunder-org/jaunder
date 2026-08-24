# Issue #870 Stylesheet/Wasm Fetch Implementation Outline

> Execute with `jaunder-iterate`; delegate individual slices with
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> changes the e2e trace JSON schema and analyzer contract.

## Trigger

Planning trigger: capture schema + analyzer output contract, plus quiescent
corpus discipline. The spec remains authoritative.

## Scope

In:

- Add a module-before-init mark to both shell surfaces without changing shell
  order.
- Capture stylesheet resource timings and derived stylesheet/module deltas in
  navigation summaries.
- Teach analyzer output/tests to certify/report stylesheet diagnostic coverage
  and ordering.
- Capture/analyze the approved sqlite single-worker corpus while the host is
  quiescent.
- Publish the finding in `docs/observability.md`.

Out:

- Default stylesheet loading changes.
- Preload/modulepreload/defer/async stylesheet behavior.
- FOUC-risking product optimization.
- Node-frame reinterpretation of app boot phases.
- Closing #869, #895, #1103, or #1138.

## Task outline

- [ ] Task 1: Add shell mark and capture fields
  - Contract: both `csr/index.html` and the Rust-rendered projector shell mark
    the document immediately before `initMeasured()` with a stable
    `jaunder.module.before_init` name.
  - Contract: no shell ordering changes: ADR-0044 pre-paint script stays in
    `<head>`, stylesheet links stay before the module script, static glue import
    stays before the mark, and `initMeasured(WASM_URL)` stays explicit.
  - Contract: `NavigationSummary` gains nullable diagnostics named by the spec:
    `moduleBeforeInitMs`, `jaunderCssResponseEndMs`,
    `jaunderThemesCssResponseEndMs`, `styleMaxResponseEndMs`,
    `styleToModuleBeforeInitMs`, and `moduleBeforeInitToWasmFetchStartMs`.
  - Verification: focused TypeScript tests for complete, missing, and malformed
    inputs plus shell drift tests.

- [ ] Task 2: Teach analyzer to report stylesheet certification
  - Contract: parse the Task 1 fields from `e2e.navigation_top_json` and group
    by existing source/project/cache-warmth/experiment-arm dimensions.
  - Contract: report coverage, ordering pass rate, mean ± SE for
    `styleToModuleBeforeInitMs` and `moduleBeforeInitToWasmFetchStartMs`, and
    the share rule inputs for `styleMaxResponseEndMs` versus `wasmFetchStartMs`.
  - Contract: incomplete rows are labelled non-decisive; stylesheet diagnostics
    stay outside the app boot decomposition table.
  - Verification: Rust analyzer tests for decisive coverage, incomplete
    coverage, ordering failure, and non-decomposed placement.

- [ ] Task 3: Smoke the capture path
  - Contract: run the focused boot-marks e2e path and inspect one trace row for
    the new stylesheet/module fields before corpus capture.
  - Verification: `devtool run -- cargo xtask e2e-local boot-marks.spec.ts`,
    extract the trace, then analyzer output reports stylesheet coverage/order
    fields on the smoke corpus.

- [ ] Task 4: Capture the deciding corpus
  - Contract: preserve tarballs under
    `~/measurements/jaunder/issue-870-stylesheet-wasm-fetch/` using three valid
    sqlite single-worker runs per engine, distinct `e2eSalt` per run, and order
    `chromium→firefox`, `firefox→chromium`, `chromium→firefox`.
  - Contract: record the host-quiescence statement in the finding before reading
    the result.
  - Verification: tarballs exist, extract to JSONL, and analyzer reports
    complete diagnostic coverage plus decisive ordering/share rows; otherwise
    record the instrument limitation and stop before attribution.

- [ ] Task 5: Publish the finding
  - Contract: `docs/observability.md` gets the certified corpus path, run order,
    salt names, diagnostic coverage, ordering result, measured shares, and
    verdict under the spec's 95% ordering + half-share rule.
  - Contract: historical #866 tables remain unchanged except a forward pointer
    if useful.
  - Verification: `devtool run -- prettier -w docs/observability.md` and
    `devtool run -- cargo xtask check`.

## Risk checks

- ADR-0044: shell mark must not move, defer, or externalize the pre-paint auth
  script.
- ADR-0100: stylesheet/module timings are document-frame diagnostics only; never
  add them to Node-frame totals or app boot phase sums.
- Schema compatibility: historical traces without these fields parse as
  missing/non-decisive, not analyzer crashes.
- Measurement discipline: no attribution claim without three valid runs per
  engine, distinct salts, counterbalanced order, host quiescence, diagnostic
  coverage, and the registered verdict rule.
- Commit discipline: each committed slice is checked with the relevant focused
  verification plus the repo gate required by `jaunder-commit`.
