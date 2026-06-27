# Git-Enforced Dev Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make git mechanically enforce the right verify gate — `check` at pre-commit, `validate --no-e2e` at pre-push — and make `validate` refuse a dirty tree, so the gate is trustworthy and e2e stays ship/CI-only.

**Architecture:** A new `xtask` git-helper module supplies two pure, unit-tested predicates (working-tree dirtiness; `core.hooksPath` correctness) plus thin shell wrappers. `validate` gains a clean-tree precheck (`--allow-dirty` escape). `main` self-heals `core.hooksPath → .githooks` on every run. Two thin bash hooks under `.githooks/` route to xtask. An ADR records the contract.

**Tech Stack:** Rust (clap, xshell, anyhow), bash hooks, git config.

## Global Constraints

- **No `Co-Authored-By` trailers** on any commit.
- **xtask is NOT coverage-instrumented** — the flake excludes `xtask/`, so new xtask code faces no coverage-baseline gate; still unit-test it. Tests run via `host_tests` (`cargo test --manifest-path xtask/Cargo.toml`) in every mode.
- **No new dependencies** — tests use pure functions + clap parsing only (no `tempfile`, no shelling git in tests).
- **Dirty = `git status --porcelain` non-empty**, which includes untracked non-gitignored files and excludes gitignored paths. Only `validate` refuses a dirty tree; `check` (Fix-mode) must still run on one.
- **`core.hooksPath` is relative `.githooks`** (not absolute), so each worktree resolves its own checkout.
- **e2e stays ship/CI-only** — no hook runs e2e.
- Per-task inner loop while fixing: `cargo xtask check --no-test`. Run new unit tests directly with `cargo test --manifest-path xtask/Cargo.toml <name>`.

## File Structure

- `xtask/src/git.rs` *(new)* — git helpers: `porcelain_is_dirty`, `needs_hooks_path` (pure, tested); `working_tree_status`, `hooks_path`, `ensure_hooks_path` (shell wrappers); `HOOKS_PATH` const. Exposed as `pub mod git` so its helpers (consumed in Tasks 2/5) are crate API and do not trip `dead_code` under clippy `-D warnings` when the module lands ahead of its consumers.
- `xtask/src/lib.rs` *(modify)* — register `pub mod git;`; add `--allow-dirty` to `Validate`; add `clean_tree_precheck` + wire it into the `Validate` arm; add `ensure_hooks_installed()`.
- `xtask/src/main.rs` *(modify)* — call `xtask::ensure_hooks_installed()` before dispatch.
- `.githooks/pre-commit` *(rewrite)* — `cargo xtask check` + fail-and-restage + `SKIP_PRE_COMMIT`.
- `.githooks/pre-push` *(confirm; no change expected)* — `cargo xtask validate --no-e2e` + `SKIP_PRE_PUSH`.
- `docs/adr/0029-git-enforced-verify-gate.md` *(new)*; `docs/README.md` *(modify — add ADR row)*.
- `~/.claude/skills/jaunder-{commit,iterate,dispatch,ship}/SKILL.md` and the auto-memory note *(modify — OUT OF REPO, not in the PR)*.

---

### Task 1: git-helper module (predicates + wrappers)

**Files:**
- Create: `xtask/src/git.rs`
- Modify: `xtask/src/lib.rs` (add `mod git;`)
- Test: in-file `#[cfg(test)]` in `xtask/src/git.rs`

**Interfaces:**
- Produces: `git::HOOKS_PATH: &str`; `git::porcelain_is_dirty(&str) -> bool`; `git::needs_hooks_path(Option<&str>) -> bool`; `git::working_tree_status(&Shell) -> anyhow::Result<String>`; `git::hooks_path(&Shell) -> Option<String>`; `git::ensure_hooks_path(&Shell) -> anyhow::Result<bool>`. Module is `pub mod git` (crate API) so helpers landing ahead of their Task 2/5 consumers don't trip `dead_code`.

