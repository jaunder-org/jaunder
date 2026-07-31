# Spec — `cargo xtask pr watch` / `pr land` (issue #729)

**Issue:** [#729](https://github.com/jaunder-org/jaunder/issues/729) — "xtask:
one PR checks/merge-queue watcher, instead of re-deriving it per session"
**Date:** 2026-07-30 **Branch:** `worktree-issue-729-xtask-pr-watch`

## Problem

Every agent that ships a PR re-derives the same green→queue→merged watcher and
gets it wrong in the same ways (in one #671 session, four times). The correct
behaviour exists only as prose across `jaunder-ship` step 7,
`docs/ci-merge-queue.md`, and agent memory — and prose gets re-implemented,
which is where the bugs live. This spec replaces the prose with a command.

## Verified environment facts

These were checked against the live repo/API during the design interview, not
assumed. They are recorded because several contradict the issue body.

| #   | Fact                                                                                                                                                                                                                                                                                                                                                 | Source                                                             |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| F1  | The merge queue **is live** and `strict_required_status_checks_policy` is **`false`**                                                                                                                                                                                                                                                                | `gh api /repos/jaunder-org/jaunder/rules/branches/main`            |
| F2  | The same endpoint returns the required contexts (`Validate (no e2e)`, `e2e gate`), the strict flag, **and** whether a `merge_queue` rule exists — one read, all three                                                                                                                                                                                | ditto                                                              |
| F3  | The live `merge_queue` rule has **no requeue parameter** (`merge_method`, `grouping_strategy`, `max/min_entries_*`, `check_response_timeout_minutes` is the complete set). **Ejection is terminal until someone re-enqueues.** `docs/ci-merge-queue.md:190` claims ejected PRs are "auto-requeued" — that is **wrong** and is corrected by this work | ditto                                                              |
| F4  | GraphQL `PullRequest` exposes `mergeQueueEntry`, `isInMergeQueue`, `autoMergeRequest`, `mergeStateStatus`, `mergeable`, `statusCheckRollup`, `mergeCommit`, `mergedAt` — the whole state machine is one document                                                                                                                                     | `__type(name:"PullRequest")` introspection                         |
| F5  | `gh api` collapses every failure into **exit 1**. A 404 exits 1 with `{"message":"Not Found",…,"status":"404"}` on **stdout**; a GraphQL schema error exits 1 with `gh: Field '…' doesn't exist` on **stderr**. The discriminating information is always in the body, never the exit code                                                            | probed directly                                                    |
| F6  | **`gh` is not in the flake devShell.** `ciInputs` and `devOnly` (`flake.nix:1284–1320`) do not list it; it has been ambient on the developer's machine                                                                                                                                                                                               | `flake.nix`                                                        |
| F7  | `octocrab = "0.54"` alone resolves to **211 crates** (xtask's current lockfile: **93**), adding `tokio`, `hyper`, `rustls`, `ring`. Its **only** `merge_queue` match is an unrelated webhook payload struct; its GraphQL surface is `graphql<R: DeserializeOwned>(body)` (`lib.rs:1456`) — hand-written query, self-defined structs                  | measured lockfile + vendored source                                |
| F8  | xtask is **excluded from the Nix coverage derivation's source**, so `xtask/` is not coverage-measured; its tests run in the host suite via `steps::host_tests`                                                                                                                                                                                       | `CLAUDE.md` invariant, flake source filter                         |
| F9  | `CommandResult::exit_code()` is binary (`result.rs:95`); `main.rs:28` uses `2` for the `Err` path. `CommandResult::push()` (`result.rs:90–93`) **recomputes `ok` from the step vector on every push**                                                                                                                                                | `xtask/src/result.rs`, `xtask/src/main.rs`                         |
| F10 | **`.claude/` and `CLAUDE.md` are entirely untracked** — `git ls-files` returns nothing for either, and neither exists inside this worktree; both live only at the main checkout. (`CLAUDE.md` was assumed tracked when this spec was written; corrected during Task 9.) | `git ls-files` |
| F11 | Merge-group runs are named `gh-readonly-queue/main/pr-<N>-<BASE_sha>` — the suffix is the **base** SHA (previous `main` tip), **not** the PR head. `?branch=` needs an exact name, so prefix matching requires `?event=merge_group`                                                                                                                  | `gh api /repos/jaunder-org/jaunder/actions/runs?event=merge_group` |

## Decisions

### D1 — Two subcommands; the tool turns the crank, never makes the call

`cargo xtask pr watch [N]` **observes only**. `cargo xtask pr land [N]`
**acts**, and typing it _is_ the merge approval — the human halt is structural,
not a prompt.

Actions the tool owns (zero judgment):

1. Poll until a terminal state.
2. Re-arm auto-merge when the arm silently no-opped.
3. Confirm `MERGED` and capture the merge commit.

Actions the tool refuses (judgment required), reporting the state so a human
decides:

4. Re-running a red job — "flake or real?" is a call that must not be automated.
5. Rebasing / force-pushing — and moot under F1 anyway.
6. Re-enqueueing after ejection — requeuing a genuinely-failing PR loops
   forever.
7. The initial arm decision — that is the approval itself.

### D2 — `gh` is the transport; no Rust GitHub client

`pr watch`/`pr land` shell out to `gh api` and `gh api graphql`. Rationale is
F7: for precisely the fields this command needs, octocrab supplies **no** models
and requires the same hand-written GraphQL and the same self-defined structs —
at 211 crates and an async runtime in a synchronous CLI that rebuilds from the
working tree on every invocation.

`pkgs.gh` is added to the devShell's **`devOnly`** list, not `ciInputs`: these
are host-only manual commands, never run by a Nix check or a CI job (same status
as `traces analyze`).

### D3 — Layered module, one file that knows about shells

`xtask/src/pr/`, following the house pattern (`traces/`: parse → analyze →
render → run; `server_fn_coverage/`: io → extract → snapshot):

| File          | Job                                                                                                                                                                                       | Knows `gh`?             | Does IO?      |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- | ------------- |
| `mod.rs`      | Module wiring + the `PrReport` / outcome types the envelope serializes                                                                                                                    | no                      | no            |
| `gh.rs`       | Run `gh api`/`gh api graphql`; classify failures into a typed `ApiError` (`GhMissing`, `Unauthenticated`, `NotFound`, `RateLimited { reset }`, `Transport`, `Malformed`, `GraphQlErrors`) | **only file that does** | yes           |
| `snapshot.rs` | One GraphQL document + REST reads → typed `PrSnapshot` and `RequiredChecks`. Serde structs, no logic                                                                                      | no                      | via the trait |
| `decide.rs`   | **Pure.** `classify(&PrSnapshot, &RequiredChecks, &Progress) -> Step`. Every rule lives here and nowhere else                                                                             | no                      | **no**        |
| `watch.rs`    | The poll loop, generic over the source trait **and a `Clock`**                                                                                                                            | no                      | via traits    |
| `land.rs`     | Subject validation (divergence guard, via the existing `xtask/src/git.rs`), arming prologue, then delegates to `watch`                                                                    | no                      | git only      |

The seam is **domain-shaped**, not transport-shaped — above `gh.rs` nothing sees
a string, a JSON blob, or an exit code:

```rust
trait PrSource {
    /// Resolve the subject: the repo, and the PR number when it was omitted (D13).
    fn resolve(&self, requested: Option<PrNumber>) -> Result<Subject, ApiError>;
    fn snapshot(&self, subject: &Subject) -> Result<PrSnapshot, ApiError>;
    fn required_checks(&self, subject: &Subject) -> Result<RequiredChecks, ApiError>;
    fn ejection_run(&self, subject: &Subject) -> Result<Option<RunRef>, ApiError>;
}
```

`PrSnapshot` carries: `state`, `merged_at`, `merge_commit`, `mergeable`,
`merge_state_status`, `auto_merge_armed`, `is_in_merge_queue`,
`queue: QueueState` (including position), `head_sha`, **`head_committed_at`**
(needed by D10), and `checks: Vec<CheckEntry>` where `CheckEntry` is
`{ name, state, details_url, completed_at, started_at }` flattened from the
`CheckRun` / `StatusContext` union.

`gh.rs` further splits **running** from **classifying**:
`classify(exit, stdout, stderr) -> Result<Value, ApiError>` is pure and
table-testable; the subprocess wrapper around it is a handful of lines.

`decide` is pure but needs a little history — "the queue entry vanished" is a
transition. That is the `&Progress` argument (was-queued, last-emitted
fingerprint, consecutive-error count): the loop owns the mutation, `decide`
reads it and returns the next one.

### D4 — Outcome set

Eight variants, one conditional, plus a `--once`-only non-terminal:

| Outcome             | Meaning                                                                                                         |
| ------------------- | --------------------------------------------------------------------------------------------------------------- |
| `merged`            | Landed; carries merge commit + timestamp                                                                        |
| `checks-failed`     | A **required** check concluded failure; carries which, plus its `detailsUrl`                                    |
| `ejected`           | Front-of-queue `merge_group` failed; carries the `gh-readonly-queue/main/pr-<N>-…` run                          |
| `conflicted`        | `mergeable: CONFLICTING` — permanently blocked, needs a human rebase                                            |
| `closed-unmerged`   | PR closed without merging                                                                                       |
| `stale`             | `BEHIND` **and blocking** — reachable **only when the observed ruleset is strict** (D6)                         |
| `timed-out`         | Budget expired with no terminal state. Distinct from `watcher-error`: the tooling worked, GitHub never finished |
| `watcher-error`     | The API/tooling failed persistently. Never silence, never a false negative                                      |
| `pending { phase }` | **`--once` only.** Unreachable in blocking mode                                                                 |

`conflicted` and `timed-out` are **additions** to the issue's list; `stale` is
**retained but made conditional** (see D6). Rationale for `timed-out` being
separate: an agent branches differently on "don't trust this answer" versus "go
see why the queue is stuck," and conflating them recreates the "three meanings,
one signal" defect the issue exists to kill.

### D5 — Exit codes stay 0/1; the outcome rides the JSON

**0 = `merged`. 1 = every other terminal outcome (and `pending`). 2 = xtask
could not produce a report at all** (bad arguments). Encoding outcomes in exit
codes is a memorability tax; the machine-readable branch signal is `pr.outcome`
in `--json` and `.xtask/last-result.json`.

This imposes a hard requirement, because it is where the design could quietly
reintroduce the bug: **every terminal outcome returns `Ok(CommandResult)`
carrying a `PrReport` — never `Err`.** If `watcher-error` propagated as `Err`,
`main.rs` would exit 2 and never write the sidecar (F9), so the outcome that
most needs to be legible would be the one with no JSON. Concretely: `gh`
missing, `gh` unauthenticated, and sustained rate-limiting are all
`watcher-error` **reports**.

`CommandResult` gains `pr: Option<PrReport>` alongside `audit`/`traces`/`flaky`.
**`exit_code()` is not changed at all** — see the `ok` invariant below, which
makes the existing binary mapping already correct. No `exit_override` escape
hatch on the shared type, and the `xtask-done: … exit=N` sentinel carries the
code out unchanged.

**Keeping `ok` in sync with the outcome.** Per F9, `push()` recomputes `ok` from
the step vector, so simply asserting `ok == (outcome == Merged)` would not
survive a passing step being pushed — `xtask-done: ok=true exit=1` and a sidecar
reading `"ok": true` for a `checks-failed` run, which is precisely what agents
branch on (CLAUDE.md). Therefore the `pr` commands push **exactly one**
`StepResult`, named `pr-watch`/`pr-land`, constructed `ok` iff the outcome is
`merged` and `fail` otherwise, with the outcome as its detail. `push()`'s
recomputation then agrees with `exit_code()` by construction rather than by
convention — which is precisely why neither `push()` nor `exit_code()` needs to
change. The invariant is load-bearing, so A16 pins it: pushing a second step
would silently break it.

**Human rendering.** `PrReport` renders a short summary block in `print_human()`
alongside the existing `audit`/`traces` informational payloads: the outcome, the
PR and head SHA, the outcome-specific pointer (merge commit / failing check URL
/ ejection run), and the elapsed time. `produces_json_payload()` stays `true`,
so `--json` works without a special case.

### D6 — The gate shape is data, read from the ruleset

Per F2, one read of `/repos/{owner}/{repo}/rules/branches/main` yields the
required contexts, `strict_required_status_checks_policy`, and whether a
`merge_queue` rule exists. The state machine branches on the **observed**
ruleset:

- strict on → `BEHIND` is terminal `stale`; strict off → `BEHIND` is not
  blocking.
- queue present → the enqueue/ejection phase exists at all.

So "required checks as data" generalizes to "the whole gate shape is data," and
the rollback documented in `docs/ci-merge-queue.md` (trigger: #629 OOM ejections
thrashing the queue) needs no code change.

**Check evaluation is scoped to failure, and green is never terminal on its
own.** Precisely:

- `checks-failed` fires as soon as **any required** context concludes failure.
- **"Every required context has concluded successfully" is not a terminal
  state.** When a `merge_queue` rule is present it is the _precondition for
  phase two_ — the PR must then be armed/enqueued and survive a merge-group run
  against live `main`. This is the issue's point 2: declaring victory at green
  checks is wrong. Only when the observed ruleset has **no** queue rule does
  green-plus-mergeable lead directly toward `merged`.
- A required context that has **not appeared at all** cannot satisfy anything —
  it is neither a failure nor a success — which is what makes the late-appearing
  `e2e gate` (the issue's point 5) safe. The loop never evaluates "no check is
  pending."
- A **non-required** check that fails never produces `checks-failed`.

**Matching a ruleset context to a rollup entry.** The ruleset yields strings
(`{"context":"Validate (no e2e)"}`); `statusCheckRollup.contexts` is a union of
`CheckRun` (`name`, `conclusion`) and `StatusContext` (`context`, `state`). A
required context matches an entry whose `CheckRun.name` **or**
`StatusContext.context` equals it exactly. When several entries share a name
(re-runs), the one with the **latest `completedAt`** wins; if any is still
incomplete, ties break to the latest `startedAt` and the context counts as
not-yet-concluded. This rule is load-bearing — per A8 no check name is
hardcoded, so it is the only thing joining the ruleset to the verdict.

### D7 — Timing

- **Poll interval 30s** — `--interval <SECONDS>`, integer, `value_parser` range
  `5..` (matching the `--top` precedent at `lib.rs:244`). ~180 calls over a
  90-minute watch against a 5000/hr budget, counting D10's conditional second
  call.
- **Overall budget 90 min** — `--timeout <MINUTES>`, integer, range `1..`.
  Derived: ~25 min cold e2e on the PR head +
  `min_entries_to_merge_wait_minutes: 5` + a full re-run against `main` in the
  queue, against the queue's own `check_response_timeout_minutes: 60`.
- **5 consecutive transient failures with exponential backoff →
  `watcher-error`.**
- **Rate-limiting is not a transient failure.** GitHub reports the reset. If the
  reset falls inside the remaining budget, sleep until it (emitting a
  `rate limited until HH:MM` event so silence never reads as progress) and
  continue; if it does not, report `watcher-error` immediately rather than
  burning the budget. Counting it as one of the 5 strikes would abandon after
  ~2.5 minutes a condition known to clear in, say, 12.

### D8 — One event log, two renderings

There is **one** event log. It is rendered live to **stderr** as events occur
and serialized into `pr.events[]` in the final report, so an agent reading
`--json` gets the same timeline, including heartbeats and every absorbed
transient failure, that a human watching live sees.

An event is emitted exactly when the **fingerprint** changes:

- phase (`awaiting-checks` / `armed` / `queued` / `terminal`),
- the sorted set of `(required check, status/conclusion)` pairs,
- queue state **including position** (3 → 2 is real progress, worth one line),
- the armed flag.

Deliberately **excluded** from the fingerprint: elapsed time, poll count,
`updatedAt` — anything that ticks on its own. Those are what turn a
change-emitter into a per-poll emitter (the second #671 bug).

Two additions the fingerprint cannot supply:

- **Heartbeat**: 10 minutes of stasis emits one
  `still <phase>, no change for 10m`. At most nine over a full budget; it is
  what stops a wedged queue from being indistinguishable from a dead process.
- **Errors always emit**, immediately: `poll failed (2/5): <reason>`. A silently
  absorbed API failure looking exactly like "nothing changed" was the **first**
  #671 bug.

`.xtask/last-result.json` is **not** rewritten incrementally — a partial report
that looks terminal is the false-signal class this issue exists to kill.
Mid-flight observation is the parked `.xtask/run/<id>.err` that `devtool run`
writes incrementally.

### D9 — The `land` prologue

1. **Snapshot first.** Already `conflicted` / `closed-unmerged` /
   `checks-failed` / `merged` → report it and do nothing. Arming a PR that
   cannot merge produces a misleading "armed, waiting."
2. **Arm with `gh pr merge --auto --merge`** uniformly — correct whether checks
   are pending or already green.
3. **Verify with the honest predicate** on the next snapshot:
   `autoMergeRequest.enabledAt.is_some() || is_in_merge_queue`. The issue names
   only `autoMergeRequest.enabledAt`, which would misreport a **direct enqueue**
   (green PR + live queue ⇒ `autoMergeRequest` stays null, `isInMergeQueue`
   becomes true) as a failed arm. Never trust `gh`'s exit code or stdout here
   (the issue's point 4).
4. **Re-arm once** if neither holds.
5. **Still not armed → re-snapshot and classify**, rather than inventing an
   outcome: a refused arm almost always has a reason already in the vocabulary.
   Only if nothing is blocking and the arm still will not stick is that a
   genuine contradiction → `watcher-error` carrying the raw response.
6. **Then run the identical watch loop** to `merged`.

**Divergence guard:** if `land` is invoked from a worktree whose HEAD branch is
the PR's head ref and the local commit differs from the PR head SHA, **refuse**
— what would merge is not what you are looking at. Invoked from anywhere else,
`land` is location-agnostic and simply emits the PR head SHA as its first event.

The refusal is **subject validation, not an outcome**: it happens before any
watching, so per D13 it exits **2** with a message naming both SHAs, and
produces no `PrReport`. It is not `watcher-error` — nothing failed; the request
was refused. The git reads (current branch, local HEAD SHA) live in `land.rs`
through the existing `xtask/src/git.rs`, not behind `PrSource`, which is
GitHub-only.

**`pr land --once` is rejected at parse time.** "Arm the merge and immediately
stop watching" is never intended, and leaving it legal means someone eventually
arms a merge and walks away believing they watched it.

### D10 — Ejection is detected from state, not from having been watching

A single snapshot cannot distinguish ejected from never-enqueued, and a `watch`
started a minute after an ejection has no transition left to see. So the
**`gh-readonly-queue/main/pr-<N>-…` workflow run is the primary evidence** and
the transition merely corroborates:

- **Query:**
  `/repos/{owner}/{repo}/actions/runs?event=merge_group&per_page=100`, first
  page only. Per F11 the suffix of `gh-readonly-queue/main/pr-<N>-<sha>` is the
  **base** SHA, not the head, so `?branch=` (which needs an exact name) cannot
  be used and the match is a **prefix test on `head_branch`** against
  `gh-readonly-queue/main/pr-<N>-`.
- **Discriminator:** among matching runs take the most recent by `created_at`.
  It is the ejection iff it `concluded: failure` **and** its `created_at` is
  later than the PR head commit's `head_committed_at` (D3). The timestamp
  comparison is required because F11 means recency cannot be read off the branch
  name; `committedDate` is the right anchor because git refreshes it on rebase
  and amend, so a re-pushed head reliably post-dates a stale merge-group run.
  Getting this wrong reports a **false `ejected` on a freshly pushed head**, so
  it is covered by a fixture test.
- **Query trigger:** only when the PR is `OPEN`, **all required contexts have
  concluded successfully**, and it is not queued. Ejection presupposes having
  been enqueued, which presupposes green checks — so during the long pre-green
  phase (D7's ~25 min of cold e2e) this never fires. It does fire on every poll
  in the green-but-unqueued state, which is the state that needs it.
- This depends on GitHub's branch-naming convention, which we do not control.
  Guarded the same way as the check set: a PR observed queued and then not
  queued with **no** matching run emits a loud
  `expected a merge-group run for PR N, found none` rather than quietly
  concluding "not ejected."

**Green, unqueued, and unarmed is a real resting state.** `watch` never arms, so
a green PR nobody enqueued sits there until the budget expires and reports
`timed-out`. That is correct — nothing is wrong with the PR, and `watch` is
forbidden from acting (D1) — but it is the most likely first surprise, so
entering that state emits an explicit
`green and unqueued — nothing will happen until \`pr land\`` event rather than a
silent wait punctuated only by heartbeats.

**Manual dequeue folds back into the loop** (phase reverts to `awaiting-checks`,
loud event, eventual `timed-out` if nothing follows) rather than terminating.
Rationale: a manual dequeue is usually followed by a manual re-enqueue, so
terminating would abandon the watch exactly when it remains useful.

### D11 — Test surface

Per F8 there is no coverage gate over `xtask/`; these run in the host suite.
Four surfaces, one per layer:

1. **`gh.rs` classification** — table-driven over
   `classify(exit, stdout, stderr)` using **real captured specimens** (the F5
   404 body, the F5 GraphQL error) plus synthesized 403-rate-limit and 5xx
   bodies. No subprocess, no network.
2. **`snapshot.rs` parsing** — golden fixtures in `xtask/src/pr/testdata/`,
   captured live from real GitHub responses (merged PR, queued PR,
   `/rules/branches/main`), following the `traces/testdata` and
   `server_fn_coverage/testdata` precedent.
3. **`decide.rs`** — table-driven over hand-built `PrSnapshot`s, covering every
   outcome and specifically the traps: required set incomplete because
   `e2e gate` has not appeared (must **not** fire terminal); a non-required
   check failing (must **not** fire `checks-failed`); queue entry vanished while
   `OPEN` with a failed merge-group run (must fire `ejected`, distinct from
   `merged` and from still-queued).
4. **`watch.rs`** — scripted fake `PrSource` + **virtual `Clock`**, so a
   90-minute-timeout test runs in microseconds.

**No test hits the real GitHub API** — nondeterministic, credentialed, and would
break the host suite offline. The fixtures are the contract.

**Falsifiability bar:** the `watcher-error` test must fail if the loop returns
`Ok`, returns silence, **or** returns a `merged`/`checks-failed` verdict — not
merely assert "some error happened." That is the failure mode that actually bit
in #671, so it must distinguish all three wrong answers.

### D12 — One ADR, and the doc surface

One ADR (numberless in `docs/adr/drafts/`, numbered by `cargo xtask adr promote`
at ship), covering the four decisions a future reader would otherwise excavate:

1. xtask's charter extends to host-side observation of the **CI/merge system** —
   ADR-0028's litmus is "invoking `nix`, or analyzing build outputs," which this
   fits neither of.
2. `gh` as transport rather than a Rust client, with the F7 measurements
   recorded so the question is not re-answered differently later.
3. The gate shape is data read from the ruleset, so the ADR-0077 rollback is not
   a code change.
4. The observe/act split, and that the human halt is _which subcommand you
   type_.

| File                                                               | Change                                                                                                                                                                                                                           |
| ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **(out of tree)** `.claude/skills/jaunder-ship/SKILL.md` steps 7–8 | Replace the `gh pr checks --watch` + rebase-on-every-advance prose with `cargo xtask pr watch` / `pr land`. **Also remove the keep-current-with-`main` instruction**, which F1 made obsolete — it is no longer required to merge |
| `docs/ci-merge-queue.md`                                           | **Correct the false "auto-requeued" claim** (F3, line 190) and point at the command                                                                                                                                              |
| `CONTRIBUTING.md`                                                  | Add `pr watch` / `pr land` to the manual-tools section beside `audit-wasm` and `traces`                                                                                                                                          |
| `flake.nix` `devOnly`                                              | Add `pkgs.gh` (F6)                                                                                                                                                                                                               |
| **(out of tree)** `CLAUDE.md` xtask section | One line in the command table — it is the agent-facing doc, and agent re-derivation is the whole motivation. Untracked (F10), so same handling as the skill: edited at the main checkout, verified by inspection |

**The `jaunder-ship` row is deliberately out of the tracked tree.** Per F10,
`.claude/` is untracked and absent from this worktree entirely, so that edit
**cannot appear in the PR diff**, cannot be reviewed there, and takes effect
immediately for every concurrent session with no revert path through the branch.
Consequences, accepted explicitly:

- The edit is made against the **main checkout** path, not the worktree.
- It is verified **by inspection outside the diff**, and A13 splits accordingly:
  the four tracked files are branch-verifiable; the skill edit is confirmed by
  reading the file.
- It is made **last, at ship**, once the command's flag names are final — so the
  skill never documents an interface that shifted.

### D13 — Subject resolution, and the exit-2 boundary

Both commands take an **optional** PR number, so "which PR, in which repo" is a
resolution step that must have a stated home and a stated failure mode.

- **Repo identity** comes from the git remote (via the existing
  `xtask/src/git.rs`), never a hardcoded `jaunder-org/jaunder`. Both the GraphQL
  document and D6/D10's REST paths take `{owner}/{repo}` from it.
- **PR number**, when omitted, resolves from the current branch the way
  `gh pr view` does — the open PR whose head ref is the checked-out branch.
- Both live behind
  `PrSource::resolve(Option<PrNumber>) -> Result<Subject, ApiError>` (D3), so
  the fake source in tests supplies a `Subject` directly and no test needs a git
  remote or a network.

**The failure boundary**, which D5 leaves implicit and which resolution makes
concrete:

| Failure                                                                                                  | Result                                                                                                                           |
| -------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| clap usage error (bad flag, `pr land --once`)                                                            | exit **2**, no report — clap's own behaviour                                                                                     |
| No open PR for the current branch; explicit `<N>` does not exist; not in a git repo / no remote          | exit **2** with a message naming what was searched for. The _subject_ could not be established, so there is nothing to report on |
| `land`'s divergence refusal (D9)                                                                         | exit **2** — the request was refused, nothing failed                                                                             |
| `gh` missing, unauthenticated, rate-limited, or erroring — **at any point, including during resolution** | `watcher-error` **report**, exit 1. The tooling broke, which is exactly what that outcome exists to say                          |

The line is: **failures to establish the subject exit 2; failures to observe an
established subject are `watcher-error`.** The one case that could look
ambiguous — `gh` unavailable _during_ resolution — falls on the `watcher-error`
side, because "the tooling is broken" is more actionable than "no such PR" and
is what actually happened.

## Non-goals

- Re-running jobs, rebasing, force-pushing, or re-enqueueing after ejection
  (D1).
- Merging via any path other than `pr land` (D1).
- Running in CI or inside any Nix check — host-only manual commands (D2).
- Watching more than one PR per invocation. The multi-PR babysit sweep composes
  `--once`.
- Any test that reaches the network (D11).

## Acceptance criteria

Each is stated so a conformance review can tell delivered from not.

**A1.** `cargo xtask pr watch <N>` runs to exactly one terminal outcome from D4
and reports it as `pr.outcome` in `--json` and in `.xtask/last-result.json`.

**A2.** Exit is `0` iff the outcome is `merged`, `1` for every other terminal
outcome and for `pending`, and `2` only for failures that precede any report
(e.g. argument parsing).

**A3.** Every terminal outcome — **including `watcher-error`** — returns
`Ok(CommandResult)` with a populated `PrReport`, so `report()` runs and the
sidecar is written on every terminal path. Asserted on the **returned value**,
not the on-disk file: a `watcher-error` run yields `Ok`, a `pr` payload whose
`outcome` serializes to `"watcher-error"`, `exit_code() == 1`, and
`ok == false`. (Asserting the file itself would write to the relative
`.xtask/last-result.json` from a host test running under an enclosing
`cargo xtask check`, racing and clobbering the sidecar an agent may be reading.)

**A4.** A scripted `PrSource` returning `ApiError` for 5 consecutive polls
yields `watcher-error`. The test fails if the loop instead returns
`Ok`/`merged`, `checks-failed`, or produces no verdict.

**A5.** A rate-limit error whose reset is inside the remaining budget causes a
wait and a `rate limited until …` event, not a strike; one whose reset is
outside the budget yields `watcher-error` immediately. Both proved with a
virtual clock.

**A6.** A snapshot showing the PR `OPEN`, not queued, with a
`gh-readonly-queue/main/pr-<N>-…` run concluded `failure` whose `created_at` is
later than `head_committed_at` yields `ejected`, carrying that run's URL —
**distinct from `merged` and from still-queued**, and reached with no prior
observation of the queue entry (i.e. `--once` reaches it too). The mirror case
is also asserted: the **same** failed run against a head whose
`head_committed_at` is _later_ does **not** yield `ejected` — a stale
merge-group run from a previous push must not report as the current outcome.

**A7.** With a required context absent from `statusCheckRollup`, the loop does
**not** reach a terminal outcome; a failing **non-required** check does **not**
produce `checks-failed`; and, with a `merge_queue` rule present, **all required
contexts green is not terminal** — the loop continues into the enqueue phase
rather than reporting success.

**A7b.** A required context appearing **twice** (a re-run) is resolved to the
entry with the latest `completedAt`; a green re-run after a red original yields
no `checks-failed`.

**A8.** The required contexts, the strict flag, and queue-presence are read from
`/rules/branches/main` per run; no check name is hardcoded in `decide.rs`. A
fixture with `strict: true` makes `BEHIND` yield `stale`; the same fixture with
`strict: false` does not.

**A9.** Two identical consecutive snapshots produce exactly **one** event; ten
virtual minutes of stasis produce exactly one `heartbeat` event; an absorbed
transient failure produces a `poll-error` event immediately.

**A10.** `pr land` verifies arming with
`autoMergeRequest.enabledAt.is_some() || is_in_merge_queue`, re-arms once on a
silent no-op, and — with a fake whose first arm no-ops and whose second succeeds
— reaches `merged` rather than reporting a failed arm. A direct-enqueue fake
(`autoMergeRequest` null, `isInMergeQueue` true) is **not** reported as a failed
arm.

**A11.** `pr land` refuses — **exit 2, no `PrReport`** — when invoked from a
worktree whose HEAD branch is the PR's head ref and whose local commit differs
from the PR head SHA, with a message naming both SHAs. `pr land --once` is
rejected at parse time (clap, exit 2).

**A12.** `pr watch --once` returns a single classification without looping, and
can return `pending { phase }`.

**A13 (branch-verifiable).** In the PR diff: `docs/ci-merge-queue.md` no longer
claims ejected PRs are auto-requeued; `CONTRIBUTING.md` documents `pr watch` /
`pr land`; `flake.nix` lists `pkgs.gh` in `devOnly`.

**A13b (out of tree, verified by inspection).**
`.claude/skills/jaunder-ship/SKILL.md` steps 7–8 reference the commands rather
than describing the protocol, and no longer instruct keeping the branch current
with `main`; `CLAUDE.md`'s xtask command table gains a `pr watch` / `pr land`
row. Per F10 **neither** can appear in the diff; both are confirmed by reading
the files at the main checkout, and are done last (D12).

**A14.** `cargo xtask validate` is green, and `cargo xtask pr watch --help`
documents `--interval <SECONDS>`, `--timeout <MINUTES>`, `--once`, and the
optional PR number.

**A15.** Subject resolution behaves per D13: with no `<N>` and no open PR for
the current branch, exit **2** with a message naming the branch searched for and
**no** `PrReport`; with `gh` unavailable during resolution, a `watcher-error`
**report** and exit 1. Repo identity is read from the git remote — a fixture
proves no `jaunder-org/jaunder` literal reaches the API layer.

**A16.** `ok`, `exit_code()`, and `pr.outcome` agree on every terminal path —
asserted for at least `merged` (`ok=true`, exit 0) and `checks-failed`
(`ok=false`, exit 1), so the single pushed `StepResult` cannot drift from the
outcome (D5).

## Deviations from the issue body (amend #729 at ship)

1. **Exit codes.** The issue requires "a distinct exit code per outcome."
   Superseded by D5 — 0/1 plus JSON detail.
2. **`stale`.** The issue describes it as unconditional; F1 makes it unreachable
   today. D6 makes it ruleset-conditional rather than deleting it, so the
   documented rollback still works.
3. **Additional outcomes.** `conflicted` and `timed-out` are not in the issue's
   list.
4. **Arming predicate.** The issue calls `autoMergeRequest.enabledAt`
   "authoritative"; D9 widens it to include `isInMergeQueue`, which the issue's
   form would misreport.
5. **`--once`.** Not requested by the issue; added so the multi-PR babysit sweep
   in `jaunder-ship` step 7 does not go back to hand-rolled `gh pr checks`.
6. **`docs/ci-merge-queue.md` correction.** The auto-requeue claim (F3) is
   factually wrong and is fixed here rather than filed separately.
