# Spec — Git-enforced dev gate (issue #99)

**Issue:** jaunder-org/jaunder#99 (Verify-gate hardening; sibling to #86/#87/#88, interacts with #37)
**Date:** 2026-06-27
**Status:** approved design, pre-implementation

## Problem

The verify gate is **agent discipline, not machine-enforced**, so it gets skipped,
misread, or — defensively — over-run. The most expensive failure mode is running the
full `cargo xtask validate` (with the ~18-minute e2e VMs) per commit when the change
can't affect e2e. The intended standard is the lighter gate per commit, with e2e
reserved for ship/CI — but nothing enforces it.

`.githooks/` already holds a half-built version of the right thing, and **neither hook
is installed** (`core.hooksPath` is the default `.git/hooks`):

- `.githooks/pre-commit` runs raw `leptosfmt --check` / `cargo fmt --check` /
  `prettier --check` / `cargo clippy` / `cargo nextest run` — verify-only, bypasses
  xtask, diverges from the real gate.
- `.githooks/pre-push` runs `cargo xtask validate --no-e2e` (with a `SKIP_PRE_PUSH=1`
  escape) — essentially correct already.

## Gate composition (verified from `xtask/src/lib.rs`)

`check` (Fix-mode) and `validate` (verify-mode) run the **same three steps** —
`static_checks` + `host_tests` (xtask's own unit suite) + `nix::coverage` — plus
`validate` adds the e2e VMs unless `--no-e2e`. The **jaunder application test suite
runs inside `nix::coverage`**; `check --no-test` runs only static + clippy + xtask
units, *not* the app tests. So "run tests pre-commit" means the full Nix coverage
build.

| Command | static | clippy | xtask units | Nix coverage (app tests + PG) | e2e | mode |
|---|---|---|---|---|---|---|
| `check --no-test` | ✓ (Fix) | ✓ | ✓ | — | — | mutates |
| `check` | ✓ (Fix) | ✓ | ✓ | ✓ | — | mutates |
| `validate --no-e2e` | ✓ (Check) | ✓ | ✓ | ✓ | — | verify-only |
| `validate` | ✓ (Check) | ✓ | ✓ | ✓ | ✓ | verify-only |

## Decisions (settled in brainstorming)

1. **Pre-commit runs full `check`** (Nix coverage, Fix-mode) — not `--no-test`.
   Mirrors the gate, runs the app tests for commit-history cleanliness, and warms the
   GC-rooted coverage cache so the pre-push `validate` is a cache hit.
2. **Fix handling = fail-and-restage.** When `check` rewrites files, the hook aborts
   and asks the author to review + `git add` + re-commit. Nothing is silently folded
   in.
3. **Keep the pre-push `validate --no-e2e` hook.** Its value is *not* a redundant
   re-verify of `check`: `check` runs on (and tolerates) a dirty tree, so it can go
   green with uncommitted/untracked changes lurking. `validate`'s dirty-tree refusal
   makes pre-push the one place that proves **what was measured == the committed tip ==
   what CI will see, with nothing uncommitted hiding.** It is the clean-tree backstop
   `check` structurally cannot be.
4. **Self-healing installer.** Any `cargo xtask` run wires `core.hooksPath` if needed.
5. **e2e stays ship/CI-only.** No hook runs e2e; CI's full `validate` on every PR is
   the backstop.

## Design

### 1. `.githooks/pre-commit` (replaces the raw-tooling hook)

```bash
#!/usr/bin/env bash
set -euo pipefail
[ "${SKIP_PRE_COMMIT:-0}" = "1" ] && exit 0

pre=$(git status --porcelain)
cargo xtask check                 # Fix-mode: static+clippy+xtask-units+Nix coverage
post=$(git status --porcelain)

if [ "$pre" != "$post" ]; then
  echo "pre-commit: check applied fixes — review, git add, and re-commit" >&2
  exit 1                          # fail-and-restage
fi
```

- The **porcelain before/after diff** detects "check changed something" (fmt, clippy,
  or a `coverage-baseline.json` re-anchor — all tracked, so all surface). Gitignored
  `.xtask/` scratch (gcroots, `last-result.json`) is excluded by porcelain, so it
  cannot false-trip the guard.
- **Happy path:** the committer ran `check` before committing, so `pre == post` →
  silent pass, and the Nix coverage step is a cache hit. The abort fires only when
  `check` was skipped — exactly when a stop-and-look is warranted.
- `SKIP_PRE_COMMIT=1` escape mirrors the existing `SKIP_PRE_PUSH`.

### 2. `.githooks/pre-push` (kept, minor tidy)

Runs `cargo xtask validate --no-e2e`; keeps `SKIP_PRE_PUSH=1`. No content change
required beyond confirming it stays routed through xtask. It now inherits the
dirty-tree refusal from `validate` (below), which is the point of keeping it.

### 3. Dirty-tree refusal on `validate` (not `check`)

`Command::Validate` gains a precheck: before running steps, if `git status
--porcelain` is non-empty, fail fast with a message listing the offending paths,
unless `--allow-dirty` is passed.

- **Porcelain semantics are deliberate:** it includes **untracked non-gitignored
  files** (the exact surface the Nix coverage source picks up — see #37) and excludes
  gitignored paths. A `git diff --quiet` check would miss untracked files and is wrong
  here.
- **Only `validate` refuses.** `check` is Fix-mode and is *meant* to run on a dirty
  tree (it fixes it), so it must not refuse.
- `--allow-dirty` is the escape for deliberate local iteration. CI checks out clean,
  so it is unaffected.
- **Interaction with #37:** refusing a dirty tree enforces "the gate runs only on a
  clean tree == HEAD," which neutralizes #37's untracked-instrumentation footgun *on
  the gate path* without changing the flake source filter. #37 (the flake-side source
  contract) remains separate and is not resolved here.

### 4. Self-healing installer

At the top of `cargo xtask` (before subcommand dispatch): read `git config --get
core.hooksPath`; if it is not `.githooks`, set it with `git config core.hooksPath
.githooks`.

- **Relative path** (`.githooks`, not absolute) so each worktree resolves to its own
  `.githooks` checkout (git resolves a relative `core.hooksPath` against the working
  tree root where hooks run).
- **Idempotent**, logs once when it sets the value, silent otherwise. `core.hooksPath`
  lives in the shared (common) config, so one set covers the main checkout and all
  worktrees.
- Fresh clones and new worktrees auto-wire on their first `cargo xtask` run — no
  separate install step to remember.

### 5. e2e placement (unchanged from intent, now enforced)

No hook runs e2e. CI runs full `cargo xtask validate` (with e2e) on every PR
(`.github/workflows/ci.yml`), so e2e is always the CI backstop. Local full `validate`
runs only at ship, and only when the branch diff plausibly touches e2e surface.

## Ripple — same cycle, tracked as plan tasks

The skills and the auto-memory must change in lockstep, or the guidance contradicts
the enforced behavior:

- **`jaunder-commit`:** per-commit gate = `cargo xtask check` (hook-enforced); **delete
  the "bump to full `validate` for e2e-relevant surface" per-commit loophole**; push
  runs `validate --no-e2e` (hook); e2e is ship/CI-only.
- **`jaunder-iterate`:** note the pre-commit hook enforces the commit gate; the inner
  fixing loop stays `check --no-test`.
- **`jaunder-dispatch`:** unchanged in spirit — subagents don't commit, so they keep
  `check --no-test` as their fast per-task gate.
- **`jaunder-ship`:** keep e2e at ship; note `validate` now requires a clean tree, so
  planning-doc archiving / any last edits must be committed before the final gate.
- **autonomous-work memory note** (`feedback_autonomous_work_authorization.md`):
  "commit only after `validate --no-e2e` passes" → "commits are gated by the
  pre-commit `check` hook; push runs `validate --no-e2e`."

## ADR

Record **ADR-0029 — Git-Enforced Verify Gate** (`docs/adr/0029-git-enforced-verify-gate.md`)
plus its row in the `docs/README.md` ADR table. It captures the enforcement contract:
hook-routed `check` (pre-commit) / `validate --no-e2e` (pre-push), clean-tree-only
gating via `validate`'s dirty refusal, fail-and-restage fix handling, and e2e
reserved for ship/CI.

## Testing

- **xtask unit tests:**
  - dirty-tree predicate: porcelain parsing returns "dirty" for untracked
    non-gitignored files, staged changes, and unstaged tracked edits; "clean" for an
    empty/whitespace-only porcelain; `--allow-dirty` bypasses the refusal.
  - installer: hooksPath logic sets `.githooks` when unset/wrong, no-ops when already
    correct.
- **Hook scripts** are thin bash; their behavior (clean-pass vs fail-and-restage,
  `SKIP_*` escapes) is verified by the predicate/installer tests plus manual
  confirmation, not a bespoke harness.
- Coverage policy per `CONTRIBUTING.md`: new xtask logic carries tests or an approved
  baseline entry.

## Out of scope

- The flake-side source contract (#37) — only neutralized on the gate path, not fixed.
- Coverage gate UX / re-anchor ergonomics (#86/#87/#88).
- Changing what e2e itself runs (#93).

## Acceptance

- `.githooks/pre-commit` runs `cargo xtask check` with fail-and-restage; `.githooks/pre-push`
  runs `cargo xtask validate --no-e2e`; both active via `core.hooksPath` → `.githooks`.
- A self-healing installer wires `core.hooksPath` on any `cargo xtask` run.
- `cargo xtask validate` refuses a dirty tree (porcelain incl. untracked) with a clear
  message; `--allow-dirty` escapes it.
- ADR-0029 written and listed in `docs/README.md`.
- Ripple skills + memory note updated to match the enforced gate.