- [ ] **Step 1: Write the failing tests** — create `xtask/src/git.rs` with only the tests + a `use super::*;`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_blank_is_clean() {
        assert!(!porcelain_is_dirty(""));
        assert!(!porcelain_is_dirty("\n"));
        assert!(!porcelain_is_dirty("   \n  \n"));
    }

    #[test]
    fn porcelain_untracked_is_dirty() {
        assert!(porcelain_is_dirty("?? new_file.rs"));
    }

    #[test]
    fn porcelain_staged_or_modified_is_dirty() {
        assert!(porcelain_is_dirty(" M src/lib.rs"));
        assert!(porcelain_is_dirty("A  staged.rs"));
        assert!(porcelain_is_dirty("?? a\n M b"));
    }

    #[test]
    fn needs_hooks_path_when_unset_or_wrong() {
        assert!(needs_hooks_path(None));
        assert!(needs_hooks_path(Some(".git/hooks")));
        assert!(needs_hooks_path(Some("")));
    }

    #[test]
    fn no_need_when_hooks_path_already_correct() {
        assert!(!needs_hooks_path(Some(".githooks")));
        assert!(!needs_hooks_path(Some(" .githooks \n")));
    }
}
```

- [ ] **Step 2: Register the module** — in `xtask/src/lib.rs`, add `pub mod git;` alongside the other top-level `mod` lines (after `mod coverage;`). It must be `pub` so the helpers (whose consumers land in Tasks 2/5) count as crate API and don't trip `dead_code` under clippy `-D warnings`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --manifest-path xtask/Cargo.toml git::`
Expected: FAIL to compile — `porcelain_is_dirty` / `needs_hooks_path` not found.

- [ ] **Step 4: Implement the module** — prepend above the `#[cfg(test)]` block in `xtask/src/git.rs`:

```rust
//! Git helpers for the verify gate: working-tree cleanliness (the `validate`
//! backstop) and self-healing `core.hooksPath` installation.

use anyhow::{Context, Result};
use xshell::{cmd, Shell};

/// Repo-relative hooks directory the gate routes git to. Relative (not absolute)
/// so each worktree resolves to its own `.githooks` checkout.
pub const HOOKS_PATH: &str = ".githooks";

/// True when `git status --porcelain` output denotes a dirty tree. Porcelain lists
/// staged + unstaged tracked changes AND untracked non-gitignored files (`??`), and
/// omits gitignored paths — exactly the surface the Nix coverage source picks up.
/// Any non-blank line means dirty.
pub fn porcelain_is_dirty(porcelain: &str) -> bool {
    porcelain.lines().any(|line| !line.trim().is_empty())
}

/// Whether `core.hooksPath` needs (re)pointing at [`HOOKS_PATH`], given its current
/// value (`None` = unset).
pub fn needs_hooks_path(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != HOOKS_PATH,
        None => true,
    }
}

/// `git status --porcelain` text. Errors only if git itself cannot run.
pub fn working_tree_status(sh: &Shell) -> Result<String> {
    cmd!(sh, "git status --porcelain")
        .quiet()
        .read()
        .context("running `git status --porcelain`")
}

/// True when the working tree has any staged, unstaged, or untracked
/// (non-gitignored) change.
pub fn is_working_tree_dirty(sh: &Shell) -> Result<bool> {
    Ok(porcelain_is_dirty(&working_tree_status(sh)?))
}

/// Current `core.hooksPath`, or `None` when unset/blank. `--get` exits non-zero when
/// the key is missing, so the status is ignored and an empty read maps to `None`.
pub fn hooks_path(sh: &Shell) -> Option<String> {
    cmd!(sh, "git config --get core.hooksPath")
        .quiet()
        .ignore_status()
        .read()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Ensure `core.hooksPath` points at [`HOOKS_PATH`]; set it if unset/wrong. Returns
/// `true` when it changed the config.
pub fn ensure_hooks_path(sh: &Shell) -> Result<bool> {
    if needs_hooks_path(hooks_path(sh).as_deref()) {
        cmd!(sh, "git config core.hooksPath {HOOKS_PATH}")
            .quiet()
            .run()
            .context("setting core.hooksPath")?;
        Ok(true)
    } else {
        Ok(false)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml git::`
