# Fail-Closed Rust Coverage Evidence

Issue: #1411

## Outcome

`Validate (no e2e)` is again the non-bypassable CI authority for the complete
root-workspace Rust test population. A producer that cannot resolve, build, run,
or report that population fails visibly; no empty or partial artifact can be
reported as successful coverage.

## Load-bearing decisions

- The Nix coverage check remains the single authoritative execution of the
  root-workspace Rust tests. This issue does not permanently duplicate that
  population in a second CI job or make local Git hooks authoritative.
- The coverage source closure is Cargo-complete. It contains every manifest and
  source path needed to resolve and build the selected root-workspace targets,
  including transitive path build dependencies outside the root workspace.
  Unrelated auxiliary sources remain excluded from instrumentation and cache
  invalidation.
- Source-closure completeness is derived from the build dependency boundary, not
  maintained as an unexplained one-off exception. A future required path
  dependency must either enter the closure or make the gate fail closed.
- The instrumented nextest invocation and an independent census invocation both
  select the root workspace explicitly and accept no package, test, partition,
  or expression filter. The expected population comes from the unfiltered
  machine-readable census; execution evidence comes from the subsequent run.
  Tests that nextest itself classifies as statically ignored are represented as
  ignored in both sets and are not silently treated as executed. Every other
  expected test must produce exactly one terminal executed result. Empty,
  missing, duplicate, malformed, or unexpected identities fail the producer.
- Doctests and the auxiliary `xtask` and `tools` workspace tests retain their
  existing independent authorities. They are outside the root nextest population
  and are not folded into its census.
- Required producer stages are workspace resolution, profile cleanup, the
  unfiltered test census, the instrumented test run, population reconciliation,
  text-report generation, LCOV generation, and CRAP-report generation. The
  disk-usage snapshot and copying additional human-readable diagnostics are
  best-effort and cannot change the verdict.
- Every required subprocess has an authoritative exit status. A nonzero status
  can produce retained diagnostics and a controlled failure status, but can
  never become `tests-ok` because its output lacks a recognized failure line.
- The producer status records a stable required-stage identifier, process
  outcome, expected/executed/ignored population counts when known, failed or
  missing test identities when known, and a sanitized diagnostic summary.
  `tests-ok` permits no failed stage or population mismatch and is written only
  after every required stage succeeds.
- Human-readable output parsing may enrich diagnostics but never determines
  primary success. Ordinary test failures remain distinct from infrastructure or
  producer failures when structured evidence can classify them; unknown nonzero
  outcomes fail as producer/infrastructure errors rather than being guessed
  green.
- The host consumer validates the producer-status invariants and independently
  rejects missing, malformed, contradictory, or zero-line Rust coverage
  evidence. This defense is intentionally redundant with producer validation so
  one classifier defect cannot recreate the escape.
- Sensitive command output is retained only in the existing diagnostic artifact,
  not copied into status fields.
- This restores the authority already assigned by ADR-0028, ADR-0029, ADR-0050,
  ADR-0053, and ADR-0110. It introduces no new architectural decision and
  therefore requires no new ADR or domain-glossary term.

## Acceptance

- The exact PR #1401 failure shape—Cargo metadata exits 101 because a required
  path manifest is absent, with no `FAIL [` line—produces a red coverage result
  and never serializes `tests-ok`.
- Removing a required path build dependency from the coverage source closure
  makes a focused, agent-runnable closure check fail before CI can accept the
  derivation. Changing that dependency's build-time source changes the coverage
  derivation identity, while changing unrelated auxiliary source does not.
  Build-time-only source is absent from the instrumented report and coverage
  denominator.
- The coverage test census and nextest run both name the root workspace
  explicitly and accept no narrowing filter. The producer reports nonzero
  expected and executed counts plus the classified ignored count.
- A fixture in which one expected runnable test has no executed terminal result
  fails and names the missing test; duplicate, malformed, unexpectedly filtered,
  and contradictory status identities also fail closed.
- Fixtures cover nonzero outcomes at each required producer stage and prove that
  only a complete, internally consistent status can be `tests-ok`.
- An empty text report and an otherwise-valid report containing zero executable
  lines both fail in the host consumer.
- `cargo xtask validate --no-e2e` runs the repaired coverage producer and host
  consumer, so the existing required `Validate (no e2e)` context turns red for
  every failure above without depending on pre-push execution.
- The repository's focused regression tests cover subprocess status handling,
  population reconciliation, empty-evidence rejection, and source-closure
  completeness.
- Testing documentation states the explicit workspace, population, exit-status,
  nonempty-report, and backend-parity evidence contract.

## Boundaries

- This issue does not change application behavior or the SQLite/PostgreSQL test
  semantics.
- It does not redesign the e2e, wasm, Elisp, doctest, mutation, or
  server-function coverage gates.
- It does not add retry policy, suppress failures, or classify arbitrary command
  text as success.
- It does not add a permanent duplicate root-test CI lane; a temporary emergency
  lane, if operationally necessary before this fix lands, is outside the shipped
  design.
- It does not broaden the coverage denominator to auxiliary tooling code merely
  because a build-time tool must exist in the source closure.
