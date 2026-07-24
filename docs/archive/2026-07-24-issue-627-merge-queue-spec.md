# Spec — #627: adopt GitHub merge queue to end the strict-rebase treadmill

**Status:** awaiting approval. **Milestone:** Test infrastructure & E2E (#6).
**Decision record:** an ADR (draft authored in this cycle, numbered at ship by
`cargo xtask adr promote`) recording the switch from a strict
up-to-date-before-merge ruleset to a GitHub merge queue — it **supersedes** the
strict-ruleset decision the current `main` protection encodes.

## Problem

`main` is protected by the ruleset **"Main branch protection"** (id `18086446`)
with `strict_required_status_checks_policy: true` — GitHub's
_up-to-date-before-merge_ requirement. When `main` advances during a PR's CI
(our e2e runs ~13 min cold), the PR goes `BEHIND` and must be re-synced and
**fully re-tested** before it can land, even when it neither textually nor
semantically conflicts. This is a rebase/retest treadmill that burns CI and
wall-clock. Live evidence: #624 (CI-tooling-only, near-zero conflict risk) went
green twice and was blocked both times by an intervening `main` advance.

The strict rule exists for a real reason — a past semantic conflict (textually
clean, logically incompatible) broke `HEAD`. The goal is to **keep that
combined-state guarantee while removing the manual re-sync.**

## Current state (baseline, verified)

- **`.github/workflows/ci.yml`** triggers on `pull_request` and
  `push: branches: [main]`. Jobs: `validate-no-e2e` (check name **"Validate (no
  e2e)"**), `e2e` matrix (`{sqlite,postgres}×{chromium,firefox}`),
  `elisp-integration`, and `e2e-gate` (`needs: [e2e, elisp-integration]`,
  `if: always()`, check name **"e2e gate"**). There is **no `merge_group:`
  trigger**, so none of these jobs run on queue branches today.
- **Ruleset "Main branch protection"** (id `18086446`, active): `deletion` +
  `non_fast_forward` protection; a `pull_request` rule (0 required approvals,
  `allowed_merge_methods: ["merge"]`); a `required_status_checks` rule with
  contexts **`Validate (no e2e)`** and **`e2e gate`** and
  **`strict_required_status_checks_policy: true`**. There is **no `merge_queue`
  rule**.
- The cachix step in every job carries
  `pushFilter: "jaunder-coverage|jaunder-e2e"` so the test-result check
  derivations are **never** served from cache — CI always re-runs the tests.
  This is what makes queue-branch testing meaningful: a `gh-readonly-queue/…`
  branch genuinely re-runs coverage + e2e against the combined tip, it does not
  trust a cached green.

## Decisions (interview-resolved)

1. **Two-part deliverable, split by reversibility.** The change divides into (a)
   a **code** change — the `merge_group:` trigger in `ci.yml` plus the ADR/docs
   — which rides the PR and is inert until (b) the **live ruleset flip** (enable
   the `merge_queue` rule + set `strict_required_status_checks_policy: false`),
   a repo-admin API action that cannot ride a PR and is the point of no return.
2. **The maintainer applies the flip on explicit approval, post-merge; the agent
   scripts and documents it.** This cycle's PR lands the code + a **runbook**
   with the exact GitHub API payloads to enable the queue and to **roll back**
   (restore strict, remove the `merge_queue` rule). After the PR merges under
   the _current_ strict rule, the agent applies the flip **only on a fresh,
   explicit "go" for this specific action** (an irreversible outward-facing
   change — prior approval does not carry). The queue is then validated on the
   next PR.
3. **Bootstrapping sequence is inherent, not a defect.** This PR merges under
   the strict rule (queue not yet enabled). The flip happens next. The **first
   PR to ride the queue is the one after this one** — so "a PR merges via the
   queue" is validated post-flip, not within this cycle's branch.
4. **Queue configuration:**
   - **Required checks on the queue** = **`Validate (no e2e)`** + **`e2e gate`**
     — the same two contexts already required, now also run on `merge_group`.
     The full `ci.yml` runs on queue branches; only these two aggregate contexts
     gate.
   - **Grouping strategy = ALLGREEN** — the required checks must pass for the
     queued PR **and every PR ahead of it** in the batch (GitHub stacks the
     merge commits `main+A`, `main+A+B`, … and requires each to be green). This
     keeps `main` green at **every** intermediate landing, not only at the
     combined tip that HEADGREEN alone would gate. The combined-tip state that
     catches a two-PR semantic conflict is tested under either strategy;
     ALLGREEN is chosen for the strictly stronger "every prefix green"
     guarantee. The exact ruleset enum value and its semantics are re-confirmed
     against GitHub's live ruleset schema when the runbook is authored.
   - **Merge method = `merge`** — matches the ruleset's only currently-allowed
     method; no change to how commits land on `main`.
   - **Batch size starts small** (build at most ~5 entries together). This repo
     is effectively serial single-developer, so batching rarely engages; the
     queue's real win here is removing the manual re-sync, not throughput. Exact
     min/max-entries and wait/timeout values are recorded in the runbook and may
     be tuned later without code change.
5. **`gh pr merge --auto` composes with the queue.** With a queue enabled,
   auto-merge **enqueues** a PR when it becomes mergeable (green + any required
   approval) rather than merging it directly. The existing ship flow's
   `gh pr merge --auto` therefore continues to work — this is asserted in the
   ADR and confirmed during post-flip validation.
6. **#629 (Validate OOM) is a monitored risk, not a hard gate —
   maintainer-approved reframe.** The issue names flaky containment as a hard
   prerequisite ("must land first"), singling out the OOM. That prerequisite
   work shipped in #624 (retries + flaky-surfacing) and #628 (elisp
   auth-readiness, closed). The remaining #629 OOM (~1-in-5 CI failures, P4) is
   **explicitly downgraded** — during this cycle's design interview the
   maintainer chose "proceed; monitor, don't hard-gate" — on the basis that
   under a queue an OOM-ejected PR is automatically re-queued, not silently
   poisoned. This is a deliberate deviation from the issue's hard-gate framing,
   signed off by the maintainer. The ADR records #629 as a known risk and names
   an explicit **rollback trigger** — if OOM ejections thrash the queue, revert
   the ruleset via the runbook. #629 stays tracked separately.
