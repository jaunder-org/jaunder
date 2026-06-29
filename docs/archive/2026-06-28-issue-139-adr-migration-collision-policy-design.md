# Design: ADR & migration identifier-collision policy

* Status: draft (awaiting review)
* Date: 2026-06-28
* Author: mdorman (with Claude)

## Problem

Concurrent agents/branches each create the "next" sequentially-numbered ADR
(`docs/adr/NNNN-title.md`) by reading the current maximum. Two branches both
pick the same `NNNN`; because the files have *different names* (`0034-foo.md`
vs `0034-bar.md`), git merges them with **no conflict** and the collision is
**silent**. Resolving it after the fact means a manual rename + relink +
rebase, which has become recurring drama.

Migrations (`storage/migrations/{sqlite,postgres}/NNNN_name.sql`) have the
identical structural hazard, but have **not** collided in practice and their
number is referenced nowhere outside the directory (sqlx discovers files by
scanning; no code cites "migration 0008").

## Governing rule

> A branch must never allocate a shared identifier by reading the current
> maximum and hoping it survives the merge.

There are two legal ways to satisfy this rule, and the right one depends on
whether the identifier is *referenced* elsewhere:

| Artifact   | Collides today? | Number referenced elsewhere?        | Fix |
|------------|-----------------|-------------------------------------|-----|
| ADR        | Yes, often      | Yes (code, `clippy.toml`, CONTRIBUTING) — sequential value matters | Detection test **+** `cargo xtask adr renumber` |
| Migration  | No              | No (order-only; sqlx scans the dir) | Detection test only; keep sequential |

### Why not timestamps for migrations

Timestamp versions (`YYYYMMDDHHMMSS_slug.sql`) are collision-free but **not
monotonic with respect to integration order**: a branch authored earlier can
merge later, so authoring-time order ≠ merge order. sqlx applies migrations in
version order and records applied versions, so a later-merged migration with an
*earlier* timestamp can trip sqlx's out-of-order detection on any persistent
DB. Timestamps trade a rare, loud, easily-fixed problem (collision, caught by a
test) for a subtle ordering hazard. Rejected.

### Why a detection test is enough for migrations but not ADRs

A detection test makes a collision **loud**; it does not make resolution
**cheap**. For migrations that is sufficient because the test ~never fires. For
ADRs it fires often, so "loud but still hand-fixed" leaves the drama intact —
hence the additional one-command resolver.

## Component 1 — duplicate-prefix check (ADRs + migrations)

A new static check, run as part of `cargo xtask check` / `cargo xtask validate`
(the build-free static phase, so it is cheap and runs on every branch and on
`main`'s CI).

**Behavior**

* Scan each of these sets independently:
  * `docs/adr/*.md`
  * `storage/migrations/sqlite/*.sql`
  * `storage/migrations/postgres/*.sql`
* Extract the leading integer prefix (`^(\d+)`) from each filename.
* **Fail** if any prefix appears more than once within its set.

**Failure output** (consistent with the existing "gate prints recovery
command" convention):

* the duplicated number,
* the colliding filenames,
* for ADRs, the recovery command: `cargo xtask adr renumber`.

**Backend parity (migrations):** because backend parity is a core invariant,
the migration check additionally asserts that the sqlite and postgres version
sets are identical — same `{version}_{slug}` membership across both directories.
A migration added to one backend but not the other fails the check.

**When it fires**

* On a branch the instant it is rebased onto a merge that took "its" number.
* On `main`'s CI after both colliding changes land (the last line of defense
  against a silent collision reaching `main`).

## Component 2 — `cargo xtask adr renumber` (ADRs only)

Resolves an ADR collision in one command, turning a manual rename+relink+rebase
into a no-argument invocation.

**Invariant: never mutate a published number.** The ADR reachable from
`origin/main` is immutable (other branches/citations may already reference it).
The branch's *newly-added* ADR is the one bumped.

**Algorithm**

1. Compute the branch's added ADRs:
   `git diff --diff-filter=A --name-only $(git merge-base origin/main HEAD)..HEAD -- docs/adr/`.
2. For each added ADR whose numeric prefix duplicates another ADR in the working
   tree, assign the next number = one greater than the maximum number across the
   working tree's ADRs (monotonic — never reuses a gap left by a deleted ADR).
3. `git mv` the file to the new number.
4. Rewrite references to the old number:
   * **Path-form** references (`docs/adr/0034-foo.md`, `(.../0034-foo.md)`) are
     unambiguous — the path carries the slug — so rewrite them wherever they
     appear.
   * **Bare** references (`ADR-0034`) are ambiguous, so rewrite them **only in
     files the branch touched** (`git diff --name-only merge-base..HEAD`). This
     guarantees `main`'s references to the *other* 0034 are never clobbered.

**Assumptions**

* Run on a branch rebased onto latest `origin/main` (the autonomous workflow
  already rebases). The tool uses `git merge-base origin/main HEAD` to scope
  branch-added ADRs; `origin/main` must be fetched.

**Known residual edge (accepted)**

* A branch that adds a *bare* `ADR-0034` reference into a file that already
  existed on `main` and already referenced the *other* 0034 cannot be
  disambiguated by text. This is left to the human; the duplicate-prefix check
  still guarantees no incorrect state ships silently.

## Documentation

* A new ADR (next free number — a number this very policy governs) records the
  governing rule and the per-artifact fix.
* CONTRIBUTING.md gains a short note: how to add an ADR, what the duplicate
  check is, and the `cargo xtask adr renumber` recovery path.

## Testing

* **Duplicate check:** unit tests over a fixture directory set — clean set
  passes; an injected duplicate fails with the expected message (number +
  filenames + recovery command for ADRs); a mismatched sqlite/postgres version
  set fails the parity assertion.
* **`adr renumber`:** integration test over a throwaway git repo fixture —
  construct an `origin/main` with `0034-foo.md`, a branch adding `0034-bar.md`
  plus a path-form and a bare reference, run the command, assert: `0034-bar.md`
  → next free number, file renamed, branch references rewritten, `main`'s
  `0034-foo` references untouched, and the duplicate check now passes.

## Out of scope

* Draft-slug + CI-on-`main` finalizer (considered, rejected: bot commits to
  `main` and branch-protection changes for a problem the renumber tool solves
  with no machinery).
* Migration renumber tooling / timestamp migration (rejected above).
* Auto-running `adr renumber` from a hook (it is a recovery command the
  developer or agent invokes when the check goes red).
