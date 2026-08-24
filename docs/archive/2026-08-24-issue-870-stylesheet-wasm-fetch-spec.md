# Issue #870 — test stylesheet blocking before wasm fetch

**Status:** draft, awaiting approval

**Issue:** [#870](https://github.com/jaunder-org/jaunder/issues/870)

**Branch:** `issue-870-stylesheet-wasm-fetch`

**Predecessors:** #866 measured the pre-fetch window and reverted wasm preload;
#868 separated Node/document frame skew from app boot timing; ADR-0044 makes the
pre-paint auth script load-bearing; ADR-0100 keeps this analysis in the document
frame.

## Outcome

Determine whether the two render-blocking stylesheet links in the shell are on
the critical path before the wasm fetch starts.

This issue is measurement only. It adds direct document-frame diagnostics for
the existing shell, captures a fresh quiescent corpus, and publishes a finding.
It does not change the default shell loading order, ship a non-blocking
stylesheet strategy, or accept FOUC risk.

## Evidence

#866 measured `document_start → wasm_fetch_start` at **212.5 ms/nav chromium**
and **260.7 ms/nav firefox** in the baseline shell. That section explicitly left
the stylesheet hypothesis open: the pre-fetch window is partly network-shaped,
but a warm-cache residual survives in both engines.

The current shell order is:

1. ADR-0044 pre-paint auth script in `<head>`;
2. `<link rel="stylesheet" href="/style/jaunder.css">`;
3. `<link rel="stylesheet" href="/style/jaunder-themes.css">`;
4. inline `<script type="module">` in `<body>` importing `/pkg/jaunder.js`, then
   calling `initMeasured("/pkg/jaunder.wasm")`.

A static module import means a mark inside the module body is after the glue has
loaded/evaluated, but it is still before the `initMeasured()` call that starts
the wasm fetch. Therefore the useful direct question is: by the time the module
body reaches the `initMeasured()` call, have the stylesheets already completed,
and how much of `document_start → wasm_fetch_start` do stylesheet completions
explain?

## Decisions

- **D1 — Instrument the existing shell, do not change loading behavior.** Add a
  document-frame mark immediately before `initMeasured()` in both
  `csr/index.html` and the Rust-rendered shell. Preserve ADR-0044's pre-paint
  script and the two stylesheet links in their current order.
- **D2 — Capture stylesheet timings as named diagnostics.** Extend navigation
  summaries with nullable fields for:
  - `moduleBeforeInitMs`: the mark immediately before `initMeasured()`;
  - `jaunderCssResponseEndMs` and `jaunderThemesCssResponseEndMs` from
    `PerformanceResourceTiming`;
  - `styleMaxResponseEndMs = max(jaunderCssResponseEndMs, jaunderThemesCssResponseEndMs)`;
  - `styleToModuleBeforeInitMs = moduleBeforeInitMs - styleMaxResponseEndMs`;
  - `moduleBeforeInitToWasmFetchStartMs = wasmFetchStartMs - moduleBeforeInitMs`.
- **D3 — Certify ordering before interpreting.** A navigation is decisive for
  the stylesheet hypothesis only when both stylesheet resource timings, the
  module-before-init mark, and `wasmFetchStartMs` are present and finite. The
  analyzer reports coverage and mean ± SE for the two deltas above. Negative
  `styleToModuleBeforeInitMs` means the module reached `initMeasured()` before a
  stylesheet finished, refuting strict stylesheet-blocked ordering for that row.
- **D4 — Verdict rule.** Stylesheets are reported as on-path only if, in both
  engines, at least 95% of decisive rows have `styleToModuleBeforeInitMs ≥ 0`
  and the per-engine mean `styleMaxResponseEndMs` explains at least half of
  `document_start → wasm_fetch_start`. Otherwise the finding is either
  split/partial or refuted, with measured shares. No product optimization
  follows from this issue.
- **D5 — Use a fresh quiescent corpus.** Capture sqlite single-worker runs for
  chromium and firefox, three valid runs per engine, distinct `e2eSalt` per run,
  and alternating engine order (`chromium→firefox`, `firefox→chromium`,
  `chromium→firefox`). Record the host-quiescence statement before capture and
  preserve tarballs under
  `~/measurements/jaunder/issue-870-stylesheet-wasm-fetch/`.

## Acceptance

- **AC1. Shell mark:** both shell surfaces emit the same module-before-init mark
  immediately before `initMeasured()` without reordering the pre-paint script,
  stylesheet links, module import, or wasm init call.
- **AC2. Capture schema:** each navigation summary records the stylesheet/module
  diagnostics as nullable fields; missing or malformed resource timings remain
  `null`, not guessed.
- **AC3. Analyzer:** `cargo xtask traces boot-phases` or a focused trace
  analyzer reports stylesheet diagnostic coverage, ordering pass rate,
  `styleToModuleBeforeInitMs`, and `moduleBeforeInitToWasmFetchStartMs`, grouped
  by engine/cache warmth.
- **AC4. Tests:** focused TypeScript tests cover complete, missing, and
  malformed stylesheet/module diagnostics; shell drift tests cover both shell
  surfaces; Rust analyzer tests cover decisive and non-decisive rows if Rust
  reporting is changed.
- **AC5. Corpus:** capture a fresh quiescent sqlite single-worker corpus for
  chromium and firefox: three valid runs per engine, distinct `e2eSalt` per run,
  alternating engine order, tarballs preserved, JSONL traces extracted, and
  diagnostics analyzed with the updated tool.
- **AC6. Write-up:** update `docs/observability.md` with the corpus path, run
  order, salts, diagnostic coverage, ordering result, measured shares, and
  conclusion. If ordering coverage is incomplete or the verdict rule fails, say
  so explicitly.
- **AC7. Verification:** run focused shell/capture/analyzer tests and
  `cargo xtask check` before committing.

## Boundaries

- Do not change the default shell stylesheet loading strategy.
- Do not add preload/modulepreload or defer/async stylesheet behavior in this
  issue.
- Do not weaken ADR-0044's pre-paint auth marker or introduce FOUC risk.
- Do not reinterpret Node-frame `commitToMountMs` as a document-frame boot
  phase.
- Do not close #869, #895, #1103, or #1138.
