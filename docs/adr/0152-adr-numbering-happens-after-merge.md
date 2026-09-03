# ADR-0152: ADR numbering happens after feature merge

- Status: accepted
- Date: 2026-08-24
- Issue: [#742](https://github.com/jaunder-org/jaunder/issues/742)

## Context

ADR numbers and the generated ADR index are shared, ordered resources. Under the
ship-time flow in [ADR-0048](0048-adr-out-of-git-draft-workflow.md), two feature
branches that each carry an ADR allocate the same next number and edit the same
index region. GitHub's merge queue cannot reconcile either the filename
collision or the textual table conflict, so otherwise independent changes are
ejected and rerun the full validation surface.

Assigning numbers late is still correct, but a feature branch is not late
enough. The merge queue serializes changes only when they reach `main`; that is
the first state from which a globally unique next number can be allocated
without a race.

Promotion is also the acceptance event under
[ADR-0088](0088-promotion-is-the-acceptance-event.md). Moving promotion must
preserve that transition rather than accepting a decision implicitly in the
feature PR.

## Decision

Feature pull requests commit numberless, `proposed` ADRs under
`docs/adr/drafts/` and cite them by their slug-bearing paths. Drafts are tracked
but remain outside numbered-ADR gates and the generated index. Existing path
citation rewriting is the only draft identity; there is no parallel bare draft
token.

After feature changes reach `main`, a serialized GitHub Actions workflow runs
`cargo xtask adr promote` and opens one stable promoter pull request. Promotion
assigns numbers from current `main`, performs the tracked-file rename, rewrites
path citations and `proposed` status, and regenerates the index. The promoter PR
runs ordinary pull-request and merge-group checks and automatically enters the
merge queue. Only the queue writes the result to `main`.

The workflow uses a dedicated GitHub App, not a personal token or the built-in
`GITHUB_TOKEN`. Its repository permissions are limited to Actions read, Contents
and pull requests read/write, checks read, commit statuses read, and mandatory
Metadata read. Actions read is used only to correlate historical merge-group
workflow runs; the App has no Actions-write, direct-main, or branch-protection
bypass. Promotion commits receive a deterministic App-bot author and committer
at the Git boundary.

Main-push and manual generation events coalesce separately from per-PR dequeue
recovery, so generation cannot replace a pending recovery; no active operation
is canceled. An open promoter's head SHA and generated diff are immutable:
drafts merged later wait for the next promoter. Queue and auto-merge metadata
may change as the PR advances or recovers. A `pull_request: dequeued` workflow
correlates the removed entry with its ephemeral merge-group SHA and evaluates
required contexts on both that SHA and the unchanged PR head. It re-arms
auto-merge only when both sets exist, are complete, and are green, then verifies
that the exact head has either an auto-merge request or live queue membership.
This prevents deterministic failures from looping through the queue while
accepting the common green path where GitHub enqueues immediately.

Promotion remains acceptance. On the healthy path, a merged feature is governed
by a tracked `proposed` draft only for the bounded interval in which the
promoter's ordinary PR and merge-group checks run and the queue merges it. A
failure that prevents the promoter PR from being created, checked, or merged is
not healthy lag: the decision remains proposed until that visible failure is
diagnosed, repaired, and promotion lands.

This decision supersedes ADR-0048's requirements that drafts remain out of git
and that promotion occur during feature shipping. It preserves numberless
authoring, path citations, late allocation, and the promotion mechanics. The
collision-era `adr renumber` command is deprecated for one compatibility release
and removed by [#1169](https://github.com/jaunder-org/jaunder/issues/1169) after
the promoter has operated in production.

## Decision history

On 2026-09-02, [#1214](https://github.com/jaunder-org/jaunder/issues/1214)
extended this workflow with Generate-only cleanup and regeneration of incomplete
controller-owned state. An immutable attempt is replaced only when GitHub and an
exact local merge prove it conflicts with current `main`: the controller
lease-deletes the observed stable-ref SHA, verifies absence, closes the same PR,
and regenerates from freshly fetched `main`. An exact promoter PR without its
ref and a promoter ref without an open PR are resumable interrupted states; a
changed ref fails closed. Pending or failed checks remain visible on the
existing attempt, and dequeue recovery remains exact-head re-arm only. The PR,
stable ref, and workflow result are the operator-visible failure record, so
manual close, deletion, rebase, or local promotion is not part of recovery. The
original immutable-head decision remains unchanged.

## Consequences

Concurrent feature PRs no longer allocate the same ADR number or edit the ADR
index, so ADR bookkeeping cannot make them conflict. Draft decisions and their
architecture projections are visible and reviewable in the feature PR.

Tracking drafts brings them into `doc-links`; draft-internal link forms and the
promotion rename must now be correct before feature merge as well as after
promotion. `promote` must stage deletion of the tracked source, not only
addition of the numbered destination.

The repository gains a privileged automation identity and a PR-producing
workflow. Its authority is deliberately narrower than a human maintainer's, and
all mutations remain observable in an ordinary PR and merge-group run.

A genuine promoter CI failure stops automatic queue retry and requires
diagnosis. This is preferable to an unattended retry loop that repeatedly
consumes queue and CI capacity.
