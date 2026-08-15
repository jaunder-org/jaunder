# #1044 — make `pr watch` stop at the next actionable state

Issue: [#1044](https://github.com/jaunder-org/jaunder/issues/1044). Milestone:
Developer tooling & DX. Refines the PR-observation interface introduced by
[ADR-0087](../../adr/0087-xtask-github-pr-observation.md).

## Problem

`cargo xtask pr watch` currently waits for a terminal GitHub state. The normal
shipping flow invokes it before merge approval, while `cargo xtask pr land` is
the command that embodies that approval and arms auto-merge. Once every required
check is green, an open, mergeable, unarmed, unqueued PR cannot advance until
`pr land` runs, but the watcher deliberately continues until its 90-minute
budget expires.

The implementation already diagnoses the dead wait with
`green and unqueued — nothing will happen until pr land arms the merge`. That
warning is streamed to stderr, but `devtool run` parks the stream until the
child exits. An agent therefore does not receive the actionable diagnosis and
remains blocked long after checks finish.

## Decisions

| ID      | Decision                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1**  | Default `pr watch` observes until the **next actionable outcome**, not necessarily a terminal GitHub state. Observation remains read-only; `pr land` remains the only command that arms auto-merge and running it remains the merge approval.                                                                                                                                                                                                                                                                 |
| **D2**  | After existing adverse verdicts have taken precedence, an open PR with a non-empty, entirely successful required-check set, `mergeable: MERGEABLE`, a landable merge-state status, no current-head ejection, no auto-merge arm, and no queue entry yields the new outcome `ready-to-land`. Both blocking watch and `--once` classify that snapshot identically.                                                                                                                                               |
| **D3**  | `ready-to-land` is a successful watch result: human output names the handoff, JSON serializes `pr.outcome` as `ready-to-land`, `CommandResult.ok` is true, and the process exits 0. `pr land` retains its separate invariant that exit 0 means `merged`.                                                                                                                                                                                                                                                      |
| **D4**  | Queue history is scoped to the observed head SHA. A watch that sees a head in the queue and later sees that same head's queue entry vanish without a failed current-head merge-group run yields the new actionable outcome `dequeued`. A head change resets the queue-history predicate. `dequeued` is distinct from `ejected`, because the observer has readable but anomalous state rather than evidence of a failed merge-group run.                                                                       |
| **D5**  | An empty required-check set yields `watcher-error` immediately. The observer cannot safely claim readiness when a wrong repository, ruleset drift, or insufficient token scope may have made the gate appear empty. The acting command fails closed too: `pr land` returns `watcher-error` before acquiring or invoking `PrArmer`.                                                                                                                                                                            |
| **D6**  | An already armed or queued PR continues through queue processing to `merged`, `ejected`, `dequeued`, or another existing actionable outcome. No caller action is needed merely because its ordinary checks became green.                                                                                                                                                                                                                                                                                      |
| **D7**  | `pr watch --until merged` explicitly preserves passive observation across a green-but-unarmed state, allowing another actor to approve and arm the PR. The mode still returns early for any adverse or action-requiring outcome (`checks-failed`, `ejected`, `dequeued`, `blocked`, `conflicted`, `closed-unmerged`, `stale`, `watcher-error`) and still observes the configured timeout. It does not mutate, re-run, rebase, arm, or re-enqueue.                                                             |
| **D8**  | In `--until merged` mode, green-but-unarmed is represented as a `ready-to-land` phase/event rather than the misleading `awaiting-checks` phase, but it is not returned as the outcome. Heartbeats report that phase while waiting for an external arm.                                                                                                                                                                                                                                                        |
| **D9**  | `--once` remains one snapshot: `ready-to-land` exits 0; armed, queued, or still-checking snapshots remain `pending { phase }` and exit 1; adverse outcomes retain their existing behavior. `--until merged` and `--once` are mutually exclusive because one requests persistence and the other forbids it.                                                                                                                                                                                                    |
| **D10** | Existing adverse verdicts retain precedence over the new handoff. In particular, strict-ruleset `BEHIND` remains `stale`, conflicts remain `conflicted`, and failed required checks remain `checks-failed`; none can become successful `ready-to-land`. API strike/rate-limit behavior, change-only events, heartbeat behavior, subject resolution, and the observe/act capability split otherwise remain unchanged.                                                                                          |
| **D11** | The stopping rule is recorded in a new ADR that refines ADR-0087 rather than superseding it: ADR-0087's host-side ownership, `gh` transport, ruleset-derived gate, and separate observer/armer remain current. `CONTEXT.md` is unchanged because this is a development-tool interface decision, not Jaunder domain language.                                                                                                                                                                                  |
| **D12** | For a green, unarmed, unqueued PR, `CLEAN`, `HAS_HOOKS`, `UNSTABLE`, and non-strict `BEHIND` are landable statuses. `BLOCKED`, `DRAFT`, and defensive `DIRTY` return the new exit-1 `blocked` outcome with the exact GitHub status in `detail`. `mergeable: UNKNOWN` or `mergeStateStatus: UNKNOWN` continues in the new `awaiting-mergeability` phase (and is `pending` under `--once`) because GitHub has not produced a stable verdict. A decisive adverse status outranks transient unknown mergeability. |

## State behavior

| Observed state                                                     | Default `pr watch`                  | `pr watch --until merged`           |
| ------------------------------------------------------------------ | ----------------------------------- | ----------------------------------- |
| Required checks pending                                            | Continue `awaiting-checks`          | Continue `awaiting-checks`          |
| Green, unarmed, unqueued, mergeability or merge state unknown      | Continue `awaiting-mergeability`    | Continue `awaiting-mergeability`    |
| Green, unarmed, unqueued, blocked/draft/dirty                      | Return `blocked`                    | Return `blocked`                    |
| Green, unarmed, unqueued, strict and behind                        | Return existing `stale`             | Return existing `stale`             |
| Green, unarmed, unqueued, demonstrably landable                    | Return `ready-to-land`              | Continue in `ready-to-land` phase   |
| Armed, not yet queued                                              | Continue                            | Continue                            |
| Queued                                                             | Continue                            | Continue                            |
| Same head previously queued, now absent, no explanatory failed run | Return `dequeued`                   | Return `dequeued`                   |
| New head unqueued after a previously queued head                   | Classify the new head independently | Classify the new head independently |
| Empty required-check set                                           | Return `watcher-error`              | Return `watcher-error`              |
| Existing adverse or terminal outcome                               | Return it                           | Return it                           |

## Acceptance criteria

- **AC1 — default handoff.** Given an open, unarmed, unqueued PR whose non-empty
  required-check set is entirely successful, whose mergeability and merge-state
  status satisfy D12, and whose current head has no ejection run, blocking
  `pr watch` returns without sleeping again. Its report has
  `outcome: "ready-to-land"`, `ok: true`, exit code 0, the observed head SHA,
  and human output that says the PR is ready to land.
- **AC2 — snapshot parity.** The same snapshot under `pr watch --once` produces
  the same `ready-to-land` outcome and successful exit, not `pending` /
  `awaiting-checks`.
- **AC3 — pending checks still block.** A missing or pending required context
  remains `pending { phase: "awaiting-checks" }` under `--once` and keeps
  polling in default blocking mode.
- **AC4 — armed and queued watches continue.** An armed PR and a queued PR do
  not return `ready-to-land`; scripted transitions through those states reach
  the existing `merged` or `ejected` outcomes.
- **AC5 — unexplained dequeue is actionable and head-scoped.** After the same
  invocation has observed one head in the queue, a later open, green, unqueued
  snapshot of that same head with no failed current-head merge-group run returns
  `dequeued`, includes a detail explaining the missing evidence, and exits 1
  without another sleep. A queued head A followed by a different unqueued head B
  resets queue history and cannot return `dequeued` solely because A was queued.
- **AC6 — existing adverse precedence remains.** A matching failed current-head
  merge-group run returns `ejected`; a stale run from a prior head does not. A
  strict-ruleset `BEHIND` snapshot remains `stale`, never `ready-to-land`.
  Existing conflict and required-check-failure precedence is unchanged.
- **AC7 — empty gates fail closed before action.** A ruleset response with zero
  required contexts returns `watcher-error` immediately with a detail that
  readiness could not be established. This holds in default, `--once`, and
  `--until merged` modes. `pr land` returns the same outcome before acquiring or
  invoking `PrArmer`.
- **AC8 — explicit passive mode.** `pr watch --until merged` crosses a green,
  unarmed snapshot without returning, emits `ready-to-land` as its phase, and
  can subsequently observe external arming, queue entry, and `merged`. It still
  stops on every adverse/actionable outcome and on the configured timeout.
- **AC9 — flag contract.** CLI help documents the default ready-to-land handoff
  and `--until merged`; clap rejects combining `--until merged` with `--once`
  before observation begins.
- **AC10 — approval seam.** No `pr watch` mode acquires `PrArmer` or invokes
  `gh pr merge`. `pr land` still arms, verifies the arm, and watches to a
  terminal result; its success exit remains equivalent to `merged`.
- **AC11 — result consistency.** `CommandResult.ok`, `exit_code()`, the human
  summary, and serialized `pr.outcome` agree for at least `ready-to-land`,
  `merged`, `dequeued`, and `watcher-error`. The implementation cannot retain
  the old global assumption that only `merged` is successful.
- **AC12 — documentation consistency.** CLI help, `CONTRIBUTING.md`,
  `docs/ARCHITECTURE.md`, ADR guidance, `jaunder-ship`, and
  `jaunder-ship/base-moved.md` describe the same flow: default watch returns
  when caller action is required; approval precedes `pr land`; only explicit
  `--until merged` passively waits for another actor.
- **AC13 — offline verification.** Pure state-machine and virtual-clock tests
  cover all rows of the state table without network access, and the repository's
  required xtask validation gate passes.
- **AC14 — non-landable and indeterminate states are explicit.** Table-driven
  tests cover every D12 status in default, `--once`, and `--until merged` modes.
  `BLOCKED`, `DRAFT`, and `DIRTY` produce `blocked`; transient unknowns produce
  `pending { phase: "awaiting-mergeability" }` under `--once` and keep polling
  in blocking modes; `CLEAN`, `HAS_HOOKS`, `UNSTABLE`, and non-strict `BEHIND`
  can produce `ready-to-land` once every other predicate is satisfied. None of
  the blocked or unknown cases produces `ready-to-land`.

## Risks

- **Two commands now use different success meanings.** Accepted: `watch`
  succeeds when trustworthy observation reaches its intended handoff; `land`
  succeeds only when the approved merge completes. Command-specific result
  construction and regression tests make the distinction explicit.
- **`--until merged` can still spend the full timeout waiting for external
  approval.** This is deliberate and explicit in the flag; it is no longer the
  default agent path.
- **A manually dequeued PR first observed after the dequeue is indistinguishable
  from a never-queued ready PR.** `dequeued` requires transition history. The
  observer must not invent history it did not witness; current-head failed-run
  probing still detects an actual ejection from a single snapshot.
