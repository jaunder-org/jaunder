# E2E duration-budget gate implementation outline

> Execute with jaunder-iterate and jaunder-dispatch. This outline exists because
> the Playwright-to-Nix-to-xtask discovery manifest is a cross-runtime artifact
> protocol whose identity and completeness invariants must remain aligned.

## Scope

In:

- Per-combo duration-pressure enforcement and its captured diagnostics.
- A Playwright-resolved discovery manifest and authoritative effective-timeout
  metadata reconciled with the JSON report.
- Strict xtask validation, host tests, focused E2E proof, and affected guidance.

Out:

- Budget right-sizing, reverse/headroom checks, aggregate CI history, or changes
  to retries, workers, timeout values, and trace timing semantics.

## Task outline

- [x] Task 1: Emit a complete per-combo test population and effective-budget
      artifact from the E2E runtime.
  - Contract: publish `test-results/duration-budget-manifest.json` atomically
    only at reporter completion. Its JSON shape is
    `{ "schema_version": 1, "complete": true, "tests": [...] }`. Each unique
    `tests` record contains `test_id`, `project_id`, `project_name`, `title`,
    `file`, `line`, and `attempts`; every `attempts` record contains `retry` and
    positive `effective_timeout_ms`. Discovery owns the selected identity set;
    the automatic fixture records the final `TestInfo.timeout` against its
    identity and retry after modifiers and explicit in-test budget changes.
    Publication requires one record for every discovered identity and exactly
    one budget record for each observed retry.
  - Verification: focused Playwright/TypeScript tests cover selected dependency
    projects, a normal attempt, `test.slow()`, an explicit budget, and retry
    identity; a targeted VM combo preserves both artifacts.

- [x] Task 2: Preserve the report and manifest through each Nix E2E combo before
      asserting the test result.
  - Contract: copy `test-results/results.json` to
    `capture/playwright-report-<backend>.json` and
    `test-results/duration-budget-manifest.json` to
    `capture/duration-budget-manifest-<backend>.json`; lift those exact
    basenames into `.xtask/diagnostics/e2e-<backend>-<browser>/`. Absent or
    partial artifacts remain observable as gate input errors. Failed-combo
    diagnostics retain the existing failure order and are not replaced by
    duration validation.
  - Verification: a focused E2E/VM-path test or existing equivalent harness
    proves capture precedes assertion and exposes both deterministic paths.

- [x] Task 3: Validate report, manifest, retries, and utilization after every
      successful combo in both the CI-combo and aggregate-validate routes.
  - Contract: exact one-to-one selected-test reconciliation; each selected test
    has nonempty results; retries start at zero and are contiguous; every
    attempt has a finite duration and matching effective timeout. Fail at
    utilization >= 0.80, report the maximum offending attempt with its full
    diagnostic identity, and fail closed for any unavailable or inconsistent
    successful-combo input. The aggregate route must explicitly realize, lift,
    and validate each named backend/browser output rather than rely on the
    collision-prone aggregate symlink join.
  - Verification: explicit host xtask unit tests cover normal, threshold,
    over-threshold, all timeout modes, retry masking, empty/malformed/partial
    reports, missing manifests, identity mismatches, and invalid retry streams.
    A route test proves the aggregate and `cargo xtask e2e <backend> <browser>`
    paths dispatch the same validator. Run `devtool run -- cargo xtask check`
    after the focused proof.

- [x] Task 4: Document the gate as a pressure detector, not a sizing policy.
  - Contract: contributor and observability guidance names the copied report and
    manifest as the per-combo source, explains the 80% semantics and retry
    handling, and retains deadline-derived timeout sizing as the authoring rule.
  - Verification: documentation links/commands pass the normal check gate; a
    focused E2E combo demonstrates failure only after diagnostics are captured.

## Risk checks

- The manifest must derive discovery from Playwright rather than duplicate
  project dependencies, tag filters, or source-file matching in Rust/Nix.
- Manifest and JSON reconciliation must reject a valid-looking but pruned
  report; report self-consistency alone is insufficient.
- Runtime timeout metadata must observe the final effective value, so
  `test.slow()` and `setTestBudget()` cannot be misclassified from source text.
- Preserve ADR-0037 diagnostic-before-failure ordering and ADR-0141's explicit
  host xtask test coverage boundary.
