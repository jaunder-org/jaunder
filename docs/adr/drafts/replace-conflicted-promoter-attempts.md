# ADR-DRAFT: Replace conflicted promoter attempts from fresh main

- Status: proposed
- Date: 2026-09-02
- Issue: [#1214](https://github.com/jaunder-org/jaunder/issues/1214)

## Context

ADR-0152 made a promoter pull request's generated head and diff immutable. That
prevents accidental in-place changes to an accepted decision, but it left no
supported transition when `main` later advanced and made an otherwise healthy
promoter attempt permanently unmergeable. Closing a PR and recreating its stable
branch by hand is unsafe: a stale controller could affect a successor, and a
merely failing, pending, queued, unknown, or externally closed PR does not
establish that a content conflict exists.

The post-merge promoter allocates shared ADR numbers and publishes a generated
commit. Its stable branch is controller-owned: repository administrators and
other principals with Contents or Pull Requests write permission are trusted not
to mutate it during a controller run. Commit author strings, trailers, comments,
and body markers alone are not authority to adopt or delete a branch. The
existing App permissions and workflow triggers are sufficient and remain
unchanged.

## Decision

An immutable promoter PR is one replaceable attempt, not an immutable controller
lifetime. Only a `Generate` event (`main` push or manual dispatch) may replace
an attempt; the serialized generation group owns that transition. A `dequeued`
event remains exact-head auto-merge re-arm recovery only and never closes,
deletes, or replaces an attempt.

Before replacing an open attempt, the controller captures its exact PR number,
head `H`, generated parent `B`, and current `main` `M`. It requires the durable
promoter identity, GitHub `mergeable = CONFLICTING` and
`mergeStateStatus = DIRTY`, a sole parent `B`, and proof that `B` is a strict
ancestor of `M`. It independently reproduces a content conflict with a clean
local merge-tree operation over the exact fetched `M` and `H`. Pending, running,
blocked, unknown, delayed, or failed checks never substitute for that dual
GitHub-and-local conflict proof.

The controller re-reads that exact identity and evidence immediately before
retirement, appends an immutable machine-readable retirement intent, and
re-reads the comment. The intent is authenticated by both the promoter App bot
login and `performed_via_github_app.client_id`; it records the evidence tuple
and is durable authorization to resume the proved transition, never a substitute
for rechecking exact Git objects.

Retirement linearizes at the stable ref. The controller deletes
`automation/adr-promoter` first through receive-pack using the explicit lease
`--force-with-lease=refs/heads/automation/adr-promoter:<H>` and a deletion
refspec. It verifies that the ref is absent while the PR remains at `H`, then
closes and verifies that exact PR at `H`. Only then may it recreate the absent
branch with the ordinary non-force push and verify the remote SHA. It never
force-updates a branch or uses REST/GraphQL ref deletion, which cannot express
the expected-SHA precondition.

Each generated commit has canonical provenance trailers:
`Jaunder-Promoter-Version: 1`, `Jaunder-Promoter-Base: <fresh-main-sha>`, and,
for a replacement, `Jaunder-Promoter-Replaces: <pr-number>@<stale-head-sha>`.
The base equals the commit's sole parent. The version selects immutable
generator and verifier semantics; a future behavior change must use a new
version while retaining verification for versions still reachable remotely.
Trailers are recovery coordinates, not authorization. Adoption of an orphan also
requires the canonical parent/message/trailers, the linked authenticated intent
and closed PR, and deterministic reconstruction of the candidate tree from its
exact detached base.

The controller generates every successor from freshly fetched `main` with the
existing deterministic promotion mutation. It verifies freshness immediately
before the first remote write and again after PR creation before arming. A
candidate that becomes stale before that publication linearization point is an
incomplete publication artifact: the controller records an authenticated
publication-abort intent, lease-deletes it, closes its exact PR, and regenerates
within a fixed bound. It does not treat that abort as replacement of an armed
attempt.

All interruption recovery is state-driven. A later authorized Generate run
re-reads durable intent, exact PR/ref identity, provenance, and postconditions;
it resumes only the corresponding leased deletion, exact close, generation, or
arming step. Ambiguous mutations are classified by exact postcondition reads.
Malformed, mismatched, foreign, or ambiguous state fails closed. Deterministic
check failures remain visible and immutable until new positive conflict evidence
on a later `main` advance authorizes replacement.

## Consequences

A genuinely conflicted promoter no longer blocks later proposed ADRs
indefinitely, while the failed attempt, its immutable intent, and its close
remain observable. The successor allocates from fresh `main` and includes all
pending drafts through the existing deterministic promotion path.

Operators diagnose visible promoter failures and rerun the controller. They do
not manually close, delete, rebase, or promote an attempt. The controller-owned
branch trust boundary remains an explicit assumption: a privileged external
mutation during retirement violates it, and postcondition checks can detect but
cannot undo a completed external effect.
