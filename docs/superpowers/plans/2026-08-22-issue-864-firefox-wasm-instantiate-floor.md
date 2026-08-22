# Firefox WASM Initialization Floor Implementation Outline

> Execute with `jaunder-iterate` and delegate task slices with
> `jaunder-dispatch` when useful. This outline exists because the cycle needs a
> stable measurement-arm contract across Nix bundle outputs, browser capture,
> trace analysis, and a quiescent final corpus.

## Scope

In:

- Pre-register a delivery-preserving firefox-deciding experiment for #864.
- Add only the experiment-arm and arm-integrity plumbing needed to distinguish
  module-shape and engine-setup candidates in current `direct-init-v1` traces.
- Capture/analyze a certified corpus on a quiescent machine and write the result
  to `docs/observability.md`.

Out:

- No wasm bundle optimization, preload/modulepreload, shell delivery-path
  change, navigation-count change, or generic observability framework.
- No claim from the historical `wasm_instantiate` residual.
- No decisive per-module/setup contrast if preserving request URL, initiator,
  `wasmInitPath`, and resource-size invariants proves impossible.

## Task outline

- [ ] Task 1: Pre-register the #864 experiment arms
  - Contract: Record the run matrix before capture: firefox deciding, chromium
    control, sqlite single-worker, distinct `e2eSalt` per run, randomized or
    counterbalanced realized arm order, quiescent host requirement, delivery
    invariants, expected direction and approximate magnitude (or bounded
    qualitative prediction) for each included candidate, and null-result
    interpretation. Baseline arm is the current bundle. Shape arm must be
    verified by actual module-shape counts, not branch names. Engine-setup arm
    is included only if it keeps delivery/API invariants; if it cannot, stop and
    either select another separable size-independent candidate under the same
    corpus or return for spec approval before reducing the experiment to one
    candidate.
  - Verification: Pre-registration exists in `docs/observability.md` or a linked
    issue-local note before any final capture, and the wording identifies the
    independent in-trace discriminator for every included arm.

- [x] Task 2: Add experiment-arm and module-shape discriminators to capture
  - Contract: Existing gate attrs remain unchanged. Experiment attrs or explicit
    bundle options produce distinct arms while preserving `/pkg/jaunder.wasm`,
    shell import, `initMeasured`, compression/content negotiation, and
    `direct-init-v1`. Navigation JSON carries a closed arm id and actual served
    wasm shape counts such as raw bytes, decoded bytes, imports, imported
    functions, functions, exports, tables, memories, and code bytes. Malformed
    or absent discriminator data becomes `null`, not a thrown capture failure.
  - Verification: Focused tests prove current default traces keep the same
    schema and experiment traces surface arm/shape fields independent of
    `wasmApiMs`/`wasmInitMs`. Run `devtool run -- cargo xtask check` before the
    task commit.

- [ ] Task 3: Add focused #864 trace certification/reporting
  - Contract: Analyzer grouping includes source/project/cache-warmth/arm where
    arm exists. It reports current `direct-init-v1` counts, direct-complete and
    closure status, dropped/truncated populations, arm-order reconciliation,
    quiescence metadata presence, shape-count integrity, delivery-invariant
    checks, and run-mean firefox-first timing with uncertainty for every
    reported contrast across `wasmApiMs`, `wasmInitMs`, and the exclusive
    document-frame boot fields. It never sums overlapping direct diagnostics
    into ADR-0100's exclusive decomposition.
  - Verification: Rust analyzer tests cover a passing certified two-arm corpus,
    missing arm discriminator, confounded arm order, non-current legacy rows,
    and delivery-invariant drift. Run the focused test lane first, then
    `devtool run -- cargo xtask check` before the task commit.

- [ ] Task 4: Run the quiescent corpus and publish the finding
  - Contract: Capture uses the pre-registered matrix and records host
    quiescence, realized arm order, salts, commands, output paths, and corpus
    certification. Firefox decides; chromium is a control. If an included arm
    fails integrity or delivery-invariant checks, the result is reported as
    non-decisive for that candidate rather than repaired in prose.
  - Verification: `cargo xtask traces analyze`/`boot-phases` or the focused #864
    analyzer certifies the extracted traces before analysis.
    `docs/observability.md` records corpus path, certification, arm integrity,
    tested candidates, measured results, and remaining unknowns. Run
    `devtool run -- cargo xtask check` before the task commit.

## Risk checks

- ADR-0100: document-frame exclusive decomposition stays
  `wasmInitStartMs + wasmInitStartToBootEntryMs + bootPhases = bootTotalMs`; do
  not use Node `commitToMountMs` as a closure target.
- #887: `wasmApiMs` and `wasmInitMs` are overlapping diagnostics and include
  wasm-bindgen/Rust-start behavior; never present them as compile-only
  durations.
- #866/ADR-0121: no preload retry and no delivery-dependence claim from residual
  reattribution.
- Arm integrity comes from actual trace/artifact facts: module-shape counts,
  request/resource invariants, and realized arm order; not from filename or run
  intent alone.
- Existing gate and non-experiment e2e attrs must remain byte-for-byte
  behaviorally unchanged unless the task explicitly proves the changed contract.
