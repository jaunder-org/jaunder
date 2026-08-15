# #1044 Actionable PR Watch Handoff Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make default `cargo xtask pr watch` return successfully at the
merge-approval handoff while retaining explicit passive observation and the
existing read-only/arming capability seam.

**Architecture:** Keep GitHub snapshot parsing unchanged. Deepen the existing
pure `decide` state machine with explicit ready, blocked, indeterminate, and
same-head dequeue states; keep stopping policy in the watch loop through
`WatchConfig`. Preserve `pr land` as the only `PrArmer` consumer and make result
success command-specific at the existing envelope seam.

**Tech Stack:** Rust 2024, clap, serde, the existing synchronous `gh` transport,
virtual-clock/scripted-source unit tests, cargo-nextest, and `cargo xtask`.

**Approved spec:**
[`docs/superpowers/specs/2026-08-14-issue-1044-pr-watch-ready.md`](../specs/2026-08-14-issue-1044-pr-watch-ready.md)

## Review

**Scope — in:**

- `pr watch` outcomes, phases, stopping policy, CLI flags, JSON/human result
  consistency, and offline tests.
- `pr land` empty-required-check refusal before arming.
- PR-observation documentation, ADR projection, and shipping-skill guidance.

**Scope — out:**

- GitHub query/transport changes, network tests, retries, re-running jobs,
  rebasing, arming from `watch`, or automatic re-enqueueing.
- Changes to the merge queue, required checks, CI workflows, or Jaunder domain
  vocabulary.

**Tasks:**

1. Implement and verify the complete command contract across the pure verdict,
   watch loop, CLI, result envelope, and land guard.
2. Align tracked documentation and agent guidance with the actionable handoff.

**Key risks/decisions:**

- Existing adverse verdicts must outrank `ready-to-land`; strict `BEHIND`
  remains `stale`.
- Queue history is head-scoped so a force-push cannot become a false `dequeued`
  verdict.
- `ready-to-land` is successful only for `pr-watch`; `pr-land` still succeeds
  only on `merged`.
- `--until merged` is the sole passive-wait opt-in and conflicts with `--once`.
- The ADR draft is gitignored. Its architecture projection must be present now,
  then `cargo xtask adr promote` must run during `jaunder-ship` before push so
  the tracked link is rewritten to the numbered ADR.

## Global Constraints

- `pr watch` remains read-only: no watch path acquires `PrArmer` or invokes
  `gh pr merge`.
- `pr land` remains the merge approval and the sole arming command.
- Required-check contexts remain ruleset-derived; an empty set fails closed as
  `watcher-error`.
- Existing API strike, rate-limit, timeout, heartbeat, change-only-event, and
  subject-resolution behavior remains unchanged.
- No new dependencies and no network-reaching tests.
- `xtask` is a separate workspace: test with
  `cargo nextest run --manifest-path xtask/Cargo.toml ...`, never `-p xtask`
  from the root workspace.
- Run commands through `devtool run --`; invoke pinned tools directly, never
  through `npx`, npm, or `nix develop`.
- Tick this plan's task checkbox before each commit gate. Stage, then commit; no
  `Co-Authored-By` trailer.

## File Structure

- `xtask/src/pr/types.rs` — wire outcome/phase vocabulary and outcome
  predicates.
- `xtask/src/pr/decide.rs` — pure GitHub-state classification, adverse
  precedence, landability, and head-scoped dequeue decision.
- `xtask/src/pr/watch.rs` — polling/stopping policy, ready phase, head-history
  maintenance, event emission, and virtual-clock tests.
- `xtask/src/pr/land.rs` — preserve `Step::Ready` as permission to proceed with
  approved arming; assert empty-gate refusal happens before `PrArmer`.
- `xtask/src/pr/execute.rs` — command-specific result-envelope success and
  structural no-arm tests.
- `xtask/src/pr/test_support.rs` — focused snapshot/ruleset fixtures used by the
  state, watch, and land tests.
