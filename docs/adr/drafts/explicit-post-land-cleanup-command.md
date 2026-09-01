# ADR-DRAFT: Explicit post-land cleanup command

- Status: proposed
- Date: 2026-09-01
- Issue: [#1155](https://github.com/jaunder-org/jaunder/issues/1155)

## Context

After `cargo xtask pr land [N]` reports a merged PR, its checkout still holds
the feature branch and accumulated build artifacts. The previous workflow
required four manual local operations. Those operations must not turn a
confirmed remote merge into a new approval boundary or permit cleanup of a
different checkout.

The cleanup command needs independent evidence that GitHub reports a merged PR
whose base is exactly `main`, and that the current checkout is on that PR's
exact head ref and SHA with a clean working tree. An omitted PR number cannot
establish uniqueness from a bounded result page: it must examine every merged
candidate before accepting exactly one branch-and-SHA match.

## Decision

Add the explicit, local-only command `cargo xtask pr cleanup [N]`. It observes
PR evidence through the read-only PR source: an explicit number reads that PR;
an omitted number cursor-paginates the complete merged candidate set and
requires exactly one exact head-ref-and-SHA match for the captured checkout.

Before mutation, cleanup fails closed unless the PR is merged, its base is
exactly `main`, a branch is checked out, that branch and local HEAD exactly
match the PR head ref and SHA, and the working tree is clean. Safety refusals
are blocking failures, not successful no-ops.

Cleanup reports the ordered boundaries `pr-cleanup-precheck`, `fetch-origin`,
`verify-origin-main`, `detach-origin-main`, `delete-local-branch`, and
`cargo-clean`, stopping at the first failed boundary. After precheck it fetches
`origin`, proves the captured head SHA is an ancestor of `origin/main`, detaches
at `origin/main`, safely deletes only the captured local branch
(`git branch -d --`), and runs root `cargo clean`.

`pr land` remains the separate approval-bearing merge command and never invokes
cleanup. Cleanup does not stash, reset, force-delete, move local `main`, delete
remote branches, or mutate an issue, project status, or other tracker state.

## Consequences

A completed cleanup exits 0; a refused or failed cleanup exits 1 and reports its
own actionable failure. Failure to construct the command environment before a
result exists remains exit 2. A cleanup failure cannot revise the already
observed merged result from `pr land`.

Local checkout facts and operations are injected behind a narrow interface under
ADR-0016, while GitHub observation stays read-only. Contributors run cleanup
explicitly after a merge and release the issue claim separately by setting
Status to Done.

<!--
Shipping an ADR includes updating docs/ARCHITECTURE.md (and CONTEXT.md when
the ubiquitous language changes) in the same change — the view is the home
of current truth. Later addenda to a shipped ADR are written in past tense
("as of <date>, Y held; current state: ARCHITECTURE.md §Z"), never as
present-tense patches: an ADR is an immutable event. See
docs/adr/0127-architecture-view-materialized-from-adrs.md.
-->
