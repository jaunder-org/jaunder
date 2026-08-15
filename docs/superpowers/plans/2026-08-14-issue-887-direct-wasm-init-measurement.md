# Direct WASM Initialization Measurement Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the false `responseEnd → boot.entry` wasm-instantiation
residual with direct successful-WebAssembly-API and wasm-bindgen-initialization
diagnostics, without changing the application’s document lifecycle or delivery
path.

**Architecture:** `devtool csr-bundle` appends the single measured initializer
to wasm-bindgen’s generated glue; both HTML shells call it fire-and-forget. The
e2e init script observes its completion mark and harvests document timing
separately from boot-phase marks. The summary and Rust analyzer distinguish
exclusive `init_start → boot.entry` closure fields from overlapping direct
diagnostics and classify pre-cutover traces as legacy.

**Tech Stack:** Rust (`tools/devtool`, `xtask`), generated ES modules,
TypeScript/Playwright, OpenTelemetry JSON analysis, existing e2e Nix matrix.

## Review header

**Scope — in:** generated-glue instrumentation; shared-shell consumption;
document timing capture/merge/schema; boot-phase and coverage analysis; focused
browser/unit proofs; ADR-0100, architecture projection, and current
observability prose.

**Scope — out:** alternate byte-first compilation, wasm delivery/size/preload
changes, Node-clock measurements, re-derived historical data, and #864’s
measurement experiment.

**Tasks:**

1. Add and unit-test the generated-glue measured initializer; migrate both
   shells to its one export.
2. Capture and reconcile direct timing without changing lifecycle; atomically
   replace the e2e navigation schema, analyzer closure/coverage/rendering, and
   browser/Rust regression proofs.
3. Project the superseded residual into current measurement documentation and
   run focused contracts plus the per-commit gate.

**Key risks / decisions:** wasm-bindgen runs `__wbindgen_start()` before
`init()` resolves, so `wasmApiMs` and `wasmInitMs` are overlapping diagnostics.
Do not await the helper in either shell. API wrappers record a path only after
success, restore exact identities in `finally`, and keep a rejected-streaming →
buffered fallback attributable to the buffered success. `jaunder.wasm.*` marks
are never Rust `bootPhases`; the completion observer—not `load`—captures a late
successful initializer. Every current summary carries `direct-init-v1`;
unversioned raw traces are legacy, never current instrument loss.

## Global Constraints

- Implement
  [the approved specification](../specs/2026-08-14-issue-887-direct-wasm-init-measurement.md),
  especially D1–D7 and AC1–AC13.
- Preserve ADR-0100’s document-clock-only decomposition. Exact closure is
  `wasmInitStartMs + wasmInitStartToBootEntryMs + bootPhases = bootTotalMs`.
- `wasmApiMs`, `wasmInitMs`, `wasmInitPath`, and resource fields are
  diagnostics; no analyzer path may sum them into the decomposition.
- Preserve fire-and-forget module-script behavior: neither shell may await the
  initializer or add a request.
- The public current schema has no `wasmInstantiateMs` alias. Historical records
  retain their old values and current prose labels them as a superseded
  residual.
- Browser proofs use `./fixtures` and `./helpers`; do not add raw network-idle
  waits or per-test timeout hacks. Run changed contract tests before
  `devtool run -- cargo xtask check`; commit only its checked, staged tree with
  no `Co-Authored-By` trailer.

---

## File structure

- Modify `tools/devtool/src/csr_bundle.rs` — append the canonical generated-glue
  export, keep postprocess ordering before precompression, and unit-test emitted
  JavaScript contract.
- Modify `csr/index.html` and `server/src/projector/document.rs` — import and
  invoke only the canonical export; update Rust drift guards.
- Modify `end2end/tests/capture-trace.ts` — harvest typed wasm marks/details,
  install completion observer/binding, and merge boot/resource/direct-init data
  independently.
- Modify `end2end/tests/fixtures.ts` — define `direct-init-v1`, make
  `bootPhasesFrom` boot-only, derive summary fields, remove the residual.
- Modify `end2end/tests/boot-marks.spec.ts` — prove merge order, one request,
  direct completion capture, phase separation, and real-browser paths.