Expected: PASS (5 tests).

- [ ] **Step 6: Lint**

Run: `cargo xtask check --no-test`
Expected: PASS (`[ ok ]` for the static/clippy steps).

- [ ] **Step 7: Commit**

```bash
git add xtask/src/git.rs xtask/src/lib.rs
git commit -m "feat(xtask): git helpers for dirty-tree + hooksPath (issue-99)"
```

---

### Task 2: dirty-tree refusal on `validate` (`--allow-dirty`)

**Files:**
- Modify: `xtask/src/lib.rs`
- Test: in-file `#[cfg(test)]` in `xtask/src/lib.rs`

**Interfaces:**
- Consumes: `git::working_tree_status`, `git::porcelain_is_dirty` (Task 1); `StepResult`, `CommandResult`, `Mode`.
- Produces: `Command::Validate { no_e2e, allow_dirty }`; `fn clean_tree_precheck(&Shell, bool) -> StepResult`.

- [ ] **Step 1: Write the failing test** — add to (or create) the `#[cfg(test)]` module in `xtask/src/lib.rs`:

```rust
#[cfg(test)]
mod cli_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn validate_allow_dirty_parses() {
        let cli = Cli::try_parse_from(["xtask", "validate", "--allow-dirty"]).unwrap();
        match cli.command {
            Command::Validate { no_e2e, allow_dirty } => {
                assert!(!no_e2e);
                assert!(allow_dirty);
            }
            _ => panic!("expected validate"),
        }
    }

    #[test]
    fn validate_defaults_reject_dirty() {
        let cli = Cli::try_parse_from(["xtask", "validate"]).unwrap();
        match cli.command {
            Command::Validate { allow_dirty, .. } => assert!(!allow_dirty),
            _ => panic!("expected validate"),
        }
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --manifest-path xtask/Cargo.toml cli_tests::`
Expected: FAIL to compile — `Command::Validate` has no `allow_dirty` field.

- [ ] **Step 3: Add the flag** — in `xtask/src/lib.rs`, extend the `Validate` variant:

```rust
    /// Full gate (never mutates the tree): static + clippy + the host xtask unit
    /// suite (verify-only) + the Nix coverage check + the e2e VMs. `--no-e2e` skips
    /// the e2e VMs. Refuses a dirty working tree unless `--allow-dirty`.
    Validate {
        /// Skip the e2e VM checks — static + clippy + xtask tests + coverage only.
        #[arg(long)]
        no_e2e: bool,
        /// Run even when the working tree is dirty (skip the clean-tree precheck).
        #[arg(long)]
        allow_dirty: bool,
    },
```

- [ ] **Step 4: Add the precheck helper** — in `xtask/src/lib.rs`, near `finalize`, add:

```rust
/// The clean-tree precheck step for `validate`. With `--allow-dirty`, a skip.
/// Otherwise: `ok` when the tree is clean; `fail` when dirty (detail = the porcelain
/// status) or when git cannot be queried — the gate refuses to certify a tree it
/// cannot prove clean. `check` deliberately has no such precheck (Fix-mode runs on a
/// dirty tree by design).
fn clean_tree_precheck(sh: &xshell::Shell, allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(sh) {
        Ok(status) if git::porcelain_is_dirty(&status) => StepResult::fail("clean-tree").detail(
            format!(
                "working tree is dirty — commit/stash or pass --allow-dirty:\n{}",
                status.trim()
            ),
        ),
        Ok(_) => StepResult::ok("clean-tree"),
        Err(e) => StepResult::fail("clean-tree")
            .detail(format!("could not determine cleanliness: {e:#}")),
    }
}
```

- [ ] **Step 5: Wire it into the `Validate` arm** — replace the `Command::Validate { no_e2e } => { ... }` arm body with:

