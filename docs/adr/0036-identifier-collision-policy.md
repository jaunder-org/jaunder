# ADR-0036: Identifier-collision policy for ADRs and migrations

- Status: accepted
- Date: 2026-06-28

## Context and Problem Statement

Sequentially-numbered files created on concurrent branches collide: two branches
each pick the next number, and because the filenames differ (`0099-foo.md` vs
`0099-bar.md`) git merges them with no conflict — the collision is silent and
only surfaces as confusion later. ADRs (`docs/adr/NNNN-slug.md`) hit this often;
migrations (`storage/migrations/{sqlite,postgres}/NNNN_slug.sql`) have the same
shape but have not yet collided, and their number is referenced nowhere outside
the directory.

## Decision Drivers

- Make a collision loud rather than silent.
- Make ADR resolution cheap — ADR numbers are referenced in code, `clippy.toml`,
  and docs, so the sequence has value and is worth preserving.
- Proportionality: do not add machinery migrations do not need.

## Decision Outcome

The governing rule: **a branch must never allocate a shared identifier by
reading the current maximum and hoping it survives the merge.**

- A build-free `identifier-collisions` check runs inside
  `cargo xtask check`/`validate`. It fails on a duplicate numeric prefix within
  `docs/adr`, `storage/migrations/sqlite`, or `storage/migrations/postgres`, and
  on sqlite/postgres backend-parity gaps. This makes every collision loud on the
  branch (after rebase) and on `main`'s CI.
- `cargo xtask adr renumber` resolves an ADR collision in one command: the ADR
  already reachable from `origin/main` is immutable; the branch's newly-added
  ADR is bumped to the next free number, with path-form references rewritten
  repo-wide and bare `ADR-NNNN` references rewritten in branch-touched files.
- Migrations keep sequential numbering with the detection check only — no
  renumber tool, no timestamps. Timestamps were rejected: they are
  collision-free but not monotonic with respect to merge order, and a
  later-merged migration with an earlier timestamp can trip sqlx's out-of-order
  detection on a persistent DB.

## Consequences

- Good: collisions cannot ship silently; ADR collisions are a one-command fix.
- Good: no change to the established sequential naming convention.
- Bad: the `adr renumber` heuristic cannot disambiguate a bare `ADR-NNNN` that a
  branch adds into a pre-existing file already citing the other number; that
  rare case is left to the human, and the detection check still guards
  correctness.

## Addendum (#196): the README ADR table is a generated projection

The ADR index table in `docs/README.md` was hand-maintained — every ADR added a
row by hand, and nothing checked that the table agreed with `docs/adr/`. Worse,
`adr renumber` rewrites path-form and bare `ADR-NNNN` references but not the
table's visible bare `[NNNN]` number cell, so a renumber (and the planned
always-`0000` authoring flow, where every new ADR is written `0000-slug.md` and
`renumber` assigns the real number) would leave that cell stale.

The table is now a **generated projection** of `docs/adr/`, extending this
policy:

- **Generated cells, hand-owned titles.** `cargo xtask adr sync-readme`
  regenerates the number, link-target, and status cells of the table (delimited
  by `<!-- adr-table:begin -->` / `<!-- adr-table:end -->` markers) from the ADR
  files. The **title** cell stays hand-curated: a new row is seeded from the ADR
  heading, then never overwritten. `sync-readme` is folded into `adr renumber`,
  so a collision bump refreshes the table in the same run.
- **Canonical ADR style.** Each ADR's line-1 heading is `# ADR-NNNN: <title>`
  (with `NNNN` matching the filename) and its status is a single token from
  `{proposed, accepted, superseded, deprecated, rejected}` on a `- Status:`
  line. Choosing `# ADR-NNNN:` (over the legacy `# NNNN.`) means the heading
  itself carries an `ADR-NNNN` token, so `renumber`'s existing `rewrite_bare`
  fixes a bumped ADR's own heading number for free.
- **Two read-only gates**, siblings of `identifier-collisions`: `adr-format`
  (every ADR matches the canonical heading + status style) and
  `adr-readme-parity` (the table's mechanical cells and row set match
  `docs/adr/`, compared semantically so prettier's column padding and curated
  titles never trip it). Recovery for parity drift is
  `cargo xtask adr sync-readme`; format is a guided manual fix.