- `xtask/src/lib.rs` — typed `--until merged` CLI surface, conflict with
  `--once`, help text, and `WatchConfig` construction.
- `xtask/src/result.rs` — only if needed to render the ready handoff clearly;
  retain merged-pointer labeling.
- `CONTRIBUTING.md` — authoritative human/agent command contract.
- `.agents/skills/jaunder-ship/SKILL.md` — git-excluded installed guidance for
  the pre-approval watch handoff and outcome actions.
- `.agents/skills/jaunder-ship/base-moved.md` — git-excluded installed recovery
  guidance.
- `docs/ARCHITECTURE.md` — materialized view of the stopping-policy ADR.
- `docs/adr/drafts/pr-watch-actionable-handoff.md` — numberless decision record,
  promoted only during `jaunder-ship`.
- `docs/superpowers/specs/2026-08-14-issue-1044-pr-watch-ready.md` — approved
  behavioral contract.
- `docs/superpowers/plans/2026-08-14-issue-1044-pr-watch-ready.md` — execution
  progress and implementation contract.

---

### Task 1: Implement the actionable observation contract

**Files:**

- Modify: `xtask/src/pr/types.rs`
- Modify: `xtask/src/pr/decide.rs`
- Modify: `xtask/src/pr/watch.rs`
- Modify: `xtask/src/pr/land.rs`
- Modify: `xtask/src/pr/execute.rs`
- Modify: `xtask/src/pr/test_support.rs`
- Modify: `xtask/src/lib.rs`
- Modify: `xtask/src/result.rs`
- Track with the implementation commit:
  `docs/superpowers/specs/2026-08-14-issue-1044-pr-watch-ready.md`
- Track with the implementation commit:
  `docs/superpowers/plans/2026-08-14-issue-1044-pr-watch-ready.md`

**Interfaces:**

- Extend `Outcome` with `ReadyToLand`, `Dequeued`, and `Blocked`; serialize
  through `Outcome::as_str()` as `ready-to-land`, `dequeued`, and `blocked`.
- Extend `Phase` with `AwaitingMergeability` and `ReadyToLand`; serialize
  through `Phase::as_str()` as `awaiting-mergeability` and `ready-to-land`.
- Replace invocation-wide `Progress::was_queued: bool` with
  `Progress::queued_head_sha: Option<String>` (or an equivalent private
  representation) whose invariant is: it identifies only the head observed in
  the queue and is cleared before classifying a different head.
- Extend `Step` with a non-terminal-to-the-PR ready classification, e.g.
  `Step::Ready`; `decide::classify` stays independent of `--once` / `--until`
  stopping policy.
- Extend `WatchConfig` with `stop_at_ready: bool`, default `true`;
  `--until merged` and `pr land` set it to `false`.
- Add a typed clap value for `--until merged`; the `Watch` variant carries
  `until: Option<PrWatchUntil>` and conflicts with `once`.
- `pr::into_result(command: &str, report: PrReport) -> CommandResult` remains
  the envelope seam; success is `Merged | ReadyToLand` for `pr-watch`, only
  `Merged` for `pr-land`.

- [x] **Step 1: Write every failing contract test before changing shared enums**

Write all red tests first so the next implementation step can update every
exhaustive consumer in one compilable slice.

In `xtask/src/pr/decide.rs`, add:

```rust
#[test]
fn green_landable_unarmed_pr_is_ready_to_land() {
    assert!(matches!(
        classify(&open(green()), &queue_rules(), None, &Progress::default()),
        Step::Ready
    ));
}

#[test]
fn strict_behind_outranks_ready_to_land() {
    let mut snap = open(green());
    snap.merge_state_status = MergeStateStatus::Behind;
    assert!(matches!(
        classify(&snap, &strict_rules(), None, &Progress::default()),
        Step::Terminal { outcome: Outcome::Stale, .. }
    ));
}

#[test]
fn empty_required_set_is_watcher_error() {
    let empty = RequiredChecks {
        contexts: Vec::new(),
        strict: false,
        queue_present: true,
    };
    assert!(matches!(
        classify(&open(green()), &empty, None, &Progress::default()),
        Step::Terminal { outcome: Outcome::WatcherError, .. }
    ));
}
```

