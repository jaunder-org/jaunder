# ADR-0077: Adopt a GitHub merge queue for main

- Status: accepted
- Date: 2026-07-24
- Issue: [#627](https://github.com/jaunder-org/jaunder/issues/627)

## Context

`main` is protected by the ruleset **"Main branch protection"** (id `18086446`)
with `strict_required_status_checks_policy: true` — GitHub's
_up-to-date-before-merge_ requirement. When `main` advances during a PR's CI
(our e2e runs ~13 min cold), the PR goes `BEHIND` and must be re-synced and
**fully re-tested** before it can land, even when it neither textually nor
semantically conflicts. This is a rebase/retest treadmill that burns CI and
wall-clock. Live evidence: #624 (CI-tooling-only, near-zero conflict risk) went
green twice and was blocked both times by an intervening `main` advance.

The strict rule is not gratuitous: it exists because a past **semantic
conflict** — two changes textually clean but logically incompatible — merged and
broke `HEAD`. Up-to-date-before-merge closes that hole by forcing each PR to be
tested against the real tip of `main` before it lands. The goal here is to
**keep that combined-state guarantee while removing the manual re-sync.**

See the spec `docs/superpowers/specs/2026-07-24-issue-627-merge-queue.md` for
the full decision interview.

## Decision

Adopt a **GitHub merge queue** on `main`. On enqueue, GitHub builds the PR on an
ephemeral `gh-readonly-queue/…` branch stacked on everything ahead of it and
runs the required checks there — so the combined state is tested before merge
(still catching the semantic-conflict class), and developers stop hand-rebasing.

Concretely:

- **`.github/workflows/ci.yml` gains a `merge_group:` trigger** so the required
  contexts `Validate (no e2e)` and `e2e gate` (the latter via its `needs`) run
  on queue branches. All jobs run on `merge_group`; no per-job filtering.
- **Ruleset change (applied post-merge, maintainer-gated — see below):** add a
  `merge_queue` rule (grouping **`ALLGREEN`** — every stacked prefix `main+A`,
  `main+A+B`, … must pass, keeping `main` green at every intermediate landing;
  merge method **`merge`**, matching the ruleset's only allowed method; a small
  starting batch, since this repo is effectively serial) and set
  **`strict_required_status_checks_policy: false`**. This **supersedes** the
  strict-ruleset decision that the current `main` protection encodes.
- **`gh pr merge --auto` composes with the queue:** under a queue, auto-merge
  **enqueues** a PR when it becomes mergeable (green + any required approval)
  rather than direct-merging. The existing ship flow is unaffected.

The exact enable and rollback API payloads live in `docs/ci-merge-queue.md`.

**The live ruleset flip is not performed in the PR that introduces this ADR.**
The PR (ci.yml + runbook + this ADR) merges under the current strict rule; the
flip is a repo-admin action applied afterward, only on a fresh explicit
maintainer "go" (an irreversible, outward-facing change). The first PR to
actually ride the queue is the one after this one.

## Consequences

- The up-to-date-before-merge treadmill is removed: PRs no longer need manual
  re-sync on every `main` advance; the queue owns keeping them current.
- The combined-state (semantic-conflict) guarantee is **preserved** via
  `ALLGREEN` grouping + GitHub's stacked-build / bisect-on-failure. Note this
  guarantee is **argued from configuration**, not observed: a serial
  single-developer repo never naturally produces two simultaneously-queued
  conflicting PRs, so the queue branch we observe is almost always `main`+a
  single PR. What is observed post-flip is that the required checks execute
  against a `gh-readonly-queue/…` ref (proving combined-with-`main` re-testing);
  the two-PR catch rests on the config + GitHub's documented behavior.
- The "never trust a cached green check" property is retained: the
  `jaunder-coverage|jaunder-e2e` cachix `pushFilter` means the test-result
  derivations are re-run on queue branches too, so a `gh-readonly-queue/…`
  branch genuinely re-tests the combined state.
- **#629 (`Validate (no e2e)` runner OOM, ~1-in-5 CI failures, P4) is a
  monitored risk, not a hard gate** — a maintainer-approved reframe of the
  issue's "flaky containment must land first." Under a queue, an OOM-ejected PR
  is **automatically re-queued**, not silently poisoned. **Rollback trigger:**
  if OOM ejections thrash the queue, restore the strict ruleset via the rollback
  call in `docs/ci-merge-queue.md`. #629 stays tracked separately.
- Follow-up outside this repo: the agent ship skills / memories that describe
  the old rebase treadmill (`.claude/` is untracked here) should be updated to
  reflect that the queue owns keeping branches current.
