# Conflicted Promoter Replacement Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` for independent code,
> documentation, and external-skill slices. This outline covers compact
> cleanup-and-regenerate recovery for the serialized ADR promoter.

## Scope

In:

- Replace a positively conflicted immutable promoter attempt from fresh `main`.
- Clean up incomplete controller-owned publication state and regenerate it.
- Preserve visible failed required checks and exact-head dequeue re-arm.
- Align ADR-0152, architecture, contributor, and operator guidance.

Out:

- Changing ADR allocation, promotion ordering, merge-queue policy, workflow
  triggers, or App permissions.
- Retrying deterministic check failures without new positive conflict evidence.
- Protecting the controller-owned branch from concurrent privileged external
  mutation.
- Editing `CONTEXT.md` or generated `docs/README.md`.

## Task outline

- [x] Task 1: Implement compact promoter cleanup and regeneration
  - Files: `xtask/src/pr/promoter.rs`, `xtask/src/pr/snapshot.rs`, and only
    shared helpers required in `xtask/src/git.rs`, `xtask/src/pr/gh.rs`, or
    `xtask/src/test_support.rs`.
  - Contract: Generate reads the open exact promoter PR and stable ref. An open
    PR with an absent ref is closed and regenerated; a different ref fails
    closed. Positive GitHub/local conflict evidence lease-deletes the exact ref,
    closes the exact PR, and regenerates. Armed/queued remains `Existing`;
    failed required checks remain visible; unarmed pending/green arms and is
    verified at its exact head. A ref without an open PR is lease-deleted and
    regenerated. Dequeue only re-arms an exact head.
  - Contract: fresh generation starts from freshly fetched `main`, uses only a
    non-force push, exact postcondition reads, create-or-read, and exact arm
    verification. A later Generate run replaces only on new positive conflict
    evidence.
  - Verification: focused controller and adapter tests cover Generate
    classifications, positive conflict evidence, exact-SHA leased deletion,
    failed-check preservation, duplicate convergence, and ambiguous API results.
    Temp Git repositories prove changed-head deletion refusal and non-force
    publication.

- [x] Task 2: Record and project the compact recovery model
  - Files: `docs/adr/0152-adr-numbering-happens-after-merge.md`,
    `docs/ARCHITECTURE.md`, and `CONTRIBUTING.md`; delete the superseded
    numberless draft.
  - Contract: retain ADR-0152's Decision text and add only a short past-tense
    history annotation. Architecture and contributor guidance describe one
    cleanup-and-regenerate state machine: exact-SHA cleanup, fresh-main
    regeneration, failed-check preservation, duplicate convergence, and visible
    crash-state recovery. Operators rerun the controller and never perform
    manual promoter recovery.
  - Verification: ADR, architecture, contributor prose, and archived spec agree
    on the state machine. `CONTEXT.md` and `docs/README.md` remain unchanged.

- [x] Task 3: Update authoritative operator skills and distribute them
  - Files:
    `/home/mdorman/src/agent-configuration/projects/jaunder/.rulesync/skills/jaunder-adr/SKILL.md`
    and `jaunder-ship/SKILL.md` on the current agent-configuration branch.
  - Contract: failed promoter guidance preserves visible failed checks,
    distinguishes them from controller recovery state, directs operators to
    rerun the controller, and forbids manual close/delete/rebase/promote
    recovery. Generated `.agents`/`.claude` copies are distribution outputs,
    not Jaunder PR inputs.
  - Verification: commit the authoritative files without a `Co-Authored-By`
    trailer and without pushing; run
    `/home/mdorman/src/agent-configuration/bin/refresh-agent-config jaunder` and
    `/home/mdorman/src/agent-configuration/bin/refresh-agent-config --check jaunder`.

- [x] Task 4: Integrate and gate the complete change
  - Contract: all issue acceptance criteria map to observable tests or
    documentation; no obsolete manual-recovery guidance remains.
  - Verification: run `devtool run -- cargo xtask check --no-test`, perform
    parallel Standards/Spec and security reviews, then use `jaunder-commit` for
    the staged Jaunder tree. The commit hook owns `precommit`; `jaunder-ship`
    owns pre-push, PR CI, and merge approval.

## Risk checks

- A stale controller cannot delete a changed stable ref because cleanup uses an
  exact-SHA lease.
- GitHub `UNKNOWN`, queue delay, pending checks, or failed checks cannot create
  conflict authorization.
- An incomplete ref/PR state is cleaned up only at the observed exact head and
  is regenerated from fresh `main`.
- A duplicate Generate run converges on the exact armed or queued promoter.
- Existing promotion tests continue to prove fresh-main numbering, all pending
  drafts, deterministic slug order, citation/status rewrite, and index
  regeneration.