Add the full pure merge-state matrix for an open, green, unarmed, unqueued
snapshot:

- `Clean`, `HasHooks`, `Unstable`, and `Behind` under `queue_rules()` →
  `Step::Ready`.
- `Blocked`, `Draft`, and defensive `Dirty` → terminal `Outcome::Blocked`, with
  `detail` naming the exact GitHub status.
- `Mergeable::Unknown` or `MergeStateStatus::Unknown` →
  `Step::Continue { phase: Phase::AwaitingMergeability, .. }` unless a decisive
  adverse state already applies.
- `Mergeable::Conflicting` → existing `Outcome::Conflicted`.
- Failed required check → existing `Outcome::ChecksFailed`.

Add history tests using `Progress { queued_head_sha: Some("abc".into()) }`:

- unqueued head `abc`, no ejection → `Outcome::Dequeued`, with detail containing
  both “queue entry vanished” and “no failed current-head merge-group run”;
- unqueued head `def`, no ejection → not `Dequeued`;
- same-head failed current ejection run → existing `Outcome::Ejected`, proving
  ejection precedence.

Update `needs_ejection_probe` tests so an empty required set never triggers the
second API call. Replace every obsolete green/unarmed `Step::Continue`
expectation rather than leaving contradictory tests:

- `green_and_unqueued_warns_that_nothing_will_happen`;
- `vanished_queue_entry_with_no_run_warns_and_continues`;
- `all_required_green_is_not_terminal_when_a_queue_exists`;
- `failing_non_required_check_does_not_fail_the_pr`;
- `duplicate_context_resolves_to_the_latest_completion`;
- the non-strict arm of `behind_is_stale_only_when_the_ruleset_is_strict`;
- `stale_merge_group_run_older_than_head_is_not_ejected`;
- `successful_merge_group_run_is_not_an_ejection`.

Where the test's real contract is check resolution, strictness, or ejection
precedence, assert `Step::Ready` instead of deleting the test.

In `xtask/src/pr/watch.rs`, add:

1. Default config + `open(green())` → `ReadyToLand`, no phase, ready detail,
   head SHA `abc`, and clock `0`.
2. `once: true` + the same snapshot → the same `ReadyToLand`, not `Pending`.
3. `stop_at_ready: false` + snapshots
   `[open(green()), armed_snapshot(), queued_at(2), merged_snapshot()]` →
   `Merged`, with a phase event `ready-to-land`.
4. `stop_at_ready: false` + permanently green/unarmed snapshot + short timeout →
   existing `TimedOut`.
5. `[queued_at(2), open(green())]`, no ejection → `Dequeued`; detail contains
   the evidence text above and clock equals exactly one polling interval,
   proving no sleep after the verdict.
6. `[queued head abc, unqueued green head def, merged_snapshot()]` → `Merged`,
   never `Dequeued`; set the second snapshot's head SHA and commit timestamp to
   `def` and terminate deterministically on the third.
7. Empty required set in default, once, and continue-through-ready configs →
   `WatcherError` with clock `0`.

Add one table-driven watch matrix for every D12 status in all three modes:

- Modes: default (`stop_at_ready: true`), once, and passive
  (`stop_at_ready: false`).
- Blocked statuses: `Blocked`, `Draft`, `Dirty` → `Blocked` immediately in every
  mode.
- Unknown dimensions: `Mergeable::Unknown` and `MergeStateStatus::Unknown` →
  `Pending { phase: "awaiting-mergeability" }` under once; for both blocking
  modes append `merged_snapshot()` and assert the `awaiting-mergeability` phase
  event before `Merged`.
- Landable statuses: `Clean`, `HasHooks`, `Unstable`, non-strict `Behind` →
  `ReadyToLand` under default and once; under passive mode append
  `merged_snapshot()` and assert the `ready-to-land` phase before `Merged`.

