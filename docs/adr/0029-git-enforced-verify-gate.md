# ADR-0029: Git-Enforced Verify Gate — Hook-Routed `check`/`validate` and Clean-Tree Gating

Status: accepted

## Context

The verify gate (`cargo xtask check` / `validate`) was agent discipline, not
machine-enforced, so it was skipped, misread, or defensively over-run — most
expensively by running the full `validate` (with the ~18-minute e2e VMs) per commit
when the change could not affect e2e. `.githooks/` held an obsolete, uninstalled
pre-commit hook that bypassed xtask (raw `leptosfmt`/`fmt`/`prettier`/`clippy`/
`nextest`), and `core.hooksPath` still pointed at the default `.git/hooks`.

## Decision

- **Pre-commit hook → two passes** (a deliberate stopgap, see below):
  1. `cargo xtask check --no-test` — fixes fmt/leptosfmt/prettier + runs clippy, but
     **skips the Nix coverage step** so it cannot churn the coverage manifests. If it
     rewrote anything (i.e. reformatted staged code), the hook **fails and asks the
     author to restage** rather than silently folding the fix in. Detection is a
     `git status --porcelain` before/after diff — safe here because `--no-test` means
     the only possible change is real formatting.
  2. `cargo xtask validate --no-e2e --allow-dirty` — runs the real coverage/test gate in
     **Check mode** (verify-only, never mutates → no churn). `--allow-dirty` is required
     because at pre-commit time the staged commit content makes the tree "dirty" by
     validate's strict definition; the refusal is meant for pre-push, not here.

  The obvious single-pass `cargo xtask check` is **not** usable at pre-commit today:
  its Fix-mode coverage step regenerates `coverage-baseline.json` / `crap-manifest.json`
  on essentially every run (the manifests are not yet idempotent — #7), which would make
  the fail-and-restage diff fire on every commit and entangle the hook with regen
  behavior the coverage work owns. The two-pass split avoids touching the manifests at
  the cost of running fmt + clippy twice (coverage, the expensive part, runs once).
- **Pre-push hook → `cargo xtask validate --no-e2e`.** Its value is the clean-tree
  backstop below, not a re-verify of `check`.
- **`validate` refuses a dirty working tree** (`git status --porcelain` non-empty,
  including untracked non-gitignored files) unless `--allow-dirty`. `check` does not —
  Fix-mode is meant to run on a dirty tree. This makes pre-push the one point that
  proves *what was measured == the committed tip == what CI sees, nothing uncommitted
  hiding* — a guarantee `check` structurally cannot give.
- **Self-healing install:** any `cargo xtask` run points `core.hooksPath` at the
  tracked, relative `.githooks` (so each worktree uses its own checkout).
- **e2e stays ship/CI-only.** CI runs full `validate` (with e2e) on every PR as the
  backstop; no hook runs e2e.

## Consequences

- Commits run the full coverage build (slower commits) in exchange for a per-commit
  green history and a warm coverage cache that makes pre-push a near-instant cache hit.
- The two-pass pre-commit hook is a stopgap: once coverage auto-healing (#86) makes
  `check` idempotent on a clean tree, the hook collapses to a single `cargo xtask check`
  with the same porcelain-diff fail-and-restage, dropping the duplicated fmt/clippy pass.
- The dirty-tree refusal neutralizes #37's untracked-instrumentation footgun on the
  gate path without changing the flake source filter (#37 remains open for the
  flake-side contract).
- `SKIP_PRE_COMMIT` / `SKIP_PRE_PUSH` and `--allow-dirty` remain as deliberate local
  escapes; CI is the non-bypassable authority.