- Modify `xtask/src/traces/boot_phases.rs` — consume exact new exclusive fields;
  aggregate direct diagnostics and legacy/current denominators.
- Modify `xtask/src/traces/analyze.rs` and `xtask/src/traces/render.rs` — make
  boot coverage read/render the versioned current contract and report
  legacy/current direct-instrument completeness.
- Modify `xtask/src/audit_wasm.rs` — parse the canonical measured shell call
  while retaining the explicit wasm-URL budget guard.
- Modify `docs/observability.md`,
  `docs/adr/0100-measurement-frames-are-not-mixed.md`,
  `docs/adr/0121-no-wasm-preload.md`, and `docs/ARCHITECTURE.md` — distinguish
  current direct metrics from retained historical residual evidence.

## Interfaces and contracts

```ts
// Export appended by `csr-bundle` to generated jaunder.js.
export async function initMeasured(
  moduleOrPath?:
    | string
    | Request
    | URL
    | Response
    | BufferSource
    | WebAssembly.Module,
): Promise<WebAssembly.Exports>;

// Current navigation summary additions/replacements.
type WasmInitPath = "streaming" | "buffered";
type NavigationSummary = {
  wasmTimingSchema: "direct-init-v1";
  wasmInitStartMs: number | null;
  wasmInitStartToBootEntryMs: number | null;
  wasmApiMs: number | null;
  wasmInitMs: number | null;
  wasmInitPath: WasmInitPath | null;
  // no wasmInstantiateMs
};
```

The generated helper emits `jaunder.wasm.init_start` and
`jaunder.wasm.init_done`; completion detail is the only cross-boundary payload:
`{ path: WasmInitPath; apiMs: number }`. Capture treats malformed, absent, or
non-finite detail as null. It preserves the old `DocumentTiming` Rust-mark and
resource data while adding a separately merged wasm-init data member.

### Task 1: Generate one measured initializer and migrate shells

**Files:**

- Modify: `tools/devtool/src/csr_bundle.rs:65-188`
- Modify: `tools/devtool/src/csr_bundle.rs:191-351`
- Modify: `csr/index.html:18-21`
- Modify: `server/src/projector/document.rs:17-40,220-250`
- Modify: `server/tests/web/router.rs`
- Modify: `web/src/app/render.rs:47-65,266-300`
- Modify: `xtask/src/audit_wasm.rs`
- Test: `tools/devtool/src/csr_bundle.rs:191-351`
- Test: `server/src/projector/document.rs:220-250`
- Test: `server/tests/web/router.rs`
- Test: `web/src/app/render.rs:266-300`
- Test: `xtask/src/audit_wasm.rs`

**Interfaces:**

- Consumes: wasm-bindgen’s renamed default initializer and
  `WASM_URL`/`GLUE_URL`.
- Produces: `initMeasured()` appended once to served `jaunder.js`; both shells
  import it and invoke `initMeasured("/pkg/jaunder.wasm")` without `await`.

- [x] **Step 1: Write failing postprocessor, audit, and shell drift tests.** Pin
      emitted glue to export `initMeasured`; assert `performance.now()` times
      the delegated API promise, start/success marks are emitted, successful
      path is recorded only after resolution, exact function identities are
      restored in `finally`, and the append precedes JS precompression. Update
      projector/web/router assertions to require the canonical import/call and
      reject an awaited call or local WebAssembly wrapper. Update `audit_wasm`’s
      shell parser/tests to accept that canonical call while preserving its
      explicit wasm-URL guard.
- [x] **Step 2: Run focused failures.** Run
      `devtool run -- cargo nextest run --manifest-path tools/devtool/Cargo.toml     csr_bundle`,
      the targeted `web`/`server` tests, and
      `devtool run -- cargo     nextest run --manifest-path xtask/Cargo.toml audit_wasm`;
      observe failures for the absent export and old shell calls.
- [x] **Step 3: Implement the generated-glue append.** Keep the input glue’s
      default export intact; append an exported wrapper accepting the default
      initializer’s full
      `string | Request | URL | Response | BufferSource |     WebAssembly.Module`
      input surface. It marks `jaunder.wasm.init_start`, temporarily replaces
      only available `WebAssembly.instantiateStreaming` / `instantiate`
      functions, uses document-frame `performance.now()` around each delegated
      promise, accepts a path/duration only after success, emits completion
      detail only after outer success, and restores exact originals in
      `finally`.