In `xtask/src/lib.rs`, extend parser tests:

```rust
#[test]
fn pr_watch_parses_until_merged() {
    let cli = Cli::try_parse_from([
        "xtask", "pr", "watch", "731", "--until", "merged"
    ]).unwrap();
    // Destructure and assert number 731 plus the typed Merged value.
}

#[test]
fn pr_watch_rejects_once_with_until_merged() {
    assert!(Cli::try_parse_from([
        "xtask", "pr", "watch", "--once", "--until", "merged"
    ]).is_err());
}

#[test]
fn pr_watch_rejects_unknown_until_value() {
    assert!(Cli::try_parse_from([
        "xtask", "pr", "watch", "--until", "ready"
    ]).is_err());
}
```

Update existing parser destructuring to assert `until.is_none()` unless set.

In `xtask/src/pr/execute.rs`, specify envelope results:

- `ReadyToLand` under `pr-watch` → `ok`, exit 0, one passing step, JSON
  `ready-to-land`.
- `ReadyToLand` under `pr-land` → fail/exit 1 defensively.
- `Merged` under both commands → success.
- `Dequeued`, `Blocked`, `WatcherError`, and every other non-success outcome
  under both commands → fail/exit 1.
- All new outcomes serialize to their exact kebab-case spellings.

In `xtask/src/result.rs`, add a pure
`render_pr_summary(pr: &PrReport) -> String`; make `print_human` print that
string. Test `ReadyToLand`, `Merged`, `Dequeued`, and `WatcherError` summaries:
each includes the outcome and head SHA when present; ready includes the approval
handoff detail; merged retains the `merge commit` label. Pair these assertions
with the envelope assertions above to satisfy AC11 across human, JSON, `ok`, and
exit status.

In `xtask/src/pr/land.rs`, add:

- empty rules + open snapshot → `WatcherError`, armer calls `0`;
- snapshots
  `[ready initial, armed after arm verification, ready after the arm disappears, merged]`
  → `Merged`, not `ReadyToLand`, proving land continues through ready after
  approval.

- [x] **Step 2: Run the complete contract test set and verify red**

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml -E 'test(/pr::/) | test(/tests::pr_/) | test(/result::tests/)'
```

Expected: FAIL to compile because the new variants, history representation,
watch policy, typed flag, and renderer do not exist. This is one intentional red
slice; do not try to make only `decide` green while exhaustive consumers still
use the old vocabulary.

- [x] **Step 3: Implement the complete code contract as one compilable slice**

In `types.rs`, add `ReadyToLand`, `Dequeued`, and `Blocked` plus exact wire
spellings. Retain `Outcome::is_merged()` for merge-pointer rendering and land
success.

In `decide.rs`:

1. Add `Phase::{AwaitingMergeability, ReadyToLand}` and `Step::Ready`.
2. Replace `Progress::was_queued` with
   `Progress::queued_head_sha: Option<String>`.
3. Preserve adverse ordering: merged, closed, conflict, failed required check,
   strict-behind stale, current-head ejection.
4. Return `WatcherError` for an open PR with an empty required set.
5. Continue queued/armed snapshots before unarmed-ready merge-state evaluation.
6. Return `Dequeued` only when queue history names the same head and ejection
   evidence has already been ruled out; use the evidence detail pinned in Step
   1.
7. Continue `AwaitingChecks` until all required contexts are green, then
   classify D12's blocked, unknown, and landable sets exactly.
8. Make `needs_ejection_probe` require a non-empty required set.

In `watch.rs`:

- Add `stop_at_ready: true` to `WatchConfig::default()`.
- Before classification, clear queued history when it names a different head;
  after classification/event construction, record the current SHA only when the
  snapshot is queued.
- Map `Step::Ready` to `Phase::ReadyToLand`. With `stop_at_ready`, emit one
  terminal event and return `ReadyToLand` with detail “all required checks
  passed; obtain approval, then run `pr land`” without sleeping. Otherwise emit
  and change-detect the phase and continue; heartbeats say
  `still ready-to-land`.
- Handle ready before the once fallback: ready returns `ReadyToLand`, while
  other continuation states return `Pending { phase }`.
- Preserve change-only events and terminal-event suppression.

In `lib.rs`:

- Import `clap::ValueEnum`; add `PrWatchUntil { Merged }`.
- Add `until: Option<PrWatchUntil>` to `PrCommand::Watch` with
  `#[arg(long, value_name = "OUTCOME", conflicts_with = "once")]`.
