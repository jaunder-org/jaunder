# #887 — measure wasm initialization directly

Issue: [#887](https://github.com/jaunder-org/jaunder/issues/887). Milestone:
Observability & diagnostics. Prerequisite for #864.

## Summary

The browser timing schema currently calls the interval from the wasm resource's
`responseEnd` to `jaunder.boot.entry` `wasmInstantiateMs`. That value is a
residual, not an instantiation measurement: delivery changes can move work
across `responseEnd`, and the residual absorbs every unmarked activity before
Rust starts. It therefore cannot support #864's attribution claim.

Jaunder will directly measure two application-visible operations:

- the successful WebAssembly API call made by wasm-bindgen; and
- the enclosing wasm-bindgen `init()` operation.

It also records whether the successful API path was streaming or buffered.
Neither duration is called compile-only: `instantiateStreaming` overlaps
delivery, while wasm-bindgen calls `wasm.__wbindgen_start()` before `init()`
resolves, so initialization includes Jaunder's synchronous Rust boot and mount.

Both direct durations are overlapping diagnostics. The exact boot-total
decomposition remains exclusive by replacing the false residual with the
observed interval from `init_start` to `boot.entry`. Resource fetch timing and
byte sizes remain overlapping delivery diagnostics.

## Evidence and boundaries

Generated wasm-bindgen glue performs the relevant work in this order:

1. fetch or accept the wasm input;
2. call `WebAssembly.instantiateStreaming`, falling back to
   `WebAssembly.instantiate` over buffered bytes when necessary;
3. call `wasm.__wbindgen_start()`;
4. resolve `init()`.

Jaunder emits `jaunder.boot.entry` and the remaining Rust boot marks during
step 3. Therefore `init_done` occurs after `boot.entry` and normally after
`mount_done`. Adding `wasmInitMs` to the Rust boot phases would double-count
them; deriving `init_done → boot.entry` would produce a negative interval.

The browser exposes no portable compile-only timer. Timing the successful
WebAssembly API promise is nevertheless the direct boundary #887 needs:
`wasmApiMs` covers `instantiateStreaming` on the normal path or
`WebAssembly.instantiate` on the buffered path. Those paths have different
delivery overlap and are compared only with `wasmInitPath` beside them. Forcing
a byte-first compile would change the delivery path under study, so this issue
does not add an alternate boot mode or claim to isolate compilation.

## Decisions

### D1 — Measure the successful WebAssembly API and public initializer

The bundle exports one Jaunder-owned measured initializer around wasm-bindgen's
normal async initializer. It emits exact document-frame marks
`jaunder.wasm.init_start` immediately before the call and
`jaunder.wasm.init_done` only after successful resolution. Their difference is
`wasmInitMs`.

Temporary wrappers time each WebAssembly API promise with `performance.now()`.
The duration and path of the call that succeeds become `wasmApiMs` and
`wasmInitPath`. A rejected streaming attempt is not reported as successful and
its duration gets no separate field; it remains included in the enclosing
`wasmInitMs` and `wasmInitStartToBootEntryMs`. The subsequent successful
buffered call supplies `wasmApiMs` and `wasmInitPath: "buffered"`.

The helper preserves the shipped wasm-bindgen delivery behavior. It does not
pre-fetch, precompile, or replace streaming with a byte-first path.

### D2 — One implementation serves every document shell

`devtool csr-bundle`, the existing shared host/Nix bundle postprocessor, appends
the measured initializer to generated `jaunder.js`. Both the SPA shell and the
server-projected document import and invoke that same exported helper without
awaiting it, preserving the existing document lifecycle. There is no second
wrapper implementation, additional network request, or new
`DOMContentLoaded`/`load` blocker.

The helper temporarily wraps the available WebAssembly instantiation functions
for the duration of its call, delegates with the original receiver and
arguments, records an API only after its promise resolves, and restores the
exact original functions in `finally`. A successful streaming attempt records
`streaming`; a successful buffered instantiation records `buffered`, including
fallback after a failed streaming attempt.

### D3 — Success is explicit; failure stays incomplete

The completion mark carries closed detail
`{ path: "streaming" | "buffered", apiMs: number }`. Unknown, malformed,
non-finite, or absent detail is harvested as null rather than accepted as a new
path or duration.

If initialization throws or never completes, `wasmInitMs`, `wasmApiMs`, and
`wasmInitPath` are null; no zero, path inference, or synthetic completion is
emitted. The start mark remains observable.

### D4 — Wasm marks and Rust boot phases are distinct

`bootPhasesFrom` constructs intervals only from marks whose names start with
`jaunder.boot.`. The `jaunder.wasm.*` marks are harvested independently and
cannot create an interval that the analyzer mistakes for a Rust boot phase.

The mount-ready and load harvests can both precede `init_done`. The e2e init
script therefore installs a `PerformanceObserver` before document code runs; on
`jaunder.wasm.init_done` it requests a third harvest through a dedicated
Playwright binding. Production boot emits only the mark and remains
fire-and-forget. `mergeDocumentTiming` reconciles the contracts independently:
it retains the fullest Rust boot mark set and completed wasm-init data whenever
any snapshot has it. This does not rely on harvest completion order.

### D5 — Exclusive and overlapping fields are separate

Every current navigation summary carries `wasmTimingSchema: "direct-init-v1"`,
even when the document reports no timing. Its measurement vocabulary is:

| Field                                      | Meaning                                    | Role                    |
| ------------------------------------------ | ------------------------------------------ | ----------------------- |
| `wasmInitStartMs`                          | document `timeOrigin → init_start`         | exclusive boot segment  |
| `wasmInitStartToBootEntryMs`               | `init_start → jaunder.boot.entry`          | exclusive boot segment  |
| `wasmApiMs`                                | successful WebAssembly API promise         | overlapping diagnostic  |
| `wasmInitMs`                               | `init_start → successful init resolution`  | overlapping diagnostic  |
| `wasmInitPath`                             | successful API path: streaming or buffered | overlapping diagnostic  |
| existing `wasmFetch*` and wasm byte fields | resource delivery                          | overlapping diagnostics |
| existing `bootPhases`                      | `boot.entry → mount_done` intervals        | exclusive boot segments |

The exclusive segments close exactly to `bootTotalMs`:

```text
wasmInitStartMs
+ wasmInitStartToBootEntryMs
+ bootPhases
= jaunder.boot.mount_done.startTime
```

`wasmApiMs`, `wasmInitMs`, `wasmInitPath`, and resource timing are reported
beside that decomposition but never summed into it.

### D6 — Analyzer output makes instrument coverage visible

For each existing `(source, project, cacheWarmth)` population, boot-phase
analysis reports:

- total current navigations and exactly decomposed navigations;
- current navigations with and without complete direct-init diagnostics;
- `streaming` and `buffered` counts;
- medians for `wasmApiMs` and `wasmInitMs`, each with its contributing count;
- closure violations; and
- legacy-schema navigations.

Direct-init completeness requires finite `wasmApiMs` and `wasmInitMs` plus a
recognized path. It is independent of exclusive-decomposition completeness: a
row may close without having post-mount `init_done`, and that loss must remain
visible.

### D7 — Clean current cutover; explicit legacy classification

`wasmInstantiateMs` is removed from the current TypeScript schema, capture
assembly, analyzer current-schema contract, analyzer fixtures, and current
measurement documentation. No compatibility alias remains.

Raw navigation summaries without `wasmTimingSchema: "direct-init-v1"` are
classified and counted as legacy, not as current instrument loss. A legacy
`wasmInstantiateMs` value is neither copied into a current field nor used in the
new decomposition.

Historical reports and archived specifications retain their recorded field and
segment names because their numbers cannot be recomputed. Current prose that
references those results labels `wasmInstantiateMs` / `wasm_instantiate` as the
superseded response-end residual, not a direct measurement.

ADR-0100 is amended without changing its clock-frame decision: browser boot is
still decomposed only in the document frame. Its concrete segment list changes
to the exclusive fields in D5. `docs/ARCHITECTURE.md` remains the corresponding
materialized view.

## Acceptance criteria

- **AC1 — One canonical initializer without lifecycle drift.** The served
  `jaunder.js` exports one Jaunder-owned measured initializer, and both SPA and
  projected document shells invoke it without awaiting it. Neither shell
  contains its own WebAssembly wrapper implementation; boot adds no network
  request and does not delay `DOMContentLoaded` or `load`.
- **AC2 — Direct durations.** A successful boot reports finite, non-negative
  `wasmApiMs` for the successful WebAssembly API promise and `wasmInitMs` for
  `init_start → init_done`, with `wasmApiMs <= wasmInitMs`.
- **AC3 — Actual streaming path.** A normal boot whose successful load uses
  `WebAssembly.instantiateStreaming` reports `wasmInitPath: "streaming"`.
- **AC4 — Actual unavailable-streaming path.** With `instantiateStreaming`
  unavailable, buffered instantiation succeeds and reports
  `wasmInitPath: "buffered"`.
- **AC5 — Actual rejection fallback.** When `instantiateStreaming` is present
  but rejects and buffered instantiation succeeds, both engines report
  `wasmInitPath: "buffered"`; `wasmApiMs` measures the successful buffered call,
  not the rejected streaming attempt.
- **AC6 — Restoration.** After successful initialization, failed initialization,
  and rejection fallback, the page's WebAssembly instantiation functions have
  the same identities they had before the measured call.
- **AC7 — Fail incomplete.** A start without successful completion yields
  `wasmApiMs: null`, `wasmInitMs: null`, and `wasmInitPath: null`; it never
  yields zero or an inferred path.
- **AC8 — Phase separation and exact closure.** Browser output proves
  `bootPhases` contains only `jaunder.boot.* → jaunder.boot.*` intervals.
  Analyzer tests prove
  `wasmInitStartMs + wasmInitStartToBootEntryMs + bootPhases` closes to
  `bootTotalMs`, and continue to count incomplete or non-closing navigations.
- **AC9 — Observable diagnostics.** Analyzer output labels `wasmApiMs` and
  `wasmInitMs` medians with contributing counts, reports streaming/buffered and
  complete/missing counts against the total current-navigation denominator, and
  never adds direct-init or resource diagnostics to the exclusive decomposition.
- **AC10 — Reconciled capture.** A real navigation summary retains the full Rust
  boot mark set and completed direct-init data when mount-ready, load, and the
  completion observer resolve in any order. A hung initialization remains
  fire-and-forget, so load can harvest its start-only incomplete state.
- **AC11 — Clean versioned cutover.** Every current navigation carries
  `wasmTimingSchema: "direct-init-v1"`; current contracts contain no
  `wasmInstantiateMs` alias. Unversioned raw traces are reported as legacy and
  are not counted as current instrument loss or decomposed through old values.
- **AC12 — Historical evidence is honest.** Historical values remain available,
  and current prose explicitly describes them as the superseded response-end
  residual.
- **AC13 — Both engines and backends.** Focused browser proofs cover streaming,
  unavailable streaming, rejection fallback, restoration, capture
  reconciliation, and boot closure in Chromium and Firefox; the full repository
  gate passes all Chromium/Firefox × SQLite/PostgreSQL combinations.

## Out of scope

- isolating WebAssembly compilation from instantiation or Rust startup;
- changing wasm size, optimization level, preload policy, caching, or delivery;
- rerunning #864's controlled measurement session or making a Firefox root-cause
  claim;
- redefining Node-frame `commitToMountMs`, `mountToSettledMs`, or frame skew;
- correcting or re-deriving historical measurements.
