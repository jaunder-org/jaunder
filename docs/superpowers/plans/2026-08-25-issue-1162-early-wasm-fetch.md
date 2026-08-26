# Issue #1162 Early Wasm Fetch Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch` when useful. This outline exists because the change creates
> a shell-to-wasm initializer protocol and a pre-registered performance
> decision.

## Scope

In:

- Paired baseline/candidate SQLite single-worker captures for Chromium and
  Firefox on the same quiescent host.
- One shared early-fetch contract across the CSR and projector shells.
- Transparent wasm-bindgen consumption, active parser/callsite migration, drift
  guards, browser proof, and the measured retention verdict.

Out:

- Stylesheet loading changes, preload/modulepreload, response cloning or byte
  buffering, retries, a second fetch, viewer-dependent shell content, generated
  initializer replacement, and cross-frame timing comparisons.
- Historical archived spec/plan edits.

## Task outline

- [x] Task 1: Capture the unmodified baseline corpus
  - Contract: create
    `~/measurements/jaunder/issue-1162-early-wasm-fetch/{baseline,candidate}`
    and `load.tsv` with columns `arm`, `round`, `phase`, `load1`, `load5`,
    `load15`. For baseline q1..q3, use salt `issue1162-baseline-q<q>`, record
    `/proc/loadavg` before and after, discard contaminated rounds, and order
    engines Chromium→Firefox for q1/q3 and Firefox→Chromium for q2. Do not edit
    product code between rounds. Each engine build uses its exact target:
    `devtool run -- nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-chromium-single-worker`
    or
    `devtool run -- nix build --print-out-paths --no-link .#packages.x86_64-linux.e2e-sqlite-firefox-single-worker`.
    For each output, immediately run
    `devtool run -- tar -xzf <out>/capture-sqlite.tar.gz -C ~/measurements/jaunder/issue-1162-early-wasm-fetch/<arm>/ capture/otel-traces.jsonl`
    followed by
    `mv ~/measurements/jaunder/issue-1162-early-wasm-fetch/<arm>/capture/otel-traces.jsonl ~/measurements/jaunder/issue-1162-early-wasm-fetch/<arm>/<arm>-q<q>-<engine>.jsonl`
    before extracting the other engine. Restore `e2eSalt = ""` after q3.
  - Verification: six uniquely named `baseline-q<q>-<engine>.jsonl` files exist;
    `load.tsv` has paired rows for all accepted rounds;
    `devtool run -- cargo xtask precommit` proves no salt or formatter mutation
    remains.

- [x] Task 2: Pin the initializer prerequisite
  - Contract: generator coverage proves pinned wasm-bindgen accepts an awaited
    `Promise<Response>` and carries the resolved response through its existing
    streaming and buffered paths. `initMeasured` remains a transparent
    forwarding wrapper around `__wbg_init`.
  - Verification:
    `devtool run -- cargo test --manifest-path tools/devtool/Cargo.toml csr_bundle`
    passes before either shell changes. If the contract is absent, make no
    starter or consumer change; record the prerequisite failure and retained
    existing boot behavior in `docs/observability.md`.

- [x] Task 3: Start one early request on both shell surfaces
  - Contract: `web::app::EARLY_WASM_FETCH_SCRIPT` assigns
    `window.__jaunderWasmFetch` exactly once from `fetch(WASM_URL)` without
    reading viewer state. CSR order is unchanged `PREPAINT_SCRIPT`, starter,
    metadata/stylesheets; projector order relies on `document` writing
    `PREPAINT_SCRIPT` before `render_head`, which emits the starter before both
    stylesheets.
  - Verification: `devtool run -- cargo xtask test-local -- -p web app::render`
    and
    `devtool run -- cargo xtask test-local -- -p jaunder projector::document`
    prove shared bytes, ordering, cacheability, and absence of both preload
    forms.

- [x] Task 4: Consume the response promise through the existing initializer
  - Contract: both module scripts call exactly
    `initMeasured(window.__jaunderWasmFetch ?? "/pkg/jaunder.wasm")` immediately
    after the existing `jaunder.module.before_init` mark. Migrate every active
    literal parser and callsite: `audit_wasm`, web drift guards, router
    coverage, and projector ordering guards.
  - Verification:
    `devtool run -- cargo test --manifest-path tools/devtool/Cargo.toml csr_bundle`
    retains the Task 2 prerequisite proof;
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml audit_wasm`
    proves URL derivation and rejection of a missing explicit fallback.

- [ ] Task 5: Prove one-request and no-flash browser behavior
  - Contract: extend the existing boot-marks test rather than adding a separate
    navigation test. Retain pathname-based request counting and require one wasm
    request, `path: "streaming"`, and non-null wasm/style document timings.
    Timing order remains evidence, not a browser gate. Retain the authenticated
    owner pre-paint contract unchanged.
  - Verification: `devtool run -- cargo xtask e2e-local boot-marks.spec.ts`
    proves the cold `/` navigation contract, and
    `devtool run -- cargo xtask e2e-local authed-flash.spec.ts` proves the
    existing authenticated-owner pre-paint/no-flash behavior.

- [ ] Task 6: Capture candidate data and publish the retention verdict
  - Contract: repeat Task 1's complete build, extract, immediate-rename, load,
    salt, contamination, and engine-order protocol with candidate salts and
    filenames. Analyze all 12 files individually with
    `cargo xtask traces boot-phases`, then all six explicit paths per arm for
    coverage certification. For each engine, compute the three paired
    candidate-minus-baseline round means, their sample standard deviation, and
    `mean(d) - 4.3026527299 * s / sqrt(3)`.
  - Verification: `docs/observability.md` records mechanism, commands, load
    rows, complete-row/request counts, direct-init schema coverage, per-round
    and paired engine results, `wasmFetchStartMs - styleMaxResponseEndMs`,
    browser caveats, and a kept/reverted verdict. Keep only when Task 5's
    no-flash and one-request proofs pass, Task 2's initializer support is
    proven, and neither engine has both `mean(d) > 0` and a positive lower
    bound. Restore `e2eSalt = ""` and run `devtool run -- cargo xtask precommit`
    before commit.

## Risk checks

- The unchanged synchronous `PREPAINT_SCRIPT` remains the first script in both
  heads; existing authenticated-owner pre-paint/no-flash coverage stays green.
- `WASM_URL` is the Rust URL source of truth; the only explicit module fallback
  is required by the wire contract and audited against it.
- wasm-bindgen remains the sole response-body consumer; no clone, retry, second
  fetch, or silent network fallback is introduced.
- Both shell surfaces retain render-blocking stylesheets, no resource hints, and
  static anonymous projector HTML.
- Document-frame marks/resource timings are compared only with document-frame
  values; `commitToMountMs` remains separate.
- Measurement rounds run without other builds, tests, or agent workloads; any
  contaminated round is discarded and repeated before analysis.
