# ADR-0048: ADRs drafted out of git, numbered at ship

- Status: proposed
- Date: 2026-07-03
- Issue: [#219](https://github.com/jaunder-org/jaunder/issues/219)

## Context

ADR numbers are a shared monotonic sequence. Two branches authoring an ADR
concurrently both want "the next number", and the only moment the correct number
is knowable is at integration.

The prior flow (ADR-0036, #196) handled this by authoring every ADR as the
`0000` sentinel and running `cargo xtask adr renumber` to reconcile — but
`renumber` was run at _authoring_ time and its result committed, so the number
was assigned before it was final. When a rebase later revealed a collision, the
recovery (`renumber` again) landed as a _new_ commit on top of the one that
introduced the ADR. The result was guaranteed number churn in git history (e.g.
"renumber ADR 0045 -> 0046 after rebase onto main"). Two independent faults
compounded: the number was assigned too early, and its correction was a fixup
commit rather than an amend.

ADR-0036's other contributions — the `identifier-collisions` gate that makes a
concurrent-branch number clash _loud_ (two differently-named files merge without
a git conflict), and `renumber` itself — remain sound and are retained. This ADR
revises only 0036's _authoring_ half (the always-`0000` sentinel).

## Decision

New ADRs live **out of git** until ship, and are numbered at the last possible
moment:

- Authored numberless as `docs/adr/drafts/<slug>.md`; the `docs/adr/drafts/`
  directory is gitignored (except its `README.md`), so a number cannot be
  committed early. Drafts carry a `# ADR-0048: <Title>` heading and are
  referenced by their `docs/adr/drafts/<slug>.md` path during development.
- `cargo xtask adr promote`, run in `jaunder-ship` after the final rebase,
  assigns each draft the next free number, moves it to
  `docs/adr/NNNN-<slug>.md`, rewrites its path-form references, syncs the README
  table, and stages everything. The ADR's first appearance in history is already
  correctly numbered.
- If a collision still surfaces between the ship commit and the merge,
  re-rebase, re-run the assignment, and **amend the commit that introduced the
  ADR** — never add a fixup commit. `cargo xtask adr renumber` remains the tool
  for that already-committed-ADR case.

The three ADR gates (`identifier-collisions`, `adr-format`, `adr-readme-parity`)
require no change: their shared enumeration rule (`is_file` → `.md` → leading
number, non-recursive over `docs/adr/`) already excludes a numberless draft in a
subdirectory.

## Consequences

- No ADR number churn in normal history: the number is written to git exactly
  once, at ship.
- Forgetting to `promote` yields an ADR _absent_ from the PR (caught at review,
  since the PR references a `drafts/` path that isn't there), not a _wrong_
  number baked into history — a safer failure mode. `jaunder-ship` runs
  `promote` unconditionally to keep the common path clean.
- The "did you take someone's number?" guard for the residual race moves from a
  CI gate (which cannot see untracked drafts) to a documented amend discipline
  in `jaunder-ship`.
- Drafts are invisible in a fresh clone; a tracked `docs/adr/drafts/README.md`
  and the `CONTRIBUTING.md` / `jaunder-adr` docs carry the flow.
- ADR-0036's always-`0000` authoring flow is retired; its collision-detection
  gate and `renumber` command are unchanged.
