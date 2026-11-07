# Ordinary issue and PR workflows without xtask

Issue: [#1896](https://github.com/jaunder-org/jaunder/issues/1896)

## Outcome

Developers perform ordinary issue discovery/create/edit and PR
observe/approve/land/unmanaged-cleanup through documented single-command Just
entrypoints, without `cargo xtask`. Just is only the user-facing composition
layer: `gh` owns GitHub operations, `git` owns local repository facts, and
GitHub's protected ruleset/merge queue owns combined-main validation. Do not
build another general workflow framework.

## Approach and tradeoffs

Use Just recipes as the default frontdoor and supported `gh` commands/API for
issue operations, explicit project metadata, native dependency links, body
readback, PR identity and merge actions. Require explicit issue type, labels,
milestone and Priority; fail on body readback mismatch. Use GitHub's native
queue via `gh pr merge --auto` only after explicit per-PR approval; bind action
to the approved head (`--match-head-commit`). Preserve exact head/readback
checks around each operation. `gh pr checks --watch` is useful feedback, but its
documented contract does not establish ruleset-required-check completeness,
distinguish check failure from API/timeout/incomplete evidence, or report queue
ejection/dequeue; it is not an acceptable readiness observer by itself.

For that demonstrated gap, add one host-only `pr-observer` package/binary at
`tools/pr-observer/` and register it as a member of the existing `tools/` Cargo
workspace. Its sole command is read-only PR observation; keep only the polling
and evidence-classification algorithm needed for ADR-0087/0135, reusing the
current algorithm where that is the smallest maintenance choice. It loads the
live main ruleset and complete required contexts; refuses empty/incomplete gate
evidence; observes current head, required checks, mergeability and native queue
state; stops at `ready-to-land` when checks are green and caller action is
needed; and distinguishes failed checks, transport/observer error, timeout,
pending/incomplete evidence, blocked/conflicted/stale/closed-unmerged, merged,
and same-head dequeued/ejected outcomes. It does not land, clean up, or mutate
GitHub. A Just recipe in the root `Justfile` is the one documented observer
frontdoor and invokes this specific binary; Just does not carry polling logic.
Do not retain ordinary PR commands, dispatch, or generic result/report
forwarding in xtask. A supported COTS replacement can displace this observer
only after proving every listed contract; none is currently demonstrated. This
narrow custom host tool is the accepted tradeoff for truthful merge-safety
evidence, not a general workflow framework.

Keep land and cleanup as thin Just frontdoors over `gh` and `git`, not observer
subcommands. Land requires an explicit prior human approval scoped to the PR and
head SHA; the caller supplies that SHA, the recipe verifies it is still the PR
head, and `gh pr merge --auto --match-head-commit <sha>` provides the final
head-drift guard. A Just invocation cannot itself prove that a human approved:
the workflow owner must obtain and record that approval before invoking land.
Cleanup is only for positively verified unmanaged work. Its caller supplies the
expected PR/head and verifies merged base `main`, exact local branch/HEAD, clean
tree, and captured head ancestry of `origin/main` before safe local
detach/delete and `cargo clean`. Just cannot infer Manager assignment ownership;
Manager-assigned or uncertain ownership means do not invoke cleanup, preserve
the branch/checkout, and hand off to Manager. No stash/reset/force or tracker
mutation.

### Approved read-only continuation refinement

Human approval `issue-1896-observer-continuation-v1`, request
`4fc22750-206c-49bd-9056-c7b3e07543a5` ("Approve narrow observer extension"),
permits observer `--expected-head <SHA>` and `--acknowledged-ejection <URL>`
inputs backed by existing `WatchContinuation`. This narrowly refines ADR-0220's
no-operator-head/run-arguments assumption, not its safeguards. Just captures the
historical URL from live ejection preflight, then requires a one-snapshot
`ready-to-land` verdict from the SAME classifier with only that run acknowledged
and the same expected head before ONE separately approved native head-bound
merge call. It observes that same head afterward. Only the historical run is
acknowledged; its failed verdict is unchanged, fresh failure is terminal, and
delayed visibility uses the existing deadline. No second classifier/poller,
mutation retry, extra armer or observer mutation is permitted. Ordinary land
also stops on silent no-op or uncertain readback rather than rearming.
Implementation approval is not per-PR/head merge or recovery approval.

Human approval `issue-1896-observer-queue-continuation-v1`, request
`df5154f8-1d48-4725-9550-72460d456d8b` ("Approve queue-history input"),
authorizes read-only `--observed-queued-head <SHA>`, requiring and equaling
`--expected-head`. Just supplies it ONLY from positive live same-head queued
post-action readback. The observer seeds existing `Progress` so queue membership
lost before its establishment snapshot still yields the existing confirmed
`dequeued` classification. Default observation is unchanged. This transfers
observed history, not approval, a trust registry or mutation authority; no
second classifier/poller or bypass is permitted.

Implementation includes new, approved, numberless ADR draft documentation that
supersedes ADR-0087's xtask observer-ownership decision and amends ADR-0028's
host-tool placement boundary to permit this narrowly scoped host-only tool; it
must also update the architecture projection under current ADR policy. Those
accepted decisions remain in force until their separately approved successor
decision is accepted through the repository's ADR process. The spec/design
approval must explicitly cover both ADR-0087 ownership and ADR-0028 placement;
this wording does not silently change either ADR. Preserve ADR-0135's actionable
`ready-to-land`/read-only-observer contract, ADR-0171's unmanaged cleanup
preconditions, and ADR-0077's native merge queue semantics unchanged. The
privileged ADR promoter remains untouched.

## Acceptance

- One documented Just command per ordinary issue create/edit/discovery, PR
  observation, approved land, and unmanaged cleanup; every ordinary xtask CLI
  caller/report consumer is retired with its final workflow. Issue operations
  preserve explicit metadata, body readback and native dependency links. The
  only retained PR executable is `tools/pr-observer/`, registered in `tools/`
  and exposing read-only observation only.
- PR observation proves complete live ruleset requirements and exposes all
  outcomes named above; tests cover late required aggregate appearance,
  empty/incomplete rules, required failure versus API failure, timeout,
  actionable readiness, head drift, merge-queue success, and same-head
  dequeue/ejection. No missing evidence is interpreted as success; observation
  cannot mutate.
- Land requires recorded human approval for the exact PR/head before its Just
  recipe is invoked; that recipe checks the supplied SHA against the live PR
  head and uses `gh --match-head-commit`. The caller invokes cleanup only after
  verifying unmanaged ownership and refuses invocation for managed or uncertain
  ownership, preserving the checkout. Cleanup checks exact merged head,
  clean-tree and ancestry guards; claim release follows existing ship policy.
- Implementation includes explicitly approved successor/amendment ADR drafts for
  both ADR-0087 observer ownership and ADR-0028 tool placement, plus the
  required architecture projection. ADR-0135, ADR-0171 and ADR-0077 contracts
  remain unchanged.
- A focused standalone observer is retained only for demonstrated contracts
  unsupported by `gh`/other supported COTS. No generic runner/framework, giant
  recipe or one-for-one CLI rewrite is introduced. Docs explain commands,
  evidence and failure meanings.

## Boundaries

No wholesale xtask/devtool retirement, weakening checks/approval, managed
cleanup, privileged promoter changes, manual multi-command reconstruction, or
broad redesign of GitHub semantics. This is a proposed design contract only:
implementation waits for Lead exact-tree acceptance and explicit human
spec/design approval that specifically covers both ADR-0087 ownership and
ADR-0028 placement. Implementation must then record those approved decisions in
successor/amendment ADR drafts and update the architecture projection before
shipping; existing ADR authority is not changed by this spec. ADR-0135, ADR-0171
and ADR-0077 guarantees must not be weakened. A short outline is triggered after
spec approval because the host executable boundary and observer/land integration
are durable architectural changes; it receives its own approval before
implementation.
