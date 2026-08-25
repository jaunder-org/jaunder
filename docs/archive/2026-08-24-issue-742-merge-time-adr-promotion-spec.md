# Merge-time ADR promotion

## Outcome

Feature pull requests commit numberless ADR drafts without allocating a shared
number or modifying the generated ADR index. A serialized automation PR promotes
those drafts from `main`, passes the normal merge-queue checks, and lands
without human intervention or direct commits to `main`.

Concurrent feature pull requests that add ADRs therefore cannot conflict over an
ADR filename or the `docs/README.md` table.

## Load-bearing decisions

- A feature PR commits `docs/adr/drafts/<slug>.md`. Drafts stop being
  gitignored, remain `proposed`, and stay outside numbered-ADR enumeration and
  the generated index until promotion.
- The slug-bearing draft path is the sole pre-promotion identity. Existing path
  citations are valid in feature PRs and promotion rewrites them to the numbered
  path; no `ADR-DRAFT-<slug>` token namespace is introduced.
- Tracked drafts must use link forms that resolve both before and after their
  one-directory promotion. The documented draft-link rules and `doc-links`
  coverage change with the tracked-draft lifecycle.
- Promotion remains the ADR acceptance event. It assigns the next free number,
  moves the tracked source, rewrites `proposed` to `accepted`, rewrites path
  citations, regenerates the ADR index, and stages the complete rename and all
  projections. The short interval between a feature merge and promoter merge is
  an explicit proposed-decision lag.
- Feature shipping no longer promotes ADRs. A push to `main` and a manual
  dispatch both invoke one promoter workflow. Generation prepares fresh `main`,
  promotes locally, and checks the staged diff before reading queue policy; a
  no-draft or no-diff run is therefore a successful no-op, while a real diff
  still requires a live queue and required contexts before publication.
- The workflow uses one stable promoter branch and permits exactly one open PR
  for that head/base pair. If one is already open or queued, later drafts wait;
  the existing promoter's head SHA and generated diff never change. Queue and
  auto-merge metadata may change as it advances or recovers. Its merge into
  `main` triggers the next promotion pass for remaining drafts.
- Branch generation is serialized: main-push and manual events share one
  generation concurrency group, no active run is canceled, and redundant pending
  generation may coalesce because every run derives from current `main`. Dequeue
  recovery uses a separate per-PR operation group so a generation event cannot
  replace the event authorized to re-arm that head.
- The promoter uses the repository's pinned `setup-ci` environment and a
  dedicated GitHub App with repository permissions `Actions: read`,
  `Contents: read/write`, `Pull requests: read/write`, `Checks: read`, and
  `Commit statuses: read`; mandatory `Metadata: read` remains implicit. Actions
  read is limited to historical `merge_group` workflow-run metadata used for
  dequeue correlation. It receives no Actions-write, Administration,
  direct-main, or branch-protection bypass authority. The built-in
  `GITHUB_TOKEN` is not used to create, update, inspect, or queue the promoter
  PR. Promotion commits receive a deterministic App-bot author and committer at
  the Git boundary rather than through workflow-local Git configuration.
- The promoter PR receives the repository's ordinary pull-request and
  `merge_group` checks. It arms auto-merge with `gh pr merge --auto`, then
  verifies the unchanged head has either an auto-merge request or exact queue
  membership; green PRs commonly enter the queue immediately without retaining
  an auto-merge request. The merge queue remains the only writer to `main`, and
  the App has no branch-protection bypass.
- The workflow also handles `pull_request: dequeued`, filtered to the exact
  promoter head and base. It correlates the removed queue entry with its
  ephemeral `merge_group` SHA and evaluates required contexts on both that SHA
  and the unchanged PR head. It re-arms auto-merge only when both context sets
  exist, are complete, and are green. A failed, missing, or incomplete
  merge-group context stops retry and leaves the PR visible for diagnosis;
  deterministic failures cannot form an enqueue loop.
- `cargo xtask adr renumber` is deprecated, not removed in this issue. A
  separate issue records deletion after one release has exercised serialized
  promotion.
- This decision supersedes ADR-0048's out-of-git and ship-time portions while
  preserving its numberless-authoring goal. ADR-0088's acceptance semantics and
  the numbered-ADR gate boundaries remain in force.

## Acceptance

- Two feature branches can each add a distinct draft and their architecture
  citations without either branch adding a numbered ADR or changing
  `docs/README.md`; both feature PRs can merge without ADR bookkeeping
  conflicts.
- A tracked draft and every supported intra-draft link pass `doc-links` before
  promotion. After promotion, the same links and external path citations resolve
  to the assigned numbered ADR.
- Promotion of a tracked draft stages one complete rename, the accepted numbered
  content, every rewritten citation, and the regenerated index. A second run is
  a clean no-op.
- Numbered-ADR format, collision, index-parity, and architecture-view gates
  continue to ignore drafts and enforce promoted ADRs.
- Overlapping main-push and manual workflow events produce at most one active
  branch mutation and one open promoter PR; dequeue recovery cannot be replaced
  by either generation trigger. No run cancels an in-progress operation.
- A second draft landing while a promoter is open does not alter that PR's head
  SHA or generated diff. After the first promoter merges, a later promoter
  handles the waiting draft.
- The promoter PR runs required pull-request checks, enters the merge queue
  automatically, runs required merge-group checks, and reaches `main` only via
  the queue.
- A `pull_request: dequeued` event for the exact promoter head/base re-arms only
  when the unchanged PR head and its correlated prior `merge_group` SHA both
  have complete, green required contexts. Failed, missing, or incomplete
  contexts do not repeatedly queue the PR.
- The workflow succeeds without a personal access token. Its GitHub App
  permission manifest is limited to the enumerated repository permissions,
  including Actions read but no Actions write, and grants neither direct-main
  nor branch-protection bypass authority.
- The ADR authoring, projection, shipping, collision-recovery, and merge-queue
  documentation describe the tracked-draft promoter flow consistently.
- A follow-up issue owns final removal of `adr renumber` after the compatibility
  release.

## Boundaries

- This issue does not change how ADR numbers are allocated, their monotonic
  sequence, the generated index format, or promotion's status rewrite.
- It does not invent a second draft identifier, accept ADRs in feature PRs, or
  place unnumbered drafts in the generated index.
- It does not bypass required checks, merge-queue serialization, or repository
  branch protection.
- It does not build a general-purpose pull-request automation framework.
- It does not delete `adr renumber`; only deprecation and follow-up tracking are
  in scope.