```rust
        Command::Validate { no_e2e, allow_dirty } => {
            let sh = xshell::Shell::new()?;
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("validate");
            let step = clean_tree_precheck(&sh, allow_dirty);
            let blocked = !step.ok && !step.skipped;
            result.push(step);
            if blocked {
                finalize(&mut result, start);
                return Ok(result);
            }
            steps::static_checks::run(&sh, Mode::Check, &mut result);
            steps::host_tests::run(&sh, &mut result);
            steps::nix::coverage(&mut result, Mode::Check);
            if !no_e2e {
                steps::nix::e2e(&mut result);
            }
            finalize(&mut result, start);
            Ok(result)
        }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml`
Expected: PASS (Task 1 + cli_tests).

- [ ] **Step 7: Manually confirm the refusal** — with the spec already committed and tree clean, create an untracked file and run validate skipping the expensive steps via `--allow-dirty` first to prove the escape, then without:

```bash
cargo xtask validate --allow-dirty --no-e2e   # clean-tree -> [skip]; proceeds (may take minutes — Ctrl-C ok once you see the skip line)
touch xtask/__dirty_probe
cargo xtask validate --no-e2e                 # clean-tree -> [FAIL], lists ?? xtask/__dirty_probe, exits 1 immediately
rm xtask/__dirty_probe
```

Expected: the second run fails fast at `clean-tree` before any static/coverage step.

- [ ] **Step 8: Lint + commit**

```bash
cargo xtask check --no-test
git add xtask/src/lib.rs
git commit -m "feat(xtask): validate refuses a dirty tree, add --allow-dirty (issue-99)"
```

---

### Task 3: rewrite `.githooks/pre-commit`; confirm `.githooks/pre-push`

**Files:**
- Modify: `.githooks/pre-commit` (full rewrite)
- Confirm (no change expected): `.githooks/pre-push`

**Interfaces:** none (bash). Hooks are inert until Task 5 installs them.

- [ ] **Step 1: Rewrite `.githooks/pre-commit`** with exactly:

```bash
#!/usr/bin/env bash
# Pre-commit gate: full `cargo xtask check` (static + clippy + xtask units + Nix
# coverage, Fix-mode). check may rewrite files (fmt, clippy, coverage-baseline); if it
# does, fail-and-restage — abort so the author stages the fixes consciously rather than
# silently folding them in. Happy path (check already run before committing): the tree
# is unchanged, so this is a fast cache-hit pass. SKIP_PRE_COMMIT=1 bypasses for WIP.
set -euo pipefail

if [ "${SKIP_PRE_COMMIT:-0}" = "1" ]; then
    echo "--- pre-commit: SKIP_PRE_COMMIT=1, skipping ---"
    exit 0
fi

echo "--- pre-commit: cargo xtask check ---"
pre=$(git status --porcelain)
cargo xtask check
post=$(git status --porcelain)

if [ "$pre" != "$post" ]; then
    echo "--- pre-commit: check applied fixes — review, git add, and re-commit ---" >&2
    exit 1
fi

echo "--- pre-commit: all checks passed ---"
```

- [ ] **Step 2: Confirm `.githooks/pre-push`** runs `cargo xtask validate --no-e2e` with the `SKIP_PRE_PUSH` escape (it already does — see current contents). No edit unless it has drifted. The dirty-tree refusal now lives inside `validate`, so pre-push needs no porcelain logic of its own.

- [ ] **Step 3: Keep them executable**

Run: `chmod +x .githooks/pre-commit .githooks/pre-push`
Expected: no output.

- [ ] **Step 4: Commit** (hooks still inert — `core.hooksPath` not yet set, so this commit is not self-gated)

```bash
git add .githooks/pre-commit
git commit -m "build(githooks): pre-commit runs cargo xtask check with fail-and-restage (issue-99)"
```

---

### Task 4: ADR-0029 + README table row

**Files:**
- Create: `docs/adr/0029-git-enforced-verify-gate.md`
- Modify: `docs/README.md` (add one table row after the 0028 row)

- [ ] **Step 1: Write the ADR** — create `docs/adr/0029-git-enforced-verify-gate.md`:

