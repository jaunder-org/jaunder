# Issue #103 — Auto-register the coverage keep-ours merge driver

**Status:** approved
**Issue:** [#103](https://github.com/jaunder-org/jaunder/issues/103) — _tooling: auto-register the coverage keep-ours merge driver_
**Milestone:** 1 — Verify-gate hardening
**Builds on:** #86 / #7 (keep-ours driver + `.gitattributes`), #99 / ADR-0029 (git-enforced gate, self-healing `core.hooksPath`)

## Problem

#86/#7 added a keep-ours git merge driver for the generated coverage artifacts
(`coverage-baseline.json`, `crap-manifest.json`): committed `.gitattributes` map them to
`merge=coverage-keepours`, and a one-shot `cargo xtask install-merge-driver` registers the
driver in local git config. Git config is **not** version-controlled, so the driver only
takes effect once someone runs that one-shot. A fresh clone — or anyone who never ran it —
still hits conflict markers on `coverage-baseline.json` / `crap-manifest.json` when
overlapping branches merge.

The asymmetry is the bug: `core.hooksPath` **self-heals** on every `cargo xtask` run
(`ensure_hooks_installed()` → `git::ensure_hooks_path`), but the merge driver does **not** —
it is the lone piece of local git config still gated on the operator remembering a manual
step. Confirmation of the gap: `git config --get merge.coverage-keepours.driver` returns
nothing in the main clone today — the one-shot was apparently never run, so merges there
would still conflict.

## Scope of fix

Fold merge-driver registration into the same self-healing path that already wires
`core.hooksPath`, so every clone wires itself up on first gate run.

Settled design decisions (with rationale):

- **Auto-register on every `cargo xtask` run** — mirror `ensure_hooks_path`. Idempotent
  (a cheap `git config --get` check gated before any write), so it adds no churn and
  negligible cost despite running on every invocation across all worktrees.
- **No post-merge re-heal hook.** Re-healing the baseline/manifest requires a full
  Nix-instrumented `cargo xtask check` — there is no cheap re-heal, because true
  merged-tree coverage is unknowable without running it. A `post-merge` hook fires on
  every `git merge` **and every `git pull`**, including merges that touch nothing
  coverage-related, so eager re-heal = a heavy Nix run after every pull. Meanwhile
  keep-ours already leaves a valid (our-side) baseline that the next pre-commit
  `cargo xtask check` re-heals lazily. Lazy re-heal is sufficient; the eager version's
  cost is not justified. (#103 lists the hook as explicitly optional.)
- **Remove the now-redundant `install-merge-driver` subcommand.** Once registration
  self-heals, the manual one-shot is pure redundancy. Removing it shrinks the surface and
  removes a discoverability trap (a command that looks required but no longer is). The
  reusable `register_keepours()` helper stays — the self-heal calls it.

### Why per-clone, not per-worktree

`merge.*.driver` lives in the clone's **shared** `.git/config` (`extensions.worktreeConfig`
is not enabled here), so every worktree of a clone reads the same registration —
register once per clone and all worktrees, including ones created later, inherit it. The
thing that genuinely needs re-wiring is a **fresh clone** (new `.git/config`), and the
self-heal makes that automatic on first `cargo xtask` run. Same scoping as `core.hooksPath`.

## Design

### 1. Self-heal logic (`xtask/src/lib.rs`, mirroring `git::ensure_hooks_path`)

- `needs_merge_driver(current: Option<&str>) -> bool` — `true` when the current
  `merge.coverage-keepours.driver` value is unset or `trim() != "true"`. Pure, unit-tested
  (mirrors `needs_hooks_path`).
- A reader for `merge.coverage-keepours.driver` via the existing env-scrubbing `git_at`
  (scrubbing is load-bearing — the self-heal runs inside hooks, where ambient
  `GIT_DIR`/etc. would otherwise redirect the config write at the wrong repo).
- `ensure_merge_driver(repo_dir: &Path) -> Result<bool>` — if `needs_merge_driver`, call the
  existing `register_keepours(repo_dir)`; return whether config changed. Tested on a
  throwaway repo.
- `ensure_merge_driver_installed()` — thin best-effort wrapper over `"."`, parallel to
  `ensure_hooks_installed()`. Logs `xtask: registered merge.coverage-keepours` on change,
  a warning on failure, nothing on no-op. Never blocks the command.

### 2. Wire-up (`xtask/src/main.rs`)

Call `ensure_merge_driver_installed()` immediately after `ensure_hooks_installed()` (line 7),
unconditionally for all subcommands.

### 3. Remove the manual command (`xtask/src/lib.rs`)

Delete: the `InstallMergeDriver` enum variant (~72–78), its `command_name` match arm (~88),
its `run` dispatch arm (~140–146), and the `install_merge_driver()` fn (~257–263). Keep
`register_keepours()` and `git_at()` (now reached only via the self-heal) and their existing
tests (`git_at_scrubs_repo_redirecting_env`,
`keepours_driver_resolves_merge_to_ours_without_markers`).

### 4. Docs

- **`.gitattributes`** header comment: replace "Register the driver once per clone/worktree
  with: `cargo xtask install-merge-driver`" → note the driver auto-registers on any
  `cargo xtask` run (self-healing, like `core.hooksPath`).
- **`CONTRIBUTING.md:192`**: same correction to the sentence describing driver registration.
- **ADR-0029** (`docs/adr/0029-git-enforced-verify-gate.md`): add a `## Supplement (#103)`
  section — the merge driver now self-heals alongside `core.hooksPath`; the manual
  `install-merge-driver` command is removed; and the deliberate decision **not** to add a
  post-merge hook, with the heavy-re-heal-vs-sufficient-lazy-re-heal rationale. No
  `docs/README.md` table change (ADR-0029 stays `accepted`).

## Testing

- `needs_merge_driver` unit tests: unset (`None`), wrong (`Some("false")`, `Some("")`),
  correct (`Some("true")`, `Some(" true \n")`) — mirroring the `needs_hooks_path` tests.
- `ensure_merge_driver` on a throwaway git repo: first call returns `true` and leaves
  `merge.coverage-keepours.driver == "true"`; second call returns `false` (idempotent).
- Existing `keepours_driver_resolves_merge_to_ours_without_markers` end-to-end behavior test
  is unchanged and still proves the driver resolves a merge to our side without markers.
- Per-task gate: `cargo xtask check --no-test` (clippy + fmt). Final gate before commit:
  `cargo xtask validate` (or `validate --no-e2e` per the autonomous-gate policy).

## Out of scope

- No post-merge hook (see rationale above).
- No change to the flake source filter or the `core.hooksPath` relative/absolute behavior
  (a separate latent inconsistency, not this issue).
- ADR-0029's pre-commit description already drifted from the #113 single-pass collapse;
  reconciling that is #113's concern, not touched here beyond the appended supplement.
