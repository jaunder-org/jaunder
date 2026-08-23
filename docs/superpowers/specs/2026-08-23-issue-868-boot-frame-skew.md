# Issue #868 — attribute the boot frame skew

**Status:** draft, awaiting approval

**Issue:** [#868](https://github.com/jaunder-org/jaunder/issues/868)

**Branch:** `issue-868-boot-frame-skew`

**Predecessors:** #818 fixed boot mark capture and established ADR-0100; #866
reported the remaining boot budget and filed this issue; #864 kept frame skew
separate from wasm initialization diagnostics.

## Outcome

Add a clock-bridge diagnostic for mounted navigations so the existing
`commitToMountMs − bootTotalMs` frame skew is split into observable bridge
pieces without folding any of them into app boot phases.

The delivered write-up uses this pre-registered attribution rule: after the
bridge equation closes, a term is **dominant** only if its absolute mean
per-suite contribution is at least two thirds of absolute mean frame skew and
exceeds the next-largest bridge term by at least `3 × combined SE`. If no term
clears that bar, the result is reported as split attribution with the measured
shares, not forced into a single cause. The possible outcomes are:

- navigation commit before the document's `performance.timeOrigin` dominates;
- mount-ready document time before the Node binding timestamp dominates;
- split attribution across both measured bridge terms;
- a smaller unexplained remainder after the bridge terms are measured; or
- no safe attribution because the bridge terms do not close within the
  registered tolerance.

## Evidence

#866's arm-C corpus measured **208 mounted navigations per run** and reported:

| engine   | `commitToMount` | document-frame boot total |       frame skew |
| -------- | --------------: | ------------------------: | ---------------: |
| chromium |   143.2 s/suite |              81.9 s/suite | **61.3 s/suite** |
| firefox  |   189.5 s/suite |             151.2 s/suite | **38.3 s/suite** |

Per mounted navigation, the skew is **294.9 ms** on chromium and **184.2 ms** on
firefox. It is larger on the faster engine, so the sign points at measurement or
engine bookkeeping, not directly at application boot.

ADR-0100 forbids the old shortcut: `commitToMountMs` is Node-side wall clock,
while `bootTotalMs` and `bootPhases` are document-frame timings. The new work is
therefore a bridge diagnostic. It may explain the difference between frames, but
it must never become another app boot segment.

Current capture already has the needed Node landmarks: navigation request start,
commit, DOMContentLoaded, load, mount binding receipt, request finish, and the
document boot marks. What is missing is a stable document epoch and mount-ready
document timestamp recorded with the navigation summary so the bridge can be
reported and certified instead of inferred.

## Decisions

- **D1 — Record bridge timestamps as diagnostics, not boot phases.** Extend the
  per-navigation summary with document epoch / mount-ready bridge fields only
  when a document timing harvest exists. Keep `bootPhases` document-relative and
  keep `commitToMountMs` Node-relative.
- **D2 — Close the bridge equation before drawing a conclusion.** For a complete
  mounted navigation, report these terms separately:
  - `commitToDocumentStartMs = documentTimeOriginMs − committedMs`;
  - `documentBootTotalMs = mount_done.startTime`;
  - `mountDoneToBindingMs = mountedMs − (documentTimeOriginMs + mount_done.startTime)`;
  - `frameSkewRemainderMs = commitToMountMs − documentBootTotalMs − commitToDocumentStartMs − mountDoneToBindingMs`.
    The first and third terms deliberately cross the browser/Node boundary;
    their only valid use is explaining bridge skew.
- **D3 — Preserve ADR-0100 in names and docs.** Analyzer output must label these
  values as frame-skew / bridge diagnostics and must not mix them into the boot
  decomposition table or any app-performance total.
- **D4 — Certify coverage and closure before interpretation.** The analyzer must
  report how many mounted navigations have the full bridge fields, and must flag
  populations whose bridge remainder does not close. Closure is registered as:
  full bridge coverage for every mounted navigation in the deciding rows,
  absolute mean `frameSkewRemainderMs ≤ 1.0 ms/navigation`, and absolute max
  per-navigation remainder `≤ 2.0 ms`. A failed closure turns the result into an
  instrument finding, not an application or harness claim.
- **D5 — Use a fresh quiescent corpus for the finding.** The historical
  #836/#866 numbers motivate the work, but the deciding attribution comes from a
  new sqlite single-worker corpus with **three valid runs per engine**, a
  distinct `e2eSalt` per run, and alternating engine order (`chromium→firefox`,
  then `firefox→chromium`, then `chromium→firefox`) so run position is not
  confounded with engine. Record the host-quiescence statement before capture;
  chromium remains the control because it carried the larger skew.
- **D6 — Means decide suite impact.** Per-suite skew is a sum over mounted
  navigations, so the finding uses means over run means and multiplies by the
  mounted-navigation count. Medians may be printed only as secondary shape
  checks.

## Acceptance

- **AC1. Capture schema:** each mounted navigation with document timing records
  the document epoch and enough mount-ready timing to compute the two bridge
  terms above. Navigations without complete inputs remain `null`, not guessed.
- **AC2. Analyzer:** `cargo xtask traces boot-phases` reports bridge coverage,
  mean ± SE for `commitToDocumentStartMs`, `mountDoneToBindingMs`, and
  `frameSkewRemainderMs`, grouped by engine/cache warmth and by any existing
  experiment arm.
- **AC3. Certification:** the analyzer refuses or clearly marks a decisive row
  non-decisive when bridge coverage is incomplete or the remainder fails the
  registered tolerance: absolute mean `frameSkewRemainderMs ≤ 1.0 ms/navigation`
  and absolute max per-navigation remainder `≤ 2.0 ms`.
- **AC4. Tests:** fixture tests cover the new fields for complete, missing, and
  malformed document timing; Rust analyzer tests cover coverage accounting,
  closure failure, and the non-decomposed placement of bridge rows.
- **AC5. Corpus:** capture a fresh quiescent sqlite single-worker corpus for
  chromium and firefox: three valid runs per engine, distinct `e2eSalt` per run,
  alternating engine order (`chromium→firefox`, `firefox→chromium`,
  `chromium→firefox`), tarballs preserved under
  `~/measurements/jaunder/issue-868-frame-skew/`, JSONL traces extracted, and
  the corpus analyzed with the updated tool.
- **AC6. Write-up:** update `docs/observability.md` with the certified bridge
  coverage, measured terms, closure status, and conclusion. If the bridge does
  not close, say that explicitly and do not attribute the 38–61 s skew.
- **AC7. Verification:** run the focused fixture/analyzer tests and
  `cargo xtask check` before committing the implementation and the finding.

## Boundaries

- Do not optimize application boot or e2e runtime in this issue.
- Do not change production wasm delivery, preload, cache policy, or bundle
  shape.
- Do not reinterpret historical app boot phases by subtracting Node-frame
  values.
- Do not close #870, #895, #1103, or #1138.
