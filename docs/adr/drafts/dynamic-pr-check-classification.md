# ADR-DRAFT: Classify PR check failures from the workflow dependency graph

- Status: proposed
- Date: 2026-09-15
- Issue: [#1498](https://github.com/jaunder-org/jaunder/issues/1498)

## Context

[ADR-0087](../0087-xtask-github-pr-observation.md) makes the branch ruleset the
per-run authority for required contexts and keeps optional checks outside the
merge verdict. CI now exposes substantive failures in validation lanes and e2e
matrix jobs before their required result-only aggregates can settle. Waiting for
the aggregate delays the fix loop without adding evidence.

Those early jobs are not directly required by branch protection. Hard-coding
their current display names would recover today's failures but silently miss a
renamed or newly split upstream job. Treating every failure as terminal would
instead make genuinely optional workflows merge-blocking. The observer needs a
stable distinction among directly required, transitively required, and optional
checks without maintaining a second list of the CI topology.

## Decision

The branch ruleset remains authoritative for directly required contexts and
readiness. The observer additionally derives transitive requirement from the
exact workflow run's job dependency graph: a job is transitively required when
it is upstream of a directly required context in that run.

A failed directly or transitively required current-head check returns
`checks-failed` immediately, naming the job and pointing to its log. A failed
optional current-head check is emitted as an explicitly optional failure with
its log URL, but observation continues and readiness is unchanged.

Classification is derived from current GitHub and workflow evidence. For a
GitHub Actions check, its current-head check-run ID resolves the Actions job,
run ID, run attempt, workflow path, and workflow commit. The immutable workflow
definition at that commit supplies job keys, matrix expansion, and `needs`
edges; runtime jobs and required contexts must each correlate to exactly one
expanded graph node before ancestry is used. Unambiguous pinned local reusable
workflows may be followed. Opaque remote reusable workflows, unsupported
expressions, missing source, and ambiguous matrix or display-name joins are
observation failures under the existing poll-error policy, never implicit
optional classifications. A non-Actions status context is directly required by
exact ruleset membership or optional because it has no Actions dependency
ancestry.

The observer does not hard-code check names, job names, matrix values, workflow
names, backends, or browsers, and it does not assume every job in one workflow
contributes to a required aggregate.

The existing current-head and settledness rules remain: superseded-head checks
cannot affect the verdict, a pending rerun outranks its earlier failure, and the
latest settled attempt wins otherwise. Observation does not cancel, rerun, or
otherwise mutate workflow jobs.

## Consequences

- New or renamed upstream jobs become caller-actionable without an xtask source
  change, while optional checks remain non-blocking.
- Required aggregate success remains the only positive readiness proof;
  successful constituents cannot substitute for it.
- The observer must read Actions job/run metadata and immutable workflow source
  to establish exact dependency ancestry, adding a fallible observation boundary
  that must use the existing strike and diagnostic policy.
- Unsupported future Actions graph features fail closed until the classifier
  learns them; they cannot silently weaken a required failure into an optional
  warning.
- Optional failures become visible in the event stream instead of disappearing,
  so a ready result can coexist with recorded non-blocking failures.
- CI retains exhaustive diagnostics and `strategy.fail-fast: false`; faster
  observer feedback does not shorten the workflow itself.
- The observer still owns no CI mutation capability, preserving the
  observer/armer separation in ADR-0087.
