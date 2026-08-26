# Issue #1162 — Early single-request wasm fetch

## Outcome

Both cacheable shell surfaces start the wasm bundle request during head parsing,
after synchronous pre-paint authentication and before render-blocking
stylesheets. The existing wasm-bindgen initializer consumes that same response,
so every navigation still requests `/pkg/jaunder.wasm` exactly once.

A same-machine, quiescent, paired browser measurement determines whether the
change is kept. The result and engine-specific behavior are recorded in
`docs/observability.md`.

## Load-bearing decisions

- ADR-0044 remains authoritative: its synchronous pre-paint authentication
  script is the first script in both heads and still runs before any wasm work.
- Stylesheets remain render-blocking. Neither shell adds `preload`,
  `modulepreload`, or another resource hint; ADR-0121's no-preload decision
  remains intact.
- Anonymous projector HTML remains byte-identical for every viewer and
  cacheable. The starter is static and reads no viewer state.
- `web::app::WASM_URL` is the Rust source of truth for the starter and consumer
  fallback URL. The starter contract is the window property
  `window.__jaunderWasmFetch`, conceptually `Promise<Response> | undefined`,
  assigned once from one `fetch` call.
- Both module scripts pass the existing handle or the explicit wasm URL fallback
  to `initMeasured`. The measured wrapper transparently forwards that input to
  wasm-bindgen; wasm-bindgen remains the sole response-body consumer and retains
  its streaming and buffered instantiation paths.
- A rejected starter is not retried and its response is not cloned, consumed, or
  replaced by a second fetch. Network failure continues through the existing
  boot failure path instead of silently double-downloading.
- The explicit URL fallback preserves boot behavior when the starter is absent.
  Generator coverage pins the wasm-bindgen input contract so an incompatible
  generated initializer fails before release.
- Document-frame marks and resource timings are analyzed only with other
  document-frame measurements. They are never mixed with Node-frame
  `commitToMountMs`, preserving ADR-0100.
- The baseline is the unmodified `origin/main`-based issue branch captured
  before product changes. Baseline and candidate each use three quiescent SQLite
  single-worker rounds per Chromium and Firefox on the same host. Round `q` uses
  salt `issue1162-<arm>-q<q>` and records `/proc/loadavg` immediately before and
  after; a round contaminated by other build, test, or agent workload is
  discarded and repeated. Engine order is Chromium then Firefox for q1/q3 and
  Firefox then Chromium for q2.
- Retention is pre-registered as keep unless regression. For each engine, let
  `d1..d3` be the candidate-minus-baseline round-mean `bootTotalMs` values.
  Compute their mean, sample standard deviation `s`, and lower bound
  `mean(d) - 4.3026527299 * s / sqrt(3)`. Regression requires both `mean(d) > 0`
  and that lower bound `> 0` in either engine.
- Browser-specific request serialization is measured evidence, not independently
  an abort condition. The starter is reverted if no-flash or one-request
  invariants fail, either engine meets the registered regression rule, or the
  pinned wasm-bindgen initializer cannot be proven to accept `Promise<Response>`
  under the required delivery constraints. An initializer incompatibility is
  documented as a prerequisite failure; it does not justify buffering bytes,
  replacing the generated initializer, or adding another fetch.

## Acceptance

- Drift guards prove both shell heads contain the existing `PREPAINT_SCRIPT`
  bytes before the starter, the starter before both stylesheets, and module
  import and measured initialization after both stylesheets. Existing
  authenticated-owner pre-paint/no-flash behavioral coverage remains green.
- Drift guards prove the starter and explicit consumer fallback derive from
  `WASM_URL`, measured initialization consumes `window.__jaunderWasmFetch`, and
  neither preload form appears.
- Generator coverage proves `initMeasured` forwards a pre-started response
  promise through wasm-bindgen's streaming/buffered delivery path without
  issuing a second fetch.
- The wasm audit derives the wasm and glue URLs from the new shell contract and
  fails when the initializer lacks its explicit URL fallback.
- Projector coverage proves its rendered head has the shared order and remains
  static and cacheable.
- The existing boot-marks browser test proves a cold `/` navigation makes
  exactly one `/pkg/jaunder.wasm` request, initializes successfully through the
  streaming path, and records non-null wasm and stylesheet document timings.
- Twelve individually analyzed capture files provide complete-row and request
  counts plus per-round Chromium and Firefox means for `bootTotalMs` and
  `wasmFetchStartMs - styleMaxResponseEndMs`; six-file per-arm analyses certify
  corpus coverage.
- `docs/observability.md` records the exact mechanism, corpus commands, host
  loads, request-count completeness, direct-init schema/coverage, paired
  statistics, browser serialization caveats, and an explicit kept or reverted
  verdict under the registered rule.

## Boundaries

- No stylesheet loading change, resource hint, wasm byte buffering, retry,
  response clone, second fetch, viewer-dependent shell content, or new
  cross-frame timing comparison.
- No replacement of wasm-bindgen's generated initializer or its
  `instantiateStreaming` buffered fallback.
- Archived specifications and plans remain historical and unchanged.