- Document default actionable stopping and explicit passive waiting.
- Default watch sets `stop_at_ready: true`; `--until merged` and `pr land` set
  `stop_at_ready: false`. Land's false setting is load-bearing: after approval,
  no ready snapshot may return before a terminal PR outcome.

In `land.rs`, treat `Step::Ready` as permission to proceed with approved arming,
and delegate to `watch` with the continue-through-ready config. Empty-gate
`WatcherError` returns before `PrArmer`.

In `execute.rs`, keep exactly one step and compute success without a new generic
abstraction:

```rust
match (command, report.outcome) {
    ("pr-watch", Outcome::Merged | Outcome::ReadyToLand) => true,
    ("pr-land", Outcome::Merged) => true,
    _ => false,
}
```

In `result.rs`, route existing PR output through the pure summary renderer. Do
not alter audit/traces output or merged pointer labeling.

Update `test_support.rs` only with small fixtures needed by the matrix/history
tests; do not add a parallel state abstraction.

- [x] **Step 4: Run the targeted contract tests and verify green**

Run the Step 2 command. Expected: PASS. Specifically confirm:

- every D12 status × all three watch modes;
- same-head dequeue detail and clock, plus new-head reset;
- ready human/JSON/head/exit consistency;
- clap conflict/value parsing;
- land empty-gate no-arm and ready-after-arm continuation.

- [x] **Step 5: Run all xtask tests and smoke the CLI help**

Run:

```bash
devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml
devtool run -- cargo xtask pr watch --help
```

Expected: all xtask tests PASS. Help names `--until <OUTCOME>`, its only value
`merged`, the conflict with `--once`, the default ready/action-required handoff,
and the read-only guarantee.

- [x] **Step 6: Tick Task 1, run the commit gate, and commit**

Tick this task before the gate. Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS. Inspect and stage formatter changes made by fix mode. Stage the
Rust changes, approved spec, and this checked plan; do not stage unrelated work.
Commit:

```text
fix(xtask): stop PR watch at actionable states (#1044)
```

No `Co-Authored-By` trailer.

---

### Task 2: Align PR-observation documentation and skills

**Files:**

- Modify: `CONTRIBUTING.md:451-479`
- Modify: `.agents/skills/jaunder-ship/SKILL.md:51-110`
- Modify: `.agents/skills/jaunder-ship/base-moved.md:118-124`
- Modify: `docs/ARCHITECTURE.md:2350-2364`
- Modify: `docs/adr/drafts/pr-watch-actionable-handoff.md`
- Modify: `docs/superpowers/plans/2026-08-14-issue-1044-pr-watch-ready.md`

**Interfaces:**

- Human workflow: `pr watch` → `ready-to-land` → explicit approval halt →
  `pr land` → terminal queue outcome.
- Passive workflow: `pr watch --until merged`; it remains read-only and may
  consume the full timeout while awaiting another actor.
- Outcome handling adds `ready-to-land`, `dequeued`, and `blocked`;
  `watcher-error` still means the tool could not establish a trustworthy
  verdict.
- ADR projection cites `docs/adr/drafts/pr-watch-actionable-handoff.md` by
  descriptive link text; `jaunder-ship` later promotes and rewrites it before
  push.

- [x] **Step 1: Update the authoritative contributor documentation**

Rewrite `CONTRIBUTING.md`'s PR-watching section so it states:

