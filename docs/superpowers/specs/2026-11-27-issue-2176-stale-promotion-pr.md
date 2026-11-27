# Issue #2176 — replace a promotion PR that main has made stale

## Outcome

When `main` gains a new citation of a draft that the open ADR promoter PR
promotes, the next Generate run retires that PR and regenerates it from fresh
`main`. The replacement rewrites every citation, so `doc-links` passes. This
applies whether the stale PR is unarmed, armed or already queued.

## Approach and tradeoffs

Today `_existing_or_retire` replaces an open promoter PR only on a proven
conflict: GitHub `CONFLICTING`/`DIRTY`, a `baseRefOid` equal to fresh `main`,
the head's sole parent strictly behind `main`, and a conflicting local
`git merge-tree`. #2170 merges cleanly, so it was never replaced, even though
its merged tree still cites the draft it promotes (in ARCHITECTURE.md and in
drafts #2163 added). Once its group failed, the unarmed failed-check path left
it as `Existing` forever.

Change this to one local, exact-object staleness proof. It runs for every open
exact promoter PR whose stable ref equals its head, after the existing
interrupted-state paths (PR without ref, ref without PR) and before the
armed/queued and failed-check returns:

1. Fetch the head and fresh `main`. If the head has no sole parent, or that
   parent is not strictly behind `main`, the PR is **not stale**. The logic
   falls through to the unchanged arm/failed-check handling. (A parent equal to
   `main` is fresh by construction.)
2. `git merge-tree --write-tree main head`:
   - exit 1 (conflict): **stale**;
   - exit 0: the tree OID is read from the first stdout line;
   - any other exit: an error.
3. List the drafts the head deletes:
   `git diff --no-renames --diff-filter=D --name-only -z parent head -- docs/adr/drafts/`.
   - No deleted drafts: **not stale**.
   - Otherwise run `git grep -l --fixed-strings -e drafts/<slug> … <tree>`: exit
     0 is **stale**, exit 1 is not stale, any other exit is an error.

A stale PR goes through the existing retirement path: lease-delete the exact
ref, verify its absence, close and verify the PR, then regenerate (`Replaced`).
GitHub may close a PR itself once its head ref is deleted. `_close` therefore
treats an already-closed PR with the same exact identity as having met its
postcondition. This applies the existing "accept an outcome once its exact
postcondition is observed" rule. A probe error fails the run nonzero and never
retires.

Decisions:

- **One local proof replaces the GitHub `DIRTY` precondition.** The local merge
  of freshly fetched exact objects is authoritative. GitHub's mergeability is a
  lagging cache of the same fact, and keeping it would mean two parallel
  staleness rules. The cost is one fetch, one merge-tree and one grep on every
  Generate run that finds an open PR. The DIRTY-gated tests are replaced, not
  kept beside the new ones.
- **Queued and armed PRs are checked too.** A merge group already built with the
  stale PR can still fail `doc-links`. The fix bounds that damage: the first
  Generate run after the citing push retires the stale PR instead of leaving it
  to fail later groups or to sit failed indefinitely.
- **Citation matching mirrors promotion's rewrite.** Promotion rewrites the
  literal substring `drafts/<slug>` (`promotion.py`), and the check uses the
  same substring test. The two share a prefix hazard: a later draft
  `<slug>-more` that main cites also triggers one replacement. That replacement
  converges, because it promotes both drafts.
- **No-churn invariant.** A regenerated tree contains no `drafts/<slug>` for any
  slug it promotes, and its parent is `main`. It therefore stays fresh until
  `main` adds another citation.
- **The immutable-head rule stands.** An attempt is never force-updated, only
  retired and replaced.

Docs:

- Add decision-history entries to ADR-0152 and ADR-0242. Both are accepted; the
  original decision text stays.
- Update `docs/ARCHITECTURE.md`'s recovery paragraphs. "Armed or queued
  promoters remain `Existing`" and "drafts merged later wait" become qualified.
- Update the same "drafts merged later wait" wording in `CONTRIBUTING.md` and
  `docs/ci-merge-queue.md`.

## Acceptance

- Git-boundary tests (`test_controller_git.py`) cover our argv and exit
  interpretation for the merge-tree tree OID, the deleted-draft diff and the
  multi-`-e` grep. They also cover the empty-list short circuit and that any
  exit above 1 is an error.
- A real temporary-Git test covers our composition (deleted drafts → stems →
  grep on the merged tree):
  - it builds a promotion commit;
  - adding a new citation of the promoted draft on `main` makes the PR stale;
  - without that citation the PR is fresh.
- Controller fake-seam tests:
  - a stale PR is `Replaced` for each of the unarmed, armed and queued states;
  - a fresh PR keeps its current `Existing`/arm/failed-check behaviour;
  - a parent equal to `main`, or a missing parent, skips the merge probe;
  - a probe error fails the run without retiring;
  - an already-closed exact PR after ref deletion counts as verified.
- The promoter package tests and `dev check-no-test` are green.
- After merge (observation, not unit proof), the main push's Generate run
  replaces #2170, and the replacement lands with `doc-links` green. If GitHub
  refuses to delete the head ref of a queued PR, that observation exposes it.
  The issue closes only after this observation.

## Boundaries

- No change to dequeue recovery, App permissions, workflow triggers or the
  promotion mutation.
- The `action_required` conclusion on the dequeue recovery runs is not diagnosed
  here. If it still matters after this fix, it gets its own follow-up issue.
- No manual close or rebase of #2170. The controller replaces it.
