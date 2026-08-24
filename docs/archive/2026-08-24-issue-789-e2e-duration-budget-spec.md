# Issue #789 — E2E duration-budget gate

## Outcome

Each successful backend/browser E2E combo rejects a test attempt that consumes
80% or more of its effective whole-test timeout. The gate turns emerging timeout
pressure into an actionable failure while the Playwright report and other combo
diagnostics remain available.

## Load-bearing decisions

- The duration source is the copied Playwright JSON report. Browser timing,
  tracing spans, and driver wall-clock measurements are different measurement
  frames and are not substitutes.
- Enforcement runs per backend/browser combo after that combo has captured its
  diagnostics. It does not require an aggregate CI-artifact protocol.
- Every reported attempt is evaluated. A passing retry does not hide a slower
  earlier attempt; the maximum recorded attempt drives the result.
- An attempt is unsafe at duration / effective timeout >= 0.80. The gate covers
  only this insufficient-headroom direction; it does not prescribe a maximum
  permitted budget or right-size intentionally derived budgets.
- The comparator is the actual effective whole-test timeout, not the written
  ambient value. It preserves the existing browser and worker scaling, the
  `test.slow()` multiplier, and explicit deadline-derived test budgets. No
  adjusted-budget test is exempt.
- A successful combo requires a Playwright-resolved discovery manifest beside
  its report. The manifest's selected-test identities must exactly equal the
  report's test identities; every selected test must have nonempty results, and
  every result must identify its project, numeric duration, and contiguous retry
  sequence beginning at zero. No selected test or attempt may be silently
  omitted. A combo that has already failed for its own test/build reason is not
  masked by a secondary duration-report error.
- Failure output identifies the test, project, retry/attempt, observed duration,
  effective timeout, and utilization so a maintainer can choose to reduce work
  or deliberately re-derive the budget.

## Acceptance

- A report attempt below 80% of its resolved effective timeout passes.
- An attempt at exactly 80%, and one above it, make the otherwise-successful
  combo fail after its diagnostics are copied.
- Validation reflects all existing timeout modes: ambient browser/worker
  scaling, `test.slow()`, and explicit deadline-derived budgets.
- A first attempt at or above 80% causes the gate failure even if its retry
  succeeds; a retry is evaluated independently. An attempt below 80% does not
  fail this duration gate solely because its test status failed.
- A missing, malformed, or incomplete report or discovery manifest for an
  otherwise successful combo fails with a diagnostic explaining the unavailable
  input or reconciliation mismatch.
- The new gate logic has host-executed xtask tests, including parser/input and
  manifest-reconciliation failures, retries, and each timeout mode. Targeted E2E
  coverage proves the gate is on the per-combo execution path and preserves
  captured diagnostics.
- Contributor and observability guidance distinguish this pressure detector from
  the deadline-derived method used to size whole-test budgets.

## Boundaries

- Do not add a reverse gate for budgets larger than observed durations.
- Do not change existing timeout values, retry counts, worker allocation, or the
  existing browser/worker scaling policy.
- Do not aggregate reports across CI jobs or add persistent duration history.
- Do not change the timing semantics of `e2e.test` spans or use them as this
  gate's source data.