- default watch waits through required checks and returns at the next actionable
  outcome;
- `ready-to-land` is exit 0 for `pr watch` and is the approval handoff;
- `pr land` is still the approval-bearing action and exit 0 only when merged;
- `--until merged` is explicit passive observation across another actor's
  approval;
- `--once` is one snapshot;
- the full outcome list includes `ready-to-land`, `dequeued`, `blocked`, and
  existing outcomes;
- machine callers branch on `pr.outcome`, because exit 1 still contains multiple
  distinct adverse/actionable meanings.

Use examples for default, `--once`, `--until merged`, and `pr land`.

- [x] **Step 2: Update shipping and recovery skills**

In `jaunder-ship/SKILL.md`, make step 7 finite at `ready-to-land`:

- run default `pr watch` autonomously through CI;
- handle red/adverse outcomes as today, adding `blocked` and `dequeued` actions;
- on `ready-to-land`, report the ready PR and halt for the existing step-8
  approval;
- do not run `pr land` until the user approves this PR;
- reserve `--until merged` for deliberately passive observation by a different
  actor, not the ordinary shipping flow;
- retain the `--once` babysit sweep and “never merge” instruction.

In `base-moved.md`, state that recovery confirmation returns at the next
actionable state by default. Remove the claim that default watch necessarily
waits to a terminal PR outcome; mention `--until merged` only as explicit
passive mode.

- [x] **Step 3: Finalize the ADR draft and architecture projection**

Verify the draft records:

- next-action stopping rather than terminal-only stopping;
- command-specific success semantics;
- `dequeued` as same-head transition evidence;
- `blocked` versus indeterminate mergeability;
- empty-gate refusal in watch and land;
- unchanged observer/armer seam.

Verify `docs/ARCHITECTURE.md` states current truth and cites the draft as:

```markdown
[the actionable-handoff decision](adr/drafts/pr-watch-actionable-handoff.md)
```

Do not edit `docs/README.md`; `cargo xtask adr promote` owns its generated row
at ship. Do not number or stage the gitignored draft manually.

- [x] **Step 4: Format and verify documentation**

Run:

```bash
devtool run -- prettier -w CONTRIBUTING.md .agents/skills/jaunder-ship/SKILL.md .agents/skills/jaunder-ship/base-moved.md docs/ARCHITECTURE.md docs/adr/drafts/pr-watch-actionable-handoff.md docs/superpowers/specs/2026-08-14-issue-1044-pr-watch-ready.md docs/superpowers/plans/2026-08-14-issue-1044-pr-watch-ready.md
devtool run -- cargo xtask pr watch --help
devtool run -- cargo xtask check
```

Expected: Prettier reports all named Markdown files formatted; help and prose
agree; the full commit gate passes. Inspect and stage any fix-mode changes.

- [x] **Step 5: Tick Task 2 and commit tracked documentation**

Stage `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, and this checked plan. The ADR
draft and installed `.agents/` skill copies remain git-excluded; the draft is
intentionally promoted during `jaunder-ship`. Commit:

```text
docs: describe actionable PR watch handoff (#1044)
```

No `Co-Authored-By` trailer. Before any push, `jaunder-ship` must rebase and run
`devtool run -- cargo xtask adr promote`, stage the promoted ADR/reference/table
rewrites, and commit them so no tracked draft link reaches a clean clone.

## Self-Review

- Every spec criterion AC1–AC11 and AC13–AC14 maps to Task 1; documentation AC12
  maps to Task 2.
- No separable concern surfaced: the verdict, loop policy, CLI, result envelope,
  land guard, and docs are one command-interface change.
- New names are consistent across tasks: `ReadyToLand` / `ready-to-land`,
  `Dequeued` / `dequeued`, `Blocked` / `blocked`, `AwaitingMergeability` /
  `awaiting-mergeability`, and `--until merged`.
- No placeholder steps, deferred implementation, new dependency, network test,
  or write capability in `pr watch`.
