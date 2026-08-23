# Issue #864 — diagnose firefox's wasm initialization floor

**Status:** draft, awaiting approval  
**Issue:** [#864](https://github.com/jaunder-org/jaunder/issues/864)  
**Branch:** `issue-864-firefox-wasm-instantiate-floor`  
**Predecessors:** #818 (browser boot decomposition) → #840 (withdrew an untested
streaming claim) → #836 (size contrast found the floor) → #866 (preload trial;
residual reattribution caveat) → #887 (direct wasm initialization measurement)

## Outcome

Diagnose the ~377 ms size-independent firefox WebAssembly initialization floor
with a dedicated, certified measurement cycle. The result is a documented
finding that either attributes the floor to tested factors or records which
plausible factors were eliminated without inventing a mechanism.

## Load-bearing decisions

- The decisive object is firefox's current direct wasm initialization evidence,
  not the historical `wasm_instantiate` / `wasmInstantiateMs` residual.
  Historical #836 values remain evidence for the existence and approximate size
  of the floor, but #887's `direct-init-v1` fields are the current instrument.
- The measurement stays within ADR-0100's document-frame rule. Exclusive boot
  decomposition uses
  `wasmInitStartMs + wasmInitStartToBootEntryMs + bootPhases = bootTotalMs`;
  overlapping diagnostics such as `wasmApiMs`, `wasmInitMs`, `wasmInitPath`, and
  resource timing are read as diagnostics, not summed into the decomposition.
- The experiment must hold delivery mode constant unless the specific question
  is delivery. #866 proved that moving the fetch start makes the historical
  residual incomparable across arms; this cycle must not repeat that attribution
  error.
- A successful finding requires arm integrity inside the captured trace. The
  trace must carry an independent discriminator for the varied factor,
  comparable to #836's `wasmDecodedBytes` arm check or #866's `initiatorType`
  arm check.
- The first experiment must target at least two size-independent candidates from
  #864's issue body: a non-byte module-shape factor (for example imports,
  exports, functions, table/memory/shim count) and a per-navigation or
  per-module engine setup factor. The design may add another candidate only if
  it remains separable under the same corpus.
- Arm order is randomized or explicitly counterbalanced per run, and the
  realized order is recorded in the corpus manifest or trace. An unrandomized
  capture is non-decisive unless its positional confound and interpretation rule
  were pre-registered.
- Decisive timing captures run on a quiescent machine. Non-quiescent runs may
  validate tooling or explore shape, but they cannot close #864 with an
  attribution claim unless the load condition and interpretation rule were
  pre-registered.
- Predictions and abort/interpretation rules are written before capture. A
  timing shape may suggest a mechanism, but the write-up may claim only
  mechanisms the arm contrast actually isolates.
- Firefox is the critical path engine. Chromium may be captured as a control,
  but a chromium-only explanation does not close this issue.
- The final record lives in `docs/observability.md` with corpus path,
  certification, arm-integrity checks, tested candidates, and the conclusion. If
  a durable measurement invariant is discovered, record it in an ADR draft;
  otherwise no ADR is required.

## Acceptance

- A pre-registered measurement note names each arm, the varied factor, the
  expected direction/magnitude if the candidate explains the floor, and the rule
  for reading a null or conflicting result.
- The corpus is certified before analysis: every included navigation is counted,
  current `direct-init-v1` coverage is reported, dropped/truncated populations
  are reported, and closure checks pass or the defect is fixed before
  conclusions are drawn.
- Arm integrity is verified from trace data independent of the metric under
  test; a corpus without this check is not used for the finding.
- The realized arm order is recorded and reconciled against the pre-registered
  randomization or counterbalancing rule; a corpus that leaves arm position
  confounded with the varied factor is not used for the finding.
- The capture records the host quiescence condition. A non-quiescent corpus is
  marked exploratory unless its host-load confound was pre-registered with a
  concrete interpretation rule.
- Results report means over run means, uncertainty, cold/warm handling, and the
  firefox result first. Chromium appears as a control, not the deciding arm.
- `docs/observability.md` is updated to replace the open #864 question with the
  measured result or with a negative-result ledger of eliminated hypotheses and
  the remaining unknown.
- Current prose does not describe the historical `wasm_instantiate` residual as
  a direct compile or initialization measurement, and does not revive the
  withdrawn #866 delivery-dependence claim.
- Verification includes the focused analyzer or e2e commands that prove the
  current timing schema is still captured and decomposed correctly, plus the
  smallest project gate appropriate to any code changed to run the measurement.

## Boundaries

- Do not optimize the wasm bundle, change the SPA shell delivery path, re-add
  wasm preload, or change e2e navigation count as part of this issue.
- Do not broaden into #870's render-blocking stylesheet question or #868's frame
  skew attribution, except to cite them as separate candidates outside this
  cycle.
- Do not re-derive or rewrite historical #836/#866 numbers. Label their residual
  field honestly and use new captures for current claims.
- Do not land a generic observability framework. Add only the instrumentation or
  analysis needed to separate the pre-registered candidates.
