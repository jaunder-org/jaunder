# Issue #831: Per-combo boot-decomposition coverage gate

## Outcome

Every SQLite/PostgreSQL × Chromium/Firefox E2E combination fails when its lifted
trace evidence does not prove complete CSR boot decomposition for every executed
Playwright project. The gate turns the existing informational boot-coverage
report into a fail-closed regression boundary without changing capture
production.

## Load-bearing decisions

- Each backend×browser combination lifts diagnostics regardless of outcome; a
  failed combination retains those artifacts and skips this gate, while a
  successful combination is judged from its lifted Playwright report and trace
  capture.
- The Playwright report structurally defines the executed project population;
  the trace project set must match it exactly.
- Every executed project must contribute at least one mounted navigation. An
  empty project population is an evidence failure, not vacuous success.
- Mounted membership retains the analyzer's existing non-null `commitToMountMs`
  proxy; the gate does not infer mounts that lack an observed commit.
- The coverage floor is 100% for mounted navigations in every project; ratios
  are never pooled across projects or combinations.
- The 100% floor follows the #818 post-fix corpus: 12 captures covered every
  mounted navigation, with zero dropped entries and zero closure violations
  across 2,496 navigations.
- A covered navigation uses the current `direct-init-v1` timing schema, contains
  the complete document-frame boot decomposition, and closes to `bootTotalMs`
  within the existing 1 ms tolerance.
- Legacy, absent, partial, or non-closing decomposition is uncovered evidence.
- Any nonzero dropped-navigation count fails because duration-biased truncation
  makes the true coverage unknowable.
- Missing, empty, malformed, or project-mismatched report/capture evidence fails
  closed.
- Gate analysis remains host-owned over artifacts produced and lifted by the
  existing Nix E2E derivation, as governed by ADR-0028.
- The Playwright report's ownership of the executed project population is
  recorded in
  `docs/adr/drafts/playwright-report-defines-trace-gate-population.md`.
- The gate reads copied artifacts after Playwright exits and the VM attempts to
  flush the trace collector; it does not require a quiescent CI host or sample
  live host performance. The 1 ms tolerance checks arithmetic closure within one
  document clock.

## Acceptance

- Each of the four authoritative E2E combinations runs the boot-decomposition
  coverage gate against its own lifted artifacts.
- Complete current-schema evidence for every executed project passes at 100%
  coverage.
- A project-wide blackout, one incomplete navigation, one closure violation, one
  dropped navigation, or a zero-navigation project fails with the affected
  combination and project identified.
- Missing, empty, unparseable, or project-mismatched Playwright/trace artifacts
  fail with an evidence-integrity diagnostic.
- A failed E2E combination preserves its lifted diagnostics and primary failure
  without running the boot-evidence gate.
- Regression tests cover the passing corpus and every fail-closed class above,
  including exact project-set reconciliation.
- Contributor and observability documentation describe the enforced
  per-project/per-combo policy and distinguish it from the Page boot budget,
  source coverage, and server-function flow coverage.

## Boundaries

- Do not change browser capture, timing marks, trace attribution, or Playwright
  project configuration.
- Do not pool ratios, introduce a tunable threshold, accept legacy schemas, or
  infer missing evidence.
- Do not change the one-boot-per-Page budget, duration-pressure gate,
  source-coverage gate, or server-function coverage gate.
- No new domain terminology is introduced; ADR-0028, ADR-0034, ADR-0096,
  ADR-0100, ADR-0110, ADR-0111, and
  `docs/adr/drafts/playwright-report-defines-trace-gate-population.md` govern
  the implementation.
