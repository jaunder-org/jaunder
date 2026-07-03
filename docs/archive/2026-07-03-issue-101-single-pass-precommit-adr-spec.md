# Spec — issue #101: reconcile ADR-0029 with the already-collapsed single-pass pre-commit hook

- Issue: jaunder-org/jaunder#101
- Date: 2026-07-03
- Status: draft (pending approval)

## Summary of the finding

Issue #101 asked to **collapse the two-pass pre-commit hook to a single
`cargo xtask check`** once coverage heals became idempotent (blocked-by #86).
Investigation shows the hook was **already collapsed** — commit `40e2620a`
("feat(xtask,hooks): pre-commit runs full check; heal line-as-hint (#113)")
rewrote `.githooks/pre-commit` to a single `cargo xtask check` with a
porcelain-diff fail-and-restage. #86 (re-anchor safety) is closed.

So two of the issue's three acceptance criteria are **already satisfied** by
prior work, and the third — updating ADR-0029 — was **not** done: ADR-0029's
Decision and Consequences still prescribe the two-pass form and call it a
"stopgap [that] collapses once #86…". The ADR now contradicts the shipped hook.

This spec therefore scopes #101 down to **reconciling ADR-0029 with the
collapsed hook**, plus an empirical confirmation that the idempotency the
collapse depends on actually holds.

## Current state (verified)

- `.githooks/pre-commit` runs a **single** `cargo xtask check` guarded by a
  `git status --porcelain` before/after comparison (fail-and-restage on any
  change). Header comment already documents the collapsed rationale and that it
  "replaces the earlier two-pass hack".
- Idempotency of the Fix-mode heal is established and unit-tested in
  `xtask/src/coverage/mod.rs`:
  - `heal_baseline` only persists a new baseline "when it differs from the
    loaded baseline"; a pure line-shift is healed to a hint, not a rewrite.
  - Tests cover: heal-when-safe (Fix), no-heal in Check mode, no-heal on unsafe
    lowering, no-heal on CRAP regression, pure-shift and now-covered cases; the
    #110 test confirms classify loads the baseline from the anchor commit, not a
    dirty working tree (no double-shift).
- ADR-0029 (`docs/adr/0029-git-enforced-verify-gate.md`) **still describes the
  two-pass hook** in its Decision (the two numbered passes) and Consequences
  (the "two-pass … stopgap … collapses once #86" bullet). This is the stale
  record #101 must fix.

## Decisions

1. **In-place edit of ADR-0029, not a new ADR.** The decision to collapse once
   idempotent was already recorded in 0029's own Consequences; reconciling the
   record with the now-shipped reality is a content edit to the existing ADR,
   not a new architectural decision and not a status change. Status stays
   `accepted`. The `## Context` section (lines 6–14, describing the pre-0029
   obsolete hook) and the `## Supplement (#103)` section (merge-driver
   self-heal) are both unrelated to the collapse and left untouched.
2. **No new idempotency regression test.** The load-bearing property (a clean
   tree does not churn the manifests, so the hook does not fail-and-restage) is
   already locked by the `heal_baseline` unit tests listed above. A hook-level
   test would require running the full Nix-instrumented coverage gate twice and
   would duplicate coverage that already exists at the function level. Instead,
   acceptance is confirmed **empirically once** during this cycle (below).
3. **ADR wording records the collapse as executed, crediting the enabling work**
   (#113 for the hook rewrite; #7 line-agnostic CRAP, #86 re-anchor, #113
   line-as-hint heal for the idempotency it depends on), rather than silently
   swapping the text — so a future reader sees the stopgap was resolved
   deliberately, not lost.

## Acceptance criteria

- **AC1 — ADR Decision reflects single-pass.** ADR-0029's Decision section
  describes the pre-commit hook as a **single `cargo xtask check`** with the
  porcelain-diff fail-and-restage, with no surviving prescription of the two
  numbered passes (`check --no-test` + `validate --no-e2e --allow-dirty`) as the
  current form. (Observable: reading the Decision section, no reader would
  implement a two-pass hook.)
- **AC2 — ADR Consequences no longer frames two-pass as the live stopgap.** The
  "two-pass … stopgap … collapses once #86" bullet is rewritten to record that
  the collapse **has happened** (crediting #113/#7/#86), so Consequences and the
  shipped hook agree.
- **AC3 — Hook unchanged and already correct.** `.githooks/pre-commit` remains a
  single `cargo xtask check` with porcelain fail-and-restage. (No code change is
  expected; this criterion is a guard that the reconciliation did not perturb
  the hook.)
- **AC4 — Idempotency confirmed empirically.** On the clean committed worktree,
  a **single** `cargo xtask check` leaves `git status --porcelain` empty (no
  manifest churn on the first run ⇒ the single-pass hook, which runs `check`
  exactly once from a clean tree, would not fail-and-restage on a clean-tree
  commit). Emptiness after the _first_ run is the load-bearing observable — a
  first run that churns and a second that stabilizes would still trip the hook.
  A second run may be observed as a bonus. The result is recorded in the cycle.
- **AC5 — Gate green.** `cargo xtask validate --no-e2e` passes on the branch
  (doc-only change; no e2e-affecting surface), and the ADR README table stays in
  sync if the tooling regenerates it.

## Out of scope

- Any change to the hook's behavior or to the coverage heal machinery.
- The pre-push hook, `validate`'s dirty-tree refusal, and the #103 merge-driver
  supplement (all already documented and unrelated).
- Other milestone-1 coverage issues (#100, #37) — separate cycles.