```markdown
# ADR-0029: Git-Enforced Verify Gate — Hook-Routed `check`/`validate` and Clean-Tree Gating

Status: accepted

## Context

The verify gate (`cargo xtask check` / `validate`) was agent discipline, not
machine-enforced, so it was skipped, misread, or defensively over-run — most
expensively by running the full `validate` (with the ~18-minute e2e VMs) per commit
when the change could not affect e2e. `.githooks/` held an obsolete, uninstalled
pre-commit hook that bypassed xtask, and an `core.hooksPath` that still pointed at the
default `.git/hooks`.

## Decision

- **Pre-commit hook → `cargo xtask check`** (full, Fix-mode, incl. the Nix coverage
  step that runs the app test suite). When `check` rewrites files it **fails and asks
  the author to restage** rather than silently folding the changes in.
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
- The dirty-tree refusal neutralizes #37's untracked-instrumentation footgun on the
  gate path without changing the flake source filter (#37 remains open for the
  flake-side contract).
- `SKIP_PRE_COMMIT` / `SKIP_PRE_PUSH` and `--allow-dirty` remain as deliberate local
  escapes; CI is the non-bypassable authority.
```

- [ ] **Step 2: Add the README row** — in `docs/README.md`, immediately after the `[0028]` row, add:

```markdown
| [0029](adr/0029-git-enforced-verify-gate.md) | Git-Enforced Verify Gate — Hook-Routed check/validate and Clean-Tree Gating | accepted |
```

- [ ] **Step 3: Commit**

```bash
git add docs/adr/0029-git-enforced-verify-gate.md docs/README.md
git commit -m "docs(adr): ADR-0029 git-enforced verify gate (issue-99)"
```

---

### Task 5: self-healing installer (activates the hooks)

**Files:**
- Modify: `xtask/src/lib.rs` (add `ensure_hooks_installed`)
- Modify: `xtask/src/main.rs` (call it before dispatch)

**Interfaces:**
- Consumes: `git::ensure_hooks_path`, `git::HOOKS_PATH` (Task 1).
- Produces: `pub fn ensure_hooks_installed()`.

> **Bootstrapping note:** the instant this builds and any `cargo xtask` runs, `core.hooksPath` is set and the Task 3 hooks go live — so this is the first commit that self-gates through the new pre-commit hook. That is intended. If the hook's full-`check` run is disruptive for *this* landing commit, prefix it with `SKIP_PRE_COMMIT=1` (shown below).

- [ ] **Step 1: Add the installer** — in `xtask/src/lib.rs`, add a public function (near `run`):

```rust
/// Self-healing hook installation: point `core.hooksPath` at `.githooks` if it is not
/// already, so fresh clones and new worktrees wire up on first run. Best-effort — a
/// failure here must never block the actual command.
pub fn ensure_hooks_installed() {
    let Ok(sh) = xshell::Shell::new() else {
        return;
    };
    match git::ensure_hooks_path(&sh) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
    }
}
```

- [ ] **Step 2: Call it from `main`** — in `xtask/src/main.rs`, after `let cli = Cli::parse();` and before `match run(cli)`:

```rust
    // Self-healing: wire core.hooksPath -> .githooks on every run (best-effort).
    xtask::ensure_hooks_installed();
```

- [ ] **Step 3: Verify it sets hooksPath idempotently**

Run: `git config --unset core.hooksPath 2>/dev/null; cargo xtask check --no-test; git config --get core.hooksPath`
Expected: the build prints `xtask: set core.hooksPath = .githooks` once; the final `git config --get` prints `.githooks`. A second `cargo xtask check --no-test` does NOT reprint the "set" line.

- [ ] **Step 4: Lint**

Run: `cargo xtask check --no-test`
Expected: PASS.

- [ ] **Step 5: Commit** (hooks are now live; the pre-commit hook will run full `check` — use the skip if its coverage run is disruptive here)

