# Issue #868 Boot Frame Skew Implementation Outline

> Execute with `jaunder-iterate`; delegate individual slices with
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> changes the e2e trace JSON schema and analyzer contract.

## Scope

In:

- Add frame-skew bridge diagnostics to e2e navigation summaries.
- Extend `cargo xtask traces boot-phases` to certify/report bridge coverage,
  closure, and attribution rows.
- Capture and analyze the approved sqlite single-worker corpus.
- Publish the finding in `docs/observability.md`.

Out:

- Application boot optimization.
- Production wasm delivery/preload/cache/bundle changes.
- Reinterpreting historical app boot phases by subtracting bridge fields.
- Closing #870, #895, #1103, or #1138.

## Task outline

- [x] Task 1: Add bridge fields to e2e capture
  - Contract: `NavigationSummary` gains nullable diagnostic fields with stable
    names: `frameSkewSchema: "bridge-v1" | null`, `documentTimeOriginMs`,
    `documentBootTotalMs`, `commitToDocumentStartMs`, `mountDoneToBindingMs`,
    and `frameSkewRemainderMs`.
  - Contract: values are populated only when `committedMs`, `mountedMs`, and the
    full document timing inputs exist; missing or malformed inputs produce
    `null`, not synthesized values.
  - Contract: these fields are bridge diagnostics. They do not alter
    `bootPhases`, `commitToMountMs`, `mountToSettledMs`, or request/action
    summaries.
  - Verification: focused TypeScript tests for complete, missing, and malformed
    timing plus `devtool run -- tsc -p end2end/tsconfig.json --noEmit`.

- [x] Task 2: Teach `boot-phases` to report bridge certification
  - Contract: Rust parsing reads the Task 1 field names from
    `e2e.navigation_top_json` / trace navigation JSON and groups by existing
    project/cache-warmth/experiment-arm dimensions.
  - Contract: output reports bridge coverage, mean ± SE for
    `commitToDocumentStartMs`, `mountDoneToBindingMs`, and
    `frameSkewRemainderMs`, plus closure status using the spec's thresholds:
    full mounted-navigation coverage, mean absolute remainder ≤ 1.0 ms/nav, max
    absolute per-navigation remainder ≤ 2.0 ms.
  - Contract: rows that fail coverage or closure are labelled non-decisive; the
    analyzer must not print bridge rows as app boot segments.
  - Verification: Rust analyzer tests for complete rows, incomplete coverage,
    closure failure, and bridge rows outside the boot decomposition table.

- [x] Task 3: Smoke the capture path
  - Contract: run the existing focused boot-marks e2e path and inspect one trace
    row for the new fields before any corpus capture.
  - Verification: `devtool run -- cargo xtask e2e-local boot-marks.spec.ts`,
    then `cargo xtask traces boot-phases <extracted trace>` reports bridge
    coverage and closes on the smoke corpus.

- [x] Task 4: Capture the deciding corpus
  - Contract: preserve tarballs under
    `~/measurements/jaunder/issue-868-frame-skew/` using three valid sqlite
    single-worker runs per engine, distinct `e2eSalt` per run, and order
    `chromium→firefox`, `firefox→chromium`, `chromium→firefox`.
  - Contract: record the host-quiescence statement in the finding before reading
    the result.
  - Verification: tarballs exist, extract to JSONL, and analyzer reports
    complete bridge coverage plus closure for decisive rows; otherwise record
    the instrument finding and stop before attribution.

- [x] Task 5: Publish the finding
  - Contract: `docs/observability.md` gets the certified corpus path, run order,
    salt names, bridge coverage, closure status, bridge term table, and the
    dominant/split/non-decisive conclusion using the spec's 2/3-share plus 3×SE
    rule.
  - Contract: historical #866 app boot segment tables remain unchanged except
    for a forward pointer to the new finding if useful.
  - Verification: `devtool run -- prettier -w docs/observability.md` and
    `devtool run -- cargo xtask check`.

## Risk checks

- ADR-0100: bridge fields explain measurement-frame skew only; they are never
  summed into app boot phases.
- Schema compatibility: absent historical bridge fields parse as missing and
  yield non-decisive bridge certification, not analyzer crashes.
- Corpus discipline: no attribution claim without three valid runs per engine,
  distinct salts, counterbalanced order, full bridge coverage, and closure
  within the registered tolerance.
- Commit discipline: each committed slice is checked with the relevant focused
  verification plus the repo gate required by `jaunder-commit`.
