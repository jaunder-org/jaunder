# ADR-0029: Git-Enforced Verify Gate — Hook-Routed `check`/`validate` and Clean-Tree Gating

- Status: accepted

Historical note: the original Decision below recorded the #99/#113 hook shape.
#1079 and the later fast-local pre-push change amended the current hook
contract; see the final supplement and
[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md#the-verify-ladder--git-enforced-gate).

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

- **Pre-push hook → `cargo xtask validate --no-e2e`.** In the original hook
  shape, its value was the clean-tree backstop below, not a re-verify of
  `check`.
- **`validate` refuses a dirty working tree** (`git status --porcelain`
  non-empty, including untracked non-gitignored files) unless `--allow-dirty`.
  `check` does not — Fix-mode is meant to run on a dirty tree. In that original
  hook shape, pre-push was the point that proved _what was measured == the
  committed tip, nothing uncommitted hiding_ — a guarantee `check` structurally
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

In #103 the keep-ours merge driver for the generated coverage artifacts
(`coverage-baseline.json`, `crap-manifest.json`; `.gitattributes` →
`merge=coverage-keepours`) was made to self-heal on the same path as
`core.hooksPath`: every `cargo xtask` run called
`ensure_merge_driver_installed()`, which idempotently registered
`merge.coverage-keepours.driver=true` in the clone's local git config when
unset/wrong. This closed the last gap where local git config — not
version-controlled — depended on an operator remembering a manual one-shot: a
fresh clone thereafter wired the driver on first gate run, and because the
config is shared per-clone it covered all worktrees. The manual
`cargo xtask install-merge-driver` subcommand was removed as redundant; the
reusable `register_keepours()` helper remained and was the call the self-heal
made.

No `post-merge` re-heal hook was added (deliberately). Re-healing the
baseline/manifest to the merged tree requires a full Nix-instrumented
`cargo xtask check` — there is no cheap re-heal — and a `post-merge` hook fires
on every `git merge`/`git pull`, including merges that touch nothing
coverage-related, so eager re-heal would mean a heavy coverage run after every
pull. Keep-ours already leaves a valid our-side baseline that the next
pre-commit `cargo xtask check` re-heals lazily; lazy re-heal is sufficient and
the eager cost is not justified.

## Supplement (#1079): staged-subset pre-commit

In #1079 the pre-commit hook was rerouted from `cargo xtask check` to
`cargo xtask precommit` through the locked Cargo alias (`cargo run --locked`) so
Cargo could not rewrite `xtask/Cargo.lock` before the precommit snapshot. The
new command ran the fast Fix-mode host surface equivalent to
`cargo xtask check --no-test`, then reconciled Git/index state in Rust. It
re-staged only formatter/check mutations to already-staged tracked paths with no
pre-existing unstaged change. Mixed tracked paths, newly-created untracked
files, and delete/rename states changed during the hook failed closed with
diagnostics; pre-existing delete/rename state and untracked files stayed
unstaged and tolerated.

This traded the original per-commit Nix coverage/doctest/wasm proof for a short
commit-time gate plus a clean-tree pre-push proof. In #1113 the product Rust
test portion moved to host-native `cargo xtask test-local`; the follow-up
introduced `cargo xtask prepush` so the hook uses that fast local lane. The
slower `cargo xtask validate --no-e2e` command remains available locally for
hermetic confidence, and CI remains the non-bypassable hermetic backstop. The
current materialized gate shape lives in
[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md#the-verify-ladder--git-enforced-gate).

## Supplement (2026-08-31, #1117): fast local parity by failure surface

[#1117](https://github.com/jaunder-org/jaunder/issues/1117) keeps
`cargo xtask prepush` a fast, non-hermetic, clean-tree-gated early-failure lane:
it invokes no Nix derivation. It runs the existing verify-only host/static
surface, the existing auxiliary `xtask`/`tools` non-doc tests, the host-native
product Rust tests, and `workspace-doctests`: exactly
`cargo test --workspace --doc` plus the complete bidirectional doctest-fence
census and reconciliation. The auxiliary tests remain exactly once, and root
doctests run after the product tests.

This is parity by failure surface, not an assertion that `prepush` and
`validate --no-e2e` execute the same tests in different environments. The local
`workspace-doctests` verdict is authoritative for host-toolchain example
execution and fence reconciliation. The Nix doctest producer/gate remains the
authority for its pinned sandbox/offline environment, even though both paths
reconcile the same root-workspace fence population in both directions.

The following surfaces remain hermetic-only, under explicit
`cargo xtask validate --no-e2e` or CI authority:

- Nix static proof, because only its sandboxed offline Cargo environment proves
  the hermetic static definitions.
- Rust coverage/CRAP, because its instrumentation and SQLite/PostgreSQL backend
  parity are part of the verdict.
- Wasm browser tests, because they exercise browser primitives unavailable to a
  cheap host lane.
- Elisp coverage, because its coverage VM is the authoritative execution
  environment.
- The wasm budget, because its verdict is defined by the Nix-built artifact's
  size semantics.

Full `cargo xtask validate` additionally owns server-function flow verification
through the e2e path. That is outside the `validate --no-e2e` comparator, not a
missing prepush surface. The materialized per-surface contract is in
[`docs/ARCHITECTURE.md`](../ARCHITECTURE.md#prepush-parity-by-failure-surface).

## Supplement (2026-08-31, #1122): fail-fast local hook execution

[#1122](https://github.com/jaunder-org/jaunder/issues/1122) refines the local
hook execution policy without changing command membership, order, or authority.
`precommit` and `prepush` select orchestration-owned fail-fast execution: at
each ordered boundary — individual static checks, host-gate steps, and prepush
phases — the first newly appended failed, non-skipped step stops later work.
Unexecuted work is absent from the command result; it is not represented as a
green skipped step. The failed step retains its ordinary diagnostic and the
normal failed-command summary.

`prepush` retains the clean-tree precondition as its first boundary, so a dirty
tree prevents every gate phase from starting. `precommit` still always takes its
after-snapshot and performs its conservative Git/index staging reconciliation
after gate execution, including after an early failure: safe formatter/check
mutations remain eligible for restaging, while mixed, ambiguous, or otherwise
unsafe state continues to fail closed. This does not weaken clean-tree or
staged-subset authority.

The explicit `check` and `validate` commands, and CI's corresponding surfaces,
remain exhaustive: they continue through the unchanged ordered diagnostic graph
and retain their existing authority. The #1117 prepush parity-by-failure-surface
contract remains unchanged; this supplement changes only when the local hook
stops, not which surfaces it covers or which hermetic verdict CI owns.
