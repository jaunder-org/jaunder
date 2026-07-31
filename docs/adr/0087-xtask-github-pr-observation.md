# ADR-0087: xtask observes the CI/merge system, with `gh` as its transport

- Status: accepted
- Date: 2026-07-30
- Issue: [#729](https://github.com/jaunder-org/jaunder/issues/729)

## Context

Every agent that ships a PR re-derives the same green→queue→merged watcher, and
gets it wrong in the same ways. One #671 session produced three
`gh pr checks --watch` invocations (all died) and two hand-rolled poll loops —
the first treating an API failure as "no change", the second emitting on every
poll until it was tightened.

The behaviour was already written down, in prose, across `jaunder-ship`,
`docs/ci-merge-queue.md`, and agent memory. Prose gets re-implemented, and the
re-implementation is where the bugs live. The fussiness is entirely in failure
modes that are invisible until hit:

- `gh pr checks --watch` exits non-zero identically whether CI failed, the API
  rate-limited, or the harness killed it — three meanings, one signal.
- Green checks are phase one. Under a merge queue the real sequence continues
  `enqueue → merge_group against live main → merged`.
- Ejection is silent: a failed front-of-queue `merge_group` makes the queue
  entry vanish while the PR stays `OPEN`.
- `gh pr merge` prints the same thing whether it armed, enqueued, or did
  nothing.
- The `e2e gate` aggregate check appears late, so "no check is pending" is
  briefly true over an incomplete set.

Four decisions fall out, each of which a later reader would otherwise have to
excavate from the code.

## Decision

### 1. xtask's charter extends to host-side observation of the CI/merge system

[ADR-0028](0028-devtool-vs-xtask-boundary.md) draws the `devtool`/`xtask` line
with a litmus: _"Does it need to run where `nix`/`xtask` are absent (inside a
derivation)?"_ → `devtool`. _"Does it run on the host — invoking `nix`, or
analyzing build outputs?"_ → `xtask`.

`pr watch` fits **neither** host clause: it invokes no `nix` and analyses no
build output. It queries GitHub. Rather than let that happen silently, this ADR
states the extension: **xtask also owns host-side observation of the systems
that gate our merges.** It is the right home because it is versioned alongside
the gates it reports on — a newly-required check moves with it — and because a
skill carrying a canned script drifts the moment the required-check set changes,
which happened three times in one cycle. `devtool` remains wrong for it under
ADR-0028: this produces no artifact in a sandbox.

### 2. `gh` is the transport, not a Rust GitHub client

`pr watch`/`pr land` shell out to `gh api` and `gh api graphql` from exactly one
file (`xtask/src/pr/gh.rs`). Measured alternatives, at the time of writing:

|                            | shell `gh`                      | `octocrab` 0.54                                                         |
| -------------------------- | ------------------------------- | ----------------------------------------------------------------------- |
| Crates added to xtask's 93 | 0                               | **118** (211 total), incl. `tokio`, `hyper`, `rustls`, `ring`           |
| Merge-queue models         | n/a                             | **none** — its only `merge_queue` match is an unrelated webhook payload |
| GraphQL surface            | hand-written query, own structs | `graphql<R: DeserializeOwned>` — hand-written query, own structs        |
| Async runtime              | none needed                     | required, for one request per 30s                                       |
| Auth                       | `gh` owns the token already     | needs a PAT, or shells `gh auth token` anyway                           |

Every field this command turns on — `mergeQueueEntry`, `isInMergeQueue`,
`autoMergeRequest`, `mergeStateStatus`, `statusCheckRollup` — is GraphQL, which
octocrab does not model. It would buy the same query and the same structs for a
3× dependency graph and an async runtime in a synchronous CLI that rebuilds from
the working tree on every invocation.

The cost accepted: `gh` collapses every failure into exit 1, so classification
must read the response body. That is why `gh.rs` splits **running** from
**classifying** — `classify(exit, stdout, stderr)` is pure, and every transport
failure is therefore testable with no network.

`pkgs.gh` joins the devShell's `devOnly` list, not `ciInputs`: these are
host-only manual commands, never run by a Nix check or a CI job.

### 3. The gate's shape is data, read from the ruleset per run

One read of `/repos/{owner}/{repo}/rules/branches/main` yields the required
check contexts, `strict_required_status_checks_policy`, and whether a
`merge_queue` rule exists. The state machine branches on all three:

- strict on → `BEHIND` is terminal (`stale`); strict off → being behind is not
  blocking
- queue present → the enqueue/ejection phase exists at all

So "required checks as data" generalises to "the whole gate shape is data". The
rollback documented in `docs/ci-merge-queue.md` — restoring strict and removing
the queue if #629's OOM ejections thrash it — needs **no code change**, and a
newly-required check is picked up without one either.

### 4. Observing and acting are separate commands; running `land` is the approval

`pr watch` observes and can merge nothing. `pr land` arms auto-merge and drives
it home, and **typing it is the merge approval** — the human gate is structural
rather than a prompt. `PrArmer` is a separate trait from `PrSource` so no
observer can mutate.

The tool owns only the crank-turning steps: poll to a terminal state, re-arm
when the arm silently no-ops, confirm the merge. It refuses the judgement calls
— re-running a red job ("flake or real?"), rebasing, re-enqueueing after an
ejection — and reports the state so a human decides.

## Consequences

- **Good:** the six failure rules live in one pure function (`decide.rs`) and
  are tested from hand-built values — no network, no `gh`, no sleeping. A
  90-minute timeout is exercised in microseconds against a virtual clock.
- **Good:** `watch`/`land` return `PrReport`, never `Result`, so "the watcher
  itself failed" is a _reported outcome_ rather than an error that never gets
  written down. Silence can no longer read as success.
- **Good:** the ruleset read makes the ADR-0077 rollback and future check
  renames configuration changes rather than code changes.
- **Trade-off:** `gh` becomes a hard dependency of these two commands and is
  added to the devShell. A missing or unauthenticated `gh` is reported as
  `watcher-error`, not hidden.
- **Trade-off:** ejection detection relies on GitHub's
  `gh-readonly-queue/main/pr-<N>-…` branch convention, which we do not control.
  Guarded: a PR observed queued and then not queued with no matching run emits a
  loud warning rather than concluding "not ejected".
- **Ruled out:** a Rust GitHub client for this surface, and any future
  temptation to give `watch` the ability to merge. Adding actions to `watch`
  would collapse the structural approval gate that decision 4 exists to create.
