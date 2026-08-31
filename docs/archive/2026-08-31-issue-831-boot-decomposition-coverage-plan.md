# Issue #831 boot-decomposition coverage implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independently
> owned tasks. This outline exists because the Playwright report becomes the
> durable population authority for a cross-artifact gate.

## Scope

In:

- A strict host-side evaluator over each combo's lifted Playwright report and
  trace capture.
- Per-project current-schema, dropped-record, document-frame closure, and exact
  project-set enforcement.
- Successful-combo orchestration, regression coverage, the proposed ADR, and its
  documentation projection.

Out:

- Browser capture, timing marks, Playwright configuration, E2E failure ordering,
  and unrelated gates.
- New CLI surface or configurable thresholds.

## Task outline

- [x] Task 1: Add the reusable boot-decomposition evidence evaluator.
  - Contract: the Playwright report yields the executed project-name set;
    existing trace parsing yields per-project navigation evidence from both
    `e2e.test` and `e2e.page` spans using the current `commitToMountMs` mounted
    proxy; evaluation requires exact set equality, a non-empty mounted
    population, `direct-init-v1`, complete phases, 1 ms document-frame closure,
    and zero dropped records.
  - Verification: focused xtask unit tests prove the passing corpus plus
    missing/empty/malformed inputs, project-set differences, zero mounted
    evidence, legacy/partial/non-closing navigation, dropped records, and an
    incomplete `e2e.page` navigation.
- [x] Task 2: Gate every successful lifted E2E combination and publish the
      policy.
  - Contract:
    `boot_decomposition_coverage::validate_lifted_combo(backend, browser) -> StepResult`
    runs after unconditional artifact lift and only after VM success, alongside
    the existing duration-pressure validator; failures identify backend,
    browser, project, and evidence class without masking the primary Playwright
    failure. The existing Playwright report and `capture-<backend>.tar.gz` paths
    remain authoritative.
  - Verification: aggregate and single-combo orchestration tests prove all four
    combo paths, success-only boot-gate invocation, and retention of the
    duration step; one SQLite/Chromium E2E smoke run exercises the real artifact
    path; CONTRIBUTING and observability docs match the spec, proposed ADR, and
    architecture projection.

## Risk checks

- Preserve ADR-0037 ordering: diagnostics lift before Playwright status
  propagation; no trace gate on a failed combo.
- Preserve ADR-0100 frames: closure uses only document-frame segments and never
  `commitToMountMs` as a decomposition segment.
- Preserve analyzer compatibility: `MIN_BOOT_PHASES` remains a floor so future
  marks do not create a blackout.
- Parse population independently from the trace evidence, using the Playwright
  report; never let trace rows define their own expected project set.
- Keep backend/browser combinations isolated; never pool ratios or artifacts
  across derivations.
- Consider `CONTEXT.md` and leave it unchanged because this decision adds no
  domain vocabulary.