```bash
git add xtask/src/lib.rs xtask/src/main.rs
SKIP_PRE_COMMIT=1 git commit -m "feat(xtask): self-healing core.hooksPath install (issue-99)"
```

---

### Task 6: ripple — skills + memory (OUT OF REPO, not in the PR)

**Files (all outside the repo — edited in the user's config, NOT committed to the branch):**
- `~/.claude/skills/jaunder-commit/SKILL.md`
- `~/.claude/skills/jaunder-iterate/SKILL.md`
- `~/.claude/skills/jaunder-dispatch/SKILL.md`
- `~/.claude/skills/jaunder-ship/SKILL.md`
- `~/.config/claude/projects/-home-mdorman-src-jaunder/memory/feedback_autonomous_work_authorization.md` (+ its `MEMORY.md` pointer line)

> These are personal-config edits, not repository changes — do them as the last step before ship, and do NOT `git add` them to the branch. Use the native Edit tool.

- [ ] **Step 1: `jaunder-commit`** — change the per-commit gate from `validate --no-e2e` to "the pre-commit `check` hook enforces the gate; pushing runs `validate --no-e2e` via the pre-push hook." **Delete the "Bump to full `cargo xtask validate` only for a commit touching e2e-relevant surface" sentence** (the per-commit e2e loophole). Keep: e2e is ship/CI-only; the dirty-tree note; the rationalizations table (update the "floor is validate --no-e2e" row to "floor is the pre-commit `check` hook").

- [ ] **Step 2: `jaunder-iterate`** — step 4 ("Commit via jaunder-commit"): note the pre-commit hook now mechanically runs `check`; the inner fixing loop stays `cargo xtask check --no-test`.

- [ ] **Step 3: `jaunder-dispatch`** — no behavioral change (subagents don't commit, so the pre-commit hook never fires for them; they keep `cargo xtask check --no-test`). Add one line clarifying that the hooks gate the *controller's* commits, not subagent work.

- [ ] **Step 4: `jaunder-ship`** — step 1: note `validate` now refuses a dirty tree, so planning-doc archiving (step 3) and any last edits must be committed before the final full `validate`. Keep e2e at ship.

- [ ] **Step 5: memory note** — in `feedback_autonomous_work_authorization.md`, change "commit only after `cargo xtask validate --no-e2e` passes" to "commits are gated by the pre-commit `check` hook; pushing runs `validate --no-e2e` (pre-push hook); e2e is ship/CI-only." Update the one-line summary in `MEMORY.md` to match.

- [ ] **Step 6: confirm** the four skills and the memory note now describe the enforced gate with no remaining reference to per-commit e2e or to `validate --no-e2e` as the per-commit floor.

---

## Self-Review

**Spec coverage:** pre-commit `check` hook → Task 3/5; fail-and-restage → Task 3; pre-push `validate --no-e2e` kept → Task 3; dirty-tree refusal + `--allow-dirty` → Task 2; porcelain incl. untracked → Task 1/2; self-healing installer → Task 5; relative hooksPath → Task 1; e2e ship/CI-only → unchanged + ADR/Task 4; #37 interaction → ADR/Task 4; ripple skills+memory → Task 6; ADR-0029 + README → Task 4. All covered.

**Placeholder scan:** no TBD/TODO; every code step shows complete code; commands have expected output. Clean.

**Type consistency:** `porcelain_is_dirty`/`needs_hooks_path`/`working_tree_status`/`is_working_tree_dirty`/`hooks_path`/`ensure_hooks_path`/`HOOKS_PATH` defined in Task 1 and used unchanged in Tasks 2/5; `clean_tree_precheck` and `Command::Validate { no_e2e, allow_dirty }` defined and consumed within Task 2; `ensure_hooks_installed` defined Task 5, called in `main`. Consistent.

## Notes on ordering

Hooks are written (Task 3) and the ADR (Task 4) lands before the installer (Task 5), so when Task 5 activates `core.hooksPath` the live hooks are already the correct new versions. The installer is the last in-repo task so earlier dev commits are not hook-gated. Task 6 is out-of-repo and lands last, just before ship.
