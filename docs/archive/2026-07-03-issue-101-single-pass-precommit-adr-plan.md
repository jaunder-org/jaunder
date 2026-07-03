# Plan — issue #101: reconcile ADR-0029 with the single-pass pre-commit hook

Spec:
[`2026-07-03-issue-101-single-pass-precommit-adr.md`](../specs/2026-07-03-issue-101-single-pass-precommit-adr.md)
Issue: jaunder-org/jaunder#101

## Review header (approve this layer)

- **Goal:** Make ADR-0029's Decision and Consequences describe the _shipped_
  single-pass `cargo xtask check` pre-commit hook (collapsed by #113), instead
  of the obsolete two-pass stopgap they still prescribe. The hook and its
  idempotency are already done; this closes the one remaining #101 acceptance
  criterion (the ADR) and confirms the idempotency the collapse depends on.
- **Scope (in):** Text edits to `docs/adr/0029-git-enforced-verify-gate.md`
  (Decision lines 17–36, Consequences lines 56–59); one empirical idempotency
  check; the verify gate.
- **Scope (out):** Any change to `.githooks/pre-commit` or the coverage/heal
  code (already correct); ADR Context (6–14) and Supplement #103 (66–89); other
  milestone-1 issues (#100, #37). No separable concerns to file — the
  out-of-scope items are already-tracked issues.
- **Tasks:**
  1. Empirically confirm heal idempotency on the clean tree (AC4).
  2. Rewrite ADR-0029 Decision + Consequences to the single-pass form (AC1,
     AC2).
  3. Verify hook untouched + run the verify gate; commit (AC3, AC5).
- **Key risks/decisions:**
  - In-place ADR edit, not a new ADR (spec Decision 1) — status stays
    `accepted`.
  - No new regression test (spec Decision 2) — idempotency is already
    unit-tested in `heal_baseline` (`xtask/src/coverage/mod.rs:103`, tests
    812–938).
  - Risk: AC4's `cargo xtask check` is a full Nix-instrumented coverage run
    (~minutes, cachix-warmed). Not a correctness risk; just runtime.
  - Risk: if AC4 _fails_ (the clean tree churns), the spec's premise is wrong
    and we stop and re-open the finding — do not paper over it in the ADR.

## Global constraints

- **For agentic workers:** drive with `jaunder-iterate`; `jaunder-dispatch` is
  unnecessary here (no code, single small file). Tick checkboxes in real time.
- Run all commands from the worktree
  (`.claude/worktrees/issue-101-single-pass-precommit`) so the gate builds this
  branch, not main. Use `devtool run -- cargo xtask …` (worktree-aware, honest
  exit), then filter the parked log.
- No `Co-Authored-By` trailer on commits. Commit only with user approval per the
  cycle's before-merge gate; intra-branch commits follow `jaunder-commit`.
- Doc-only change: no e2e-affecting surface, so `validate --no-e2e` is the gate.

---

## Task 1 — Confirm heal idempotency on the clean tree (AC4)

**Why first:** validates the spec's load-bearing premise (a clean-tree `check`
does not churn the manifests) _before_ we rewrite the ADR to assert the collapse
is safe. If this fails, the finding is wrong and we halt.

**Observable (stated once):** after one `cargo xtask check` on the clean
committed tree, `git status --porcelain` shows **no new or modified
`coverage-baseline.json` or `crap-manifest.json` entries** (the new spec/plan
docs may appear — they are irrelevant to this check; the manifests are what the
hook's fail-and-restage keys on).

**Steps:**

1. Record the starting porcelain (only the new spec/plan docs expected).
2. Run a single full check: `devtool run -- cargo xtask check`.
   - Expected: exit 0 (`ok:true`); no manifest churn per the observable above.
3. (Bonus) Run it a second time; manifests still unchanged.

**Done when:** the observable holds after a single run (the single-pass hook
would not fail-and-restage). Record the observed result (exit code + manifest
porcelain) in the cycle notes / commit message. **If a manifest churns, STOP**
and surface — do not proceed to Task 2.

## Task 2 — Rewrite ADR-0029 Decision + Consequences (AC1, AC2)

**File:** `docs/adr/0029-git-enforced-verify-gate.md`

**Edit A — Decision (lines 17–36).** Replace the two-pass description
(`check --no-test` pass 1 + `validate --no-e2e --allow-dirty` pass 2, and the
"obvious single-pass … not usable today" paragraph) with the single-pass form:

- Pre-commit runs **one `cargo xtask check`** (fmt + clippy + Nix coverage/test
  gate in Fix mode, with auto-heal).
- Fail-and-restage on a `git status --porcelain` before/after difference: if
  `check` applied a real fix (reformat, or genuine coverage/CRAP change) the
  hook aborts so the author consciously `git add`s and re-commits.
- Note _why_ single-pass is now safe: the heal is idempotent on a clean tree —
  the baseline compares by line-independent text fingerprint and only persists
  when it differs (#113), the CRAP manifest ignores line attribution (#7), and
  benign pure line-shifts self-heal via re-anchor (#86) — so `check` mutates the
  tree only on a _real_ change, not on every run.

**Edit B — Consequences (lines 56–59).** Rewrite the "two-pass … stopgap …
collapses once #86" bullet to record that the collapse **has happened**: the
hook was collapsed to the single `cargo xtask check` (#113) once the heal became
idempotent on a clean tree (#7 line-agnostic CRAP, #86 re-anchor, #113
line-as-hint baseline heal), dropping the duplicated fmt/clippy pass.

**Constraints:** keep Status `accepted`; do **not** touch Context (6–14) or the
Supplement #103 (66–89); keep the title accurate (it already reads generically,
"Hook-Routed `check`/`validate`"). Match the file's existing prose style and
line wrapping.

**Done when:** reading the Decision, no reader would implement a two-pass hook
(AC1); the Consequences bullet frames the collapse as done and credits
#113/#7/#86, agreeing with the shipped hook (AC2). Cross-check against
`.githooks/pre-commit` for consistency.

## Task 3 — Verify hook untouched, run the gate, commit (AC3, AC5)

**Steps:**

1. **AC3 guard:** `git diff --stat -- .githooks/pre-commit` is empty (the hook
   was not perturbed); confirm it is still a single `cargo xtask check` with the
   porcelain fail-and-restage.
2. **AC5 gate:** `devtool run -- cargo xtask validate --no-e2e`; filter the
   parked log for the pass/fail. Expected: green (doc-only change).
   - The ADR README sync check runs inside the gate; a pure body edit with no
     title/status change leaves the README table unchanged, so it should pass
     with no README diff. If the tooling _does_ regenerate the table, stage that
     too.
3. Commit the ADR edit (+ spec/plan docs) via `jaunder-commit` — the pre-commit
   hook runs `cargo xtask check`; run `cargo xtask check` first so it passes
   clean. Commit message references #101. **No `Co-Authored-By`.**

**Done when:** hook diff empty (AC3); `validate --no-e2e` green (AC5); the ADR
reconciliation is committed on the branch.

---

## Self-review

- Every spec AC maps to a task: AC1/AC2 → Task 2; AC3/AC5 → Task 3; AC4 →
  Task 1.
- Tasks are independently verifiable (porcelain empty; ADR reads single-pass;
  hook diff empty + gate green) and ordered (premise-confirm → edit → verify).
- No task smuggles out-of-scope work; no code/test changes; no separable concern
  to file. No placeholders.
