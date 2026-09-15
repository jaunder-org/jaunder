# Dynamic CI failure classification for PR observation

## Outcome

`cargo xtask pr watch` returns control as soon as a failed current-head check is
known to contribute to a required aggregate, without waiting for that aggregate
to settle. It also reports genuinely optional failures clearly without treating
them as merge-blocking.

## Load-bearing decisions

- The branch ruleset remains the authority for the directly required check
  contexts and for readiness: every configured required context must appear and
  succeed before a PR is ready to land.
- A check is **transitively required** when the exact workflow run's job
  dependency graph places it upstream of a directly required context.
- GitHub Actions check identity is joined through the current-head check-run ID
  to its Actions job, run ID, run attempt, workflow path, and workflow commit.
  The immutable workflow definition at that commit supplies job keys, matrix
  expansion, and `needs` edges. A runtime job or required context must correlate
  to exactly one expanded graph node before its ancestry is used.
- Local reusable workflows are followed only when their pinned definitions and
  runtime identities can be resolved unambiguously. Opaque remote reusable
  workflows, unsupported expressions, missing workflow source, and ambiguous
  matrix or display-name joins are classification failures, not optional jobs.
- A non-Actions status context has no Actions dependency ancestry: exact ruleset
  membership makes it directly required; otherwise it is optional.
- Required, transitively required, and optional checks are classified from
  current GitHub and workflow evidence. No check name, job name, matrix value,
  workflow name, backend, or browser is a recognition allowlist.
- A failed directly or transitively required check on the current PR head is
  immediately caller-actionable. Observation returns `checks-failed`, names the
  failed job in its detail, and points at that job's log.
- A failed optional check on the current head is emitted once as an explicitly
  optional failure with its log URL. It remains non-terminal and cannot satisfy
  or prevent readiness.
- Failure classification follows the workflow definition and run that produced
  the observed checks, rather than assuming every job in a particular workflow
  is required.
- If GitHub or workflow evidence cannot establish a check's classification, the
  observer treats that as an observation failure under its existing poll-error
  and strike-budget policy; uncertainty is never silently converted to optional.
- Existing settledness remains authoritative per check: a pending rerun outranks
  the superseded completed failure, and the latest settled attempt wins when no
  rerun is pending.
- Only evidence for the current PR head can affect its verdict. A failure from a
  superseded head cannot terminate current observation.
- Existing outcome precedence remains: a merge conflict outranks a check
  failure; otherwise a required or transitively required failure outranks
  pending aggregates and readiness.
- The policy applies to default watch, `--once`, `--until merged`, and the watch
  used after `pr land`.
- Observation never cancels workflow jobs. The CI workflow retains exhaustive
  diagnostics, including the e2e matrix's `strategy.fail-fast: false` behavior.
- This policy is recorded in
  `docs/adr/0195-dynamic-pr-check-classification.md`.

## Acceptance

- A failed validation lane ends observation with `checks-failed` while its
  required aggregate remains pending.
- A failed e2e matrix job ends observation with `checks-failed` while sibling
  matrix jobs continue.
- Renaming an upstream job or adding a new upstream job requires no xtask source
  change for its failure to remain caller-actionable.
- A failed check outside every required context's dependency ancestry produces
  one optional-failure event with its name and log URL, while observation
  continues.
- Readiness still requires all directly required ruleset contexts to succeed;
  transitive successes never substitute for aggregate success.
- A superseded-head failure has no effect on the current head.
- A pending rerun suppresses the earlier failure until the rerun settles.
- Captured GitHub and workflow fixtures exercise the production parser and graph
  builder through classification. They cover arbitrary job renames, a new
  ancestor, matrix instances, direct required-context correlation, run attempts,
  local reusable workflow resolution, and missing or ambiguous joins.
- Decision and watch-loop tests cover early transitive failure, optional
  failure, unavailable classification evidence, superseded heads, reruns, and
  all watch modes.
- A workflow-shape test proves that e2e retains `strategy.fail-fast: false`.
- Capability and transport tests prove every watch mode can return early without
  issuing cancellation or any other workflow mutation.

## Boundaries

- This work does not change branch protection, the required aggregate contexts,
  CI lane contents, matrix dimensions, merge-queue behavior, or merge approval.
- It does not rerun, cancel, rebase, enqueue, or merge anything.
- It does not make optional checks merge-blocking.
- It does not introduce a maintained list of constituent job names.