- [x] **Step 4: Migrate both shell consumers and the audit parser.** Replace
      their `init` imports/calls with fire-and-forget `initMeasured`; update the
      embedded-shell, cross-shell URL/import, and `audit_wasm` guards rather
      than duplicating a new string convention.
- [x] **Step 5: Run focused passing contracts.** Re-run the Step 2 commands;
      verify the helper timer contract, URL parity, no-await behavior, embedded
      fallback, and bundle audit.
- [x] **Step 6: Commit the isolated generated-bundle/shell deliverable.** Tick
      this task, run `devtool run -- cargo xtask check`, inspect/stage its exact
      changes, then commit `feat(devtool): measure wasm initializer`.

### Task 2: Capture direct timing and replace the browser summary schema

**Files:**

- Modify: `end2end/tests/capture-trace.ts:168-305,326-424`
- Modify: `end2end/tests/fixtures.ts:88-139,659-727`
- Modify: `end2end/tests/boot-marks.spec.ts:1-140`
- Test: `end2end/tests/boot-marks.spec.ts`
- Test: `xtask/src/traces/boot_phases.rs:480-820`
- Test: `xtask/src/traces/analyze.rs:982-1160`
- Test: `xtask/src/traces/render.rs`

**Interfaces:**

- Consumes: `jaunder.wasm.init_start` / `init_done` marks and completion detail
  emitted by Task 1; existing `__jaunderRecordMount` pattern.
- Produces: versioned `NavigationSummary` direct fields; boot-only phase map; a
  dedicated completion-harvest binding that merges independently of Rust marks.

- [ ] **Step 1: Write failing pure capture/merge tests.** Add snapshots where
      mount-ready arrives first, load arrives first, completion arrives third
      after an earlier `settle()` pass, and completion is absent. Assert the
      fullest boot marks survive while completed wasm-init data survives
      independently; malformed detail becomes null; settlement drains a
      completion-harvest scheduled after its first queue snapshot without
      waiting forever on a hung initializer.
- [ ] **Step 2: Write failing browser regression tests.** Extend the existing
      piggy-backed boot test to assert one wasm request, `direct-init-v1`,
      non-negative direct fields, `wasmApiMs <= wasmInitMs`, `streaming`, and
      boot-only interval names. Add controlled browser scenarios for unavailable
      streaming; streaming rejection plus a non-`application/wasm` MIME response
      that makes real wasm-bindgen fall back to buffered bytes; thrown/hung
      completion through a traced load-only timing accessor; and exact restored
      API identities. Use specific event/mark waits, not network idle.
- [ ] **Step 3: Write failing analyzer and renderer fixtures.** Replace
      residual-based `boot_phases`/coverage builders with versioned summaries;
      pin exact closure, direct medians and their counts, path/current/missing/
      legacy denominators, malformed/missing data, and rendered labeled output.
- [ ] **Step 4: Run focused failures.** Run
      `devtool run -- cargo xtask     e2e-local boot-marks.spec.ts`,
      `devtool run -- cargo nextest run -p xtask boot_phases`, and
      `devtool run     -- cargo nextest run -p xtask boot_coverage`; confirm new
      contracts fail.
- [ ] **Step 5: Implement typed capture and summary cutover.** Preserve prefix
      discovery for raw evidence, parse direct mark detail separately, and make
      `bootPhasesFrom` filter strictly to `jaunder.boot.` endpoints. Derive
      `wasmInitStartMs` and `wasmInitStartToBootEntryMs` from named marks;
      derive `wasmInitMs` from ordered `init_done - init_start`; derive only
      `wasmApiMs`/`wasmInitPath` from valid completion detail. Add
      `direct-init-v1` to every current summary and remove the
      `responseEnd → boot.entry` residual.
