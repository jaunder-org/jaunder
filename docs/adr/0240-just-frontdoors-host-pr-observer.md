# ADR-0240: Just frontdoors with a host-only PR observer

- Status: accepted
- Date: 2026-11-07
- Issue: [#1896](https://github.com/jaunder-org/jaunder/issues/1896)

## Context

Issue #1896 replaces ordinary issue and PR workflows currently exposed through
`cargo xtask` with single-command Just frontdoors using supported `gh` and `git`
operations. The approved design moves read-only PR observation to a focused
host-only executable at `tools/pr-observer/`, a member of the existing `tools/`
Cargo workspace.

[ADR-0087](0087-xtask-github-pr-observation.md) assigned host-side GitHub PR
observation to xtask even though this operation neither invokes Nix nor analyzes
build outputs. Its decision also records essential safeguards: the live ruleset
is the required-check authority, observation and merge authority are separate,
and the observer must distinguish incomplete/untrustworthy evidence from a
mergeable result. [ADR-0028](0028-devtool-vs-xtask-boundary.md) currently places
host tools that invoke Nix or analyze build output in xtask and reserves
`tools/` for sandbox tools or pure shared libraries. GitHub CLI check watching
alone does not demonstrate the complete ruleset, failure/transport/timeout, and
native merge-queue outcome evidence required by ADR-0087,
[ADR-0135](0135-pr-watch-actionable-handoff.md),
[ADR-0195](0195-dynamic-pr-check-classification.md), and
[ADR-0198](0198-active-workflow-incomplete-pr-evidence.md).

## Decision

For issue #1896, make root Just recipes the single documented frontdoor for
ordinary issue discovery/create/edit and PR observe/land/unmanaged-cleanup. Use
`gh` for GitHub operations and `git` for local repository facts; Just owns only
short operation sequences and their explicit inputs, not a general workflow
engine or PR polling state machine.

Add the purpose-specific host-only package `tools/pr-observer/` to the existing
`tools/` workspace. Its sole public operation is read-only PR observation. It
reuses only the polling and evidence-classification algorithm required to read
the live main ruleset, correlate complete required-check and workflow evidence,
and report caller-actionable/terminal queue outcomes. Empty or incomplete
evidence fails closed. It may not arm auto-merge, mutate GitHub, perform local
cleanup, or expose a generic command/result framework. A Just recipe invokes
this executable; no ordinary PR command, dispatch, or report forwarding remains
in xtask. Nix may package pinned host dependencies, but no Nix derivation runs
the observer or xtask.

This decision supersedes ADR-0087's assignment of ordinary PR observation to
xtask and amends ADR-0028 to permit this narrowly scoped host-only executable in
`tools/`. ADR-0087's `gh` transport, live-ruleset authority, complete evidence,
observer/armer separation and failure semantics remain; ADR-0135's
`ready-to-land`/passive-observation distinction, ADR-0195's workflow-graph
classification and ADR-0198's incomplete-evidence policy remain unchanged.
`devtool` remains the Nix-sandbox tool; xtask remains for its other host duties.
The existing numbered ADR files remain immutable historical decisions; this
draft records the newer ownership rule without editing them.

Land and unmanaged cleanup are separate thin Just operations over `gh`/`git`,
not observer subcommands. Land requires a separately obtained and recorded human
approval for the exact PR/head. The caller supplies that SHA; the recipe checks
it against the live PR head and invokes GitHub's native queue using
`gh pr merge --auto --match-head-commit <sha>`. Just does not attest that human
approval exists. `just pr-land <PR> <approved-head-SHA> [--reenqueue]` makes at
most one native call after shared-observer readiness; silent no-op or uncertain
action/readback never permits another call. It verifies actual arm, queue or
complete merged readback, then observes the same approved head to merge;
arming/queueing is not merge success.

Explicit recovery preserves [ADR-0220](0220-explicit-one-shot-pr-reenqueue.md)'s
one-shot semantics. Human approval `issue-1896-observer-continuation-v1`,
request `4fc22750-206c-49bd-9056-c7b3e07543a5` ("Approve narrow observer
extension"), authorizes read-only `--expected-head <SHA>` and
`--acknowledged-ejection <URL>` backed by existing `WatchContinuation`. This
narrowly supersedes ADR-0220's assumption that no operator-supplied head/run
inputs are needed, without changing its failed-verdict or recovery guarantees.
Just captures a historical URL from live `ejected` preflight, refuses queued or
armed PRs, then uses the SAME observer for a one-snapshot readiness proof with
only that run acknowledged before ONE separately approved head-bound native
call. Post-action observation uses that same head/acknowledgment. No second
classifier/poller, retries, extra armer or observer mutation is introduced. Only
the historical run is disregarded as an ejection discriminator; its failed
verdict remains failed, delayed visibility uses the normal deadline, and a fresh
distinct failure is terminal. Observation and implementation approval never
authorize per-PR/head merge or recovery.

Approval `issue-1896-observer-queue-continuation-v1`, request
`df5154f8-1d48-4725-9550-72460d456d8b` ("Approve queue-history input"), adds
only read-only `--observed-queued-head <SHA>`, requiring equality with
`--expected-head`. Just supplies it ONLY after positive live same-head queued
post-action readback. Existing `Progress`/poller retains that queue history even
if the entry vanishes before observer establishment, preserving ADR-0135/0220's
confirmed same-head dequeue outcome. Default observation is unchanged. This is
observed evidence, not a trust registry, approval bypass or mutation capability;
no second classifier/poller is introduced.

Unmanaged cleanup preserves
[ADR-0171](0171-explicit-post-land-cleanup-command.md)'s guards: merged PR
targeting `main`, exact local branch and HEAD match, clean tree, and captured
head ancestry of `origin/main` before safe detach/delete and `cargo clean`. The
approved replacement frontdoor is Just; the guard contract does not change. The
caller must positively establish that the assignment is unmanaged and must not
invoke cleanup for Manager-assigned or uncertain ownership; Just cannot infer
that assignment fact. Managed work preserves its branch and checkout for Manager
handoff. [ADR-0077](0077-adopt-github-merge-queue.md)'s native merge queue
remains authoritative. The privileged ADR promoter remains untouched.

The new issue #1896 design was approved under decision identity
`issue-1896-spec-design-v1` (Manager relay request
`f83b8b7a-3af7-4396-9dc5-347f089eec3c`). That approval explicitly covers the
ADR-0087 observer ownership and ADR-0028 host-tool placement changes recorded
here. This draft remains proposed until serialized promotion after the feature
reaches `main`.

## Consequences

- `Justfile` supplies ordinary single-command frontdoors; issue operations keep
  explicit metadata/body readback and native dependency links, and land/cleanup
  retain their scoped caller preconditions.
- A narrow host-only executable retains the observer evidence algorithm that
  `gh pr checks --watch` alone does not provide, without keeping ordinary PR CLI
  orchestration, dispatch, or generic result/report machinery in xtask.
- `tools/` now includes one purpose-specific host executable in addition to its
  existing tool roles; no general host-runner abstraction is introduced. This
  exception is narrow and does not move sandbox execution into the host.
- ADR-0135, ADR-0171, ADR-0195, ADR-0198, ADR-0220, and ADR-0077 safeguards
  remain in force. Any future change to their guarantees requires its own
  explicit approval.
- The accepted
  [specification](https://github.com/jaunder-org/jaunder/blob/6fcec003899c5178ea9b4b380b1ce4952a264447/docs/superpowers/specs/2026-11-06-issue-1896-ordinary-issue-pr-workflows.md)
  remains available in branch history after working-document retirement.
  Promotion assigns this draft a number after merge; do not edit
  `docs/README.md` or invoke promoter tooling from the feature branch.