7. **Semantic-conflict guarantee = argued from config, evidenced by observing
   queue-branch check execution.** We do **not** manufacture two
   logically-incompatible PRs (a serial single-dev repo never produces them
   naturally, and the queue branch we would observe is almost always
   `main`+single-PR). So the guarantee is split into two honest halves:
   - **Observed (mechanical):** the required checks demonstrably **execute
     against a `gh-readonly-queue/…` ref** (i.e. against `main`+the queued
     change, not the stale PR head), proving the queue re-tests the
     combined-with-`main` state rather than trusting the PR's own green.
   - **Argued (config + docs):** that a _two-PR_ conflict is caught before merge
     rests on grouping = ALLGREEN plus GitHub's documented stacked-build /
     bisect-on-failure behavior — **not** on any single observation in this
     repo. Acceptance #4 asserts only the observed half; the argued half is
     recorded in the ADR as design rationale.

## Acceptance criteria (observable)

1. **`ci.yml` runs the required checks on queue branches.** `ci.yml` includes a
   `merge_group:` event trigger such that both `Validate (no e2e)` and
   `e2e gate` (and their `needs`) execute on a `merge_group` event, verifiable
   by reading the workflow triggers and by the presence of check runs on a
   `gh-readonly-queue/…` ref after the flip.
2. **A merge-queue runbook exists and is exact.** A committed doc gives the
   precise GitHub API calls (endpoint + JSON payload) to (a) enable the
   `merge_queue` ruleset rule with the config in Decision 4 and set
   `strict_required_status_checks_policy: false`, and (b) **roll back** to the
   current strict configuration. The payloads are derived from the live
   ruleset's actual shape, not guessed.
3. **A decision record is authored.** An ADR draft in `docs/adr/drafts/` records
   the adoption, states that it **supersedes** the strict-ruleset decision,
   names #629 as a monitored risk with a rollback trigger, and asserts
   `gh pr merge --auto` enqueues under the queue.
4. **(Post-flip, after maintainer applies the ruleset change) the queue works,
   observably:** a PR reaches `main` through the queue with **no manual
   re-sync**; the required checks are **observed running on an ephemeral
   `gh-readonly-queue/…` ref** (proving the combined-with-`main` state is
   re-tested, not the stale PR head); and `strict_required_status_checks_policy`
   reads `false` on the live ruleset (treadmill removed). This criterion asserts
   only these **observable** facts. The stronger _two-PR_ semantic-conflict
   catch is **not** claimed as observed here — it rests on ALLGREEN + GitHub's
   documented behavior (Decision 7) and is recorded as ADR rationale. This
   criterion is satisfied outside the PR branch, during post-merge validation.

## Out of scope

- **Fixing #629 (Validate OOM).** Tracked separately; folded in here only as a
  documented risk + rollback trigger, not a fix.
- **Reducing e2e wall-clock / batch-throughput tuning** beyond choosing
  conservative starting values. Queue config is data, tunable post-adoption
  without a code change.
- **Removing the `push: branches: [main]` post-merge CI run.** Redundant with
  the queue but harmless; leaving it is belt-and-suspenders.
- **Updating the agent ship skills / memories** that describe the old rebase
  treadmill (`.claude/` is untracked here). Noted for follow-up outside the
  repo.

## Verification

- **In-PR:** `ci.yml` is valid YAML with the `merge_group:` trigger wired so the
  two required contexts run on that event (confirm by workflow lint / reading
  triggers). `cargo xtask validate --no-e2e` stays green (no source touched;
  this is a CI-config + docs change). The runbook's API payloads are validated
  against the live ruleset's real JSON shape (a dry read of the ruleset, not a
  mutation).
- **Post-flip (maintainer-gated):** enable the queue via the runbook, then
  observe on the next PR: check runs appear on a `gh-readonly-queue/…` ref; the
  PR merges without manual re-sync; the live ruleset shows
  `strict_required_status_checks_policy: false` and a `merge_queue` rule. Roll
  back via the runbook if OOM ejection thrashes the queue.
