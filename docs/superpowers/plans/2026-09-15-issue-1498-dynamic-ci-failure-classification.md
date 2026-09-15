# Dynamic CI failure classification implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for a bounded task
> when useful. This outline exists because the observer gains a durable GitHub
> evidence boundary and workflow-graph contract.

## Scope

In:

- Classify current-head checks as directly required, transitively required, or
  optional from ruleset, Actions, and immutable workflow evidence.
- Return early for required failures and report optional failures without
  blocking readiness.
- Preserve rerun settledness, observer-only capabilities, exhaustive CI jobs,
  and existing merge/queue verdicts.
- Ship the proposed decision record, architecture projection, and operator docs.

Out:

- Branch-protection, aggregate-job, CI-lane, matrix, and merge-queue changes.
- Workflow cancellation, rerun, enqueue, rebase, or merge capabilities.
- A maintained constituent-job allowlist or permissive fallback for evidence the
  classifier cannot resolve.

## Task outline

- [x] Task 1: Build a pure workflow graph and requirement classifier
  - Contract: Parse an immutable workflow definition into job keys, matrix job
    instances, local reusable-workflow nodes, and `needs` edges; join each
    runtime job identity to exactly one graph node; classify graph ancestry from
    directly required contexts as direct, transitive, or optional.
  - Contract: Opaque remote reusable workflows, unsupported expressions, missing
    source, and zero-or-many runtime joins return typed classification errors
    rather than optional classifications.
  - Verification: Captured workflow/runtime fixtures cover arbitrary renames, a
    newly inserted ancestor, matrix expansion, pinned local reuse, optional
    siblings, unsupported remote reuse, and ambiguous/missing joins through the
    production parser and graph builder.

- [x] Task 2: Add current-head GitHub evidence acquisition
  - Contract: Enrich check observations with stable provider/check-run identity;
    resolve a GitHub Actions check to its job, run ID, run attempt, workflow
    path, workflow commit, complete paginated job population, and workflow
    source. Non-Actions status contexts remain direct only by exact ruleset
    membership.
  - Contract: Cache immutable graph evidence by workflow commit/path and bind
    runtime classification to head SHA, run ID, and attempt so a push or rerun
    cannot reuse stale ancestry.
  - Contract: Every new operation remains on the read-only `PrSource`/`gh`
    boundary; any transport, decode, pagination, source, or correlation failure
    enters the existing poll-error and strike-budget path.
  - Verification: Boundary fixtures cover multiple workflow runs on one head,
    pagination, run attempts, non-Actions contexts, superseded heads, malformed
    responses, and unavailable workflow source; transport tests prove no
    mutation endpoint or subprocess invocation is introduced.

- [x] Task 3: Integrate classified failures into decision and watch behavior
  - Contract: Conflict remains higher precedence; directly or transitively
    required failure returns `checks-failed` with job name and log URL before
    aggregate settlement. Optional failure emits one explicitly optional event
    with the same evidence and observation continues.
  - Contract: Existing resolution semantics apply per logical runtime job: an
    in-flight rerun outranks an older completion and the latest settled attempt
    wins otherwise. Directly required aggregate success remains the sole
    positive readiness proof.
  - Verification: Pure decision and virtual-clock loop tests cover early lane
    and matrix failures, optional failure followed by readiness, pending and
    successful reruns, head replacement, classification poll failures, `--once`,
    default watch, `--until merged`, and the post-`pr land` watch.

- [ ] Task 4: Lock CI shape and document the observer contract
  - Contract: The production workflow parser verifies the real CI workflow keeps
    the e2e matrix at `strategy.fail-fast: false`; observation continues to own
    no workflow mutation capability.
  - Contract: Update `CONTRIBUTING.md` and the architecture projection; retain
    the proposed ADR at `docs/adr/drafts/dynamic-pr-check-classification.md` for
    serialized post-merge promotion.
  - Verification: The workflow-shape test reads `.github/workflows/ci.yml` and
    fails if e2e becomes fail-fast; focused xtask tests pass with
    `devtool run -- cargo test --manifest-path xtask/Cargo.toml pr::`; the final
    repository gate is selected by `jaunder-iterate`/`jaunder-ship`.

## Risk checks

- The GitHub rollup and Actions job endpoints must be exhaustively paginated;
  truncation cannot classify an unseen job as optional.
- Display names are presentation, not authority. Matrix interpolation or
  reusable-workflow naming that cannot produce a unique graph join fails closed.
- Workflow source comes from the exact run commit/path, not the mutable working
  tree or default branch.
- A head or run-attempt change invalidates every cached runtime classification.
- Optional failure events participate in change detection and do not repeat on
  every poll.
- Required aggregate readiness, merge-group ejection detection, queue-history
  reset, and conflict precedence retain their existing tests.
- `PrSource` remains read-only and `PrArmer` remains the sole merge mutation
  capability under ADR-0087.
- `CONTEXT.md` needs no change: this is CI observation policy, not Jaunder
  domain language.
