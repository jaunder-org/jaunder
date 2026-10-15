# Issue #1779: Complete or explicitly incomplete candidate inventories

## Outcome

`cargo xtask issue candidates` must never present a truncated inventory as
exhausted or actionable work. Successful discovery reports the complete gathered
open-issue inventory for one exactly resolved open milestone, with trustworthy
claim and prerequisite classification; incomplete discovery fails explicitly.

## Load-bearing decisions

- Follow all REST pages for milestone issues and native `blocked_by`
  relationships. Closed prerequisites do not block; an open prerequisite on any
  page does.
- Required GraphQL metadata must be complete before it supports a decision.
  Continue relevant connections or refuse explicitly when continuation is not
  supported; connection limits must not masquerade as missing milestones,
  missing project status, or unknown metadata.
- Prefer existing nonzero command failure over a new partial-success report. Any
  incomplete required input, malformed continuation evidence, or API failure
  prevents a successful candidate report, including failure after earlier pages.
- Preserve exact, unambiguous open-milestone title/number resolution, existing
  claimed/missing-status/blocked classifications, ordering, and local resume
  overlays. Complete means exhaustive traversal of the observed API inventory,
  not an atomic snapshot or a runtime ownership guarantee.
- Keep discovery read-only. Do not claim, dispatch, or modify tracker state.
  Shared metadata safety changes may affect issue creation's preflight reads,
  but must not change its mutation policy or introduce new writes.

## Acceptance

- Deterministic offline tests cover more than 100 milestone issues across pages,
  excluding pull requests, and classify every gathered issue exactly once.
- An open prerequisite beyond the first dependency page prevents candidacy even
  when the first page contains only closed prerequisites.
- Required metadata beyond GraphQL connection limits is gathered or produces an
  explicit incomplete-input failure, never a false absence or successful count;
  cover milestone resolution and Jaunder Backlog project-status lookup.
- Later-page API failure, GraphQL errors, malformed responses, and missing or
  nonadvancing continuation evidence cannot yield successful discovery.
- Tests preserve exact/ambiguous/missing milestone behavior and demonstrate no
  mutation requests during discovery. Counts in human output, JSON lists, and
  step detail agree with the actual gathered candidate/skipped inventory; an
  empty successful result is possible only after exhaustive required reads.
- Document success/failure completeness semantics in maintained command-facing
  documentation. Regression tests and minimal fixtures are committed; run logs
  and review evidence remain outside tracked source deliverables.

## Boundaries

No scheduler, dispatch, new claim protocol, ownership inference, live GitHub
mutation tests, product/storage changes, or historical incident claim. The
current static truncation paths are applicable evidence, not proof of a past
runtime failure. Implementation and test execution require approved scope.
