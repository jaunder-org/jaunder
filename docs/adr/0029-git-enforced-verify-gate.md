# ADR-0029: Git-Enforced Verify Gate — Hook-Routed `check`/`validate` and Clean-Tree Gating

- Status: accepted

## Context

The verify gate (`cargo xtask check` / `validate`) was agent discipline, not
machine-enforced, so it was skipped, misread, or defensively over-run — most
expensively by running the full `validate` (with the ~18-minute e2e VMs) per
commit when the change could not affect e2e. `.githooks/` held an obsolete,
uninstalled pre-commit hook that bypassed xtask (raw
`leptosfmt`/`fmt`/`prettier`/`clippy`/ `nextest`), and `core.hooksPath` still
pointed at the default `.git/hooks`.

## Decision

- **Pre-commit hook → a single `cargo xtask check`** (fmt/leptosfmt/prettier +
  clippy + the Nix coverage/test gate, all in **Fix mode** with auto-heal). If
  the run changed the tree — a reformat, or a genuine coverage-baseline / CRAP
  heal — the hook **fails and asks the author to restage** rather than silently
  folding the fix into their commit. Detection is a `git status --porcelain`
  before/after diff.

  This single pass is safe because the Fix-mode heal is **idempotent on a clean
  tree**: the accepted-uncovered baseline compares by a line-independent text
  fingerprint and is rewritten only when it genuinely differs, the CRAP manifest
  ignores line attribution (#7), and a benign pure line-shift self-heals to a
  hint via re-anchor (#86) instead of churning the file (#113). So `check`
  mutates the tree only on a **real** change, never on every run — the
  fail-and-restage fires only when there is something genuine to restage. (This
  replaced an earlier two-pass stopgap — `check --no-test` +
  `validate --no-e2e --allow-dirty` — that ran fmt/clippy twice to avoid
  touching the then-non-idempotent manifests; see Consequences.)

- **Pre-push hook → `cargo xtask validate --no-e2e`.** Its value is the
  clean-tree backstop below, not a re-verify of `check`.
- **`validate` refuses a dirty working tree** (`git status --porcelain`
  non-empty, including untracked non-gitignored files) unless `--allow-dirty`.
  `check` does not — Fix-mode is meant to run on a dirty tree. This makes
  pre-push the one point that proves _what was measured == the committed tip ==
  what CI sees, nothing uncommitted hiding_ — a guarantee `check` structurally
  cannot give.
- **Self-healing install:** any `cargo xtask` run points `core.hooksPath` at the
  tracked, relative `.githooks` (so each worktree uses its own checkout).
- **e2e stays ship/CI-only.** CI runs full `validate` (with e2e) on every PR as
  the backstop; no hook runs e2e.

## Consequences

- Commits run the full coverage build (slower commits) in exchange for a
  per-commit green history and a warm coverage cache that makes pre-push a
  near-instant cache hit.
- The pre-commit hook was collapsed to a single `cargo xtask check` (#113) once
  the Fix-mode heal became idempotent on a clean tree (#7 line-agnostic CRAP,
  #86 re-anchor safety, #113 line-as-hint baseline heal), dropping the earlier
  two-pass stopgap and its duplicated fmt/clippy pass. A clean-tree commit no
  longer triggers fail-and-restage from manifest churn.
- The dirty-tree refusal neutralizes #37's untracked-instrumentation footgun on
  the gate path without changing the flake source filter (#37 remains open for
  the flake-side contract).
- `SKIP_PRE_COMMIT` / `SKIP_PRE_PUSH` and `--allow-dirty` remain as deliberate
  local escapes; CI is the non-bypassable authority.

## Supplement (#103): merge-driver self-heal

The keep-ours merge driver for the generated coverage artifacts
(`coverage-baseline.json`, `crap-manifest.json`; `.gitattributes` →
`merge=coverage-keepours`) now self-heals on the same path as `core.hooksPath`:
every `cargo xtask` run calls `ensure_merge_driver_installed()`, which
idempotently registers `merge.coverage-keepours.driver=true` in the clone's
local git config when unset/wrong. This closes the last gap where local git
config — not version-controlled — depended on an operator remembering a manual
one-shot: a fresh clone now wires the driver on first gate run, and because the
config is shared per-clone it covers all worktrees. The manual
`cargo xtask install-merge-driver` subcommand is removed as redundant; the
reusable `register_keepours()` helper remains and is the call the self-heal
makes.

No `post-merge` re-heal hook is added (deliberately). Re-healing the
baseline/manifest to the merged tree requires a full Nix-instrumented
`cargo xtask check` — there is no cheap re-heal — and a `post-merge` hook fires
on every `git merge`/`git pull`, including merges that touch nothing
coverage-related, so eager re-heal would mean a heavy coverage run after every
pull. Keep-ours already leaves a valid our-side baseline that the next
pre-commit `cargo xtask check` re-heals lazily; lazy re-heal is sufficient and
the eager cost is not justified.