- [ ] **Step 6: Implement completion observation and drainable reconciliation.**
      Register a dedicated binding and init-script `PerformanceObserver` before
      document code. Bind each completion to its navigation and track its
      harvest in a per-navigation handoff that `settle()` repeatedly drains;
      completion may arrive after mount/load harvests, but an absent completion
      never blocks settlement. Merge boot marks/resource fields and direct-init
      fields independently, preserving callback-order independence.
- [ ] **Step 7: Implement analyzer and renderer in the same cutover.** Replace
      `WASM_SEGMENTS` with the two new exclusive fields and retain only `boot.`
      intervals for closure. Extend population, coverage, and rendered tables
      with direct diagnostic medians/counts, successful path counts,
      current/missing counts, and legacy count. Require exact `wasmTimingSchema`
      for current parsing; never map legacy residuals into new fields or sum
      diagnostics/resource timing into closure.
- [ ] **Step 8: Pass focused cross-layer contracts.** Re-run the Step 4 commands
      in Chromium and Firefox as needed for AC2–AC11, including unavailable and
      real MIME-eligible rejection fallback plus late completion reconciliation.
- [ ] **Step 9: Commit the atomic schema/analyzer deliverable.** Tick this task,
      run `devtool run -- cargo xtask check`, inspect/stage all capture,
      analyzer, renderer, and test changes, then commit
      `feat(e2e): capture and     analyze direct wasm initialization`.

### Task 3: Project the measurement contract and verify the integrated change

**Files:**

- Modify: `docs/observability.md` — replace current residual explanations and
  tables; retain historical values explicitly labeled as superseded.
- Modify: `docs/adr/0100-measurement-frames-are-not-mixed.md` — retain the frame
  decision; name direct API/init diagnostics and the new closure segments.
- Modify: `docs/adr/0121-no-wasm-preload.md` — label its retained 125 ms
  `wasm_instantiate` observation as the superseded response-end residual.
- Modify: `docs/ARCHITECTURE.md:1229-1248` — maintain the ADR materialized view.
- Modify:
  `docs/superpowers/plans/2026-08-14-issue-887-direct-wasm-init-measurement.md`
  — tick completed plan tasks as execution evidence; do not mutate the approved
  specification.
- Test: `end2end/tests/boot-marks.spec.ts`
- Test: `xtask/src/traces/boot_phases.rs`
- Test: `xtask/src/traces/analyze.rs`

**Interfaces:**

- Consumes: concrete current output from Tasks 1–2 and ADR-0100’s amended
  document-frame invariant.
- Produces: current-reader prose that cannot mistake retained historical
  residuals for direct WebAssembly measurements.

- [ ] **Step 1: Write documentation assertions/search targets.** Enumerate every
      current `wasmInstantiateMs` / `wasm_instantiate` reference and classify it
      as historical evidence, archived material, or a required current
      replacement. Do not rewrite archived reports/specifications.
- [ ] **Step 2: Update current documentation.** State exact current field
      semantics, non-additive analyzer treatment, fire-and-forget/observer
      capture behavior, and direct API versus initialization distinction. Amend
      ADR-0100 and ADR-0121 without weakening their no-mixed-clocks/historical
      evidence meanings; keep `ARCHITECTURE.md` aligned.
- [ ] **Step 3: Run focused integrated proof.** Run
      `devtool run -- cargo xtask     e2e-local boot-marks.spec.ts`,
      `devtool run -- cargo nextest run -p xtask boot_phases`, and
      `devtool run     -- cargo nextest run -p xtask boot_coverage`; confirm
      only documented historical residual references remain outside archives.
- [ ] **Step 4: Run per-commit gate.** Run `devtool run -- cargo xtask check`.
      Inspect and stage every mechanical formatter/fix output with the docs.
- [ ] **Step 5: Commit the projection and integrated proof.** Tick this task and
      commit `docs: describe direct wasm initialization measurement` from the
      checked staged tree.

## Final verification

- [ ] Run `devtool run -- cargo xtask validate` after the task commits on the
      final branch head. Treat every browser/backend combination as required.
- [ ] Run the repository’s branch review and spec/plan conformance review before
      shipping; resolve every actionable finding.
- [ ] Keep this plan and its approved spec live until `jaunder-ship` archives
      them after final rebase and final validation.
