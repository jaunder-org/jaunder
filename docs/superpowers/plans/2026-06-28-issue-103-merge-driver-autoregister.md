# Auto-register the coverage keep-ours merge driver (#103) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `coverage-keepours` git merge driver register itself on every `cargo xtask` run, mirroring the existing `core.hooksPath` self-heal, and remove the now-redundant manual `install-merge-driver` subcommand.

**Architecture:** Add a self-healing registration path in `xtask/src/lib.rs` that parallels `git::ensure_hooks_path` — a pure `needs_merge_driver` predicate, a config reader via the existing env-scrubbing `git_at`, an `ensure_merge_driver(repo_dir)` that calls the existing `register_keepours` when needed, and a best-effort `ensure_merge_driver_installed()` wrapper invoked from `main.rs` alongside `ensure_hooks_installed()`. Then delete the manual subcommand and update the docs.

**Tech Stack:** Rust, `clap` (xtask CLI), `anyhow`, `std::process::Command` (git invocation), `cargo nextest` (tests), `cargo xtask` (the dev/CI gate).

## Global Constraints

- **Verify ladder (git-enforced — ADR-0029):** while iterating, the per-task gate is `cargo xtask check --no-test` (fmt + clippy — fast feedback only). The commit itself is gated automatically by the **pre-commit hook**, which runs the full `cargo xtask check` (fmt + clippy + Nix coverage in Fix mode) and fails-and-restages if it heals/reformats anything — so history stays green commit-by-commit with no manual step. `cargo xtask validate --no-e2e` is the **pre-push** gate at the issue boundary (also hook-enforced); CI runs the full `cargo xtask validate` (with e2e). Run any manual gate from the worktree (`cd <worktree> &&` or the Bash tool) — context-mode runs against the main repo.
- **No Co-Authored-By trailers** in any commit.
- **Coverage policy:** testable logic must be covered. The thin `*_installed()` wrapper that operates on `"."` is best-effort glue (like `ensure_hooks_installed`) and is not unit-tested; the testable logic (`needs_merge_driver`, `ensure_merge_driver`) is covered on a throwaway repo.
- **Driver value is the shell builtin `true`** — `merge.coverage-keepours.driver` must equal exactly `"true"`.
- **`git_at` env-scrubbing is load-bearing** — the self-heal runs inside hooks; all git config ops on a `repo_dir` must go through `git_at`, never a raw `git`/`xshell` call that ambient `GIT_DIR` could redirect.

---

### Task 1: Self-healing merge-driver registration + wire-up

**Files:**
- Modify: `xtask/src/lib.rs` (add `needs_merge_driver`, `merge_driver_value`, `ensure_merge_driver`, `ensure_merge_driver_installed`; add tests to the existing `merge_driver_tests` module)
- Modify: `xtask/src/main.rs:6-7` (call the new wrapper after `ensure_hooks_installed()`)
- Commit also includes the planning docs created during start/brainstorming:
  `docs/superpowers/specs/2026-06-28-issue-103-merge-driver-autoregister-design.md`,
  `docs/superpowers/plans/2026-06-28-issue-103-merge-driver-autoregister.md`

**Interfaces:**
- Consumes (already present in `lib.rs`): `fn git_at(repo_dir: &std::path::Path) -> std::process::Command`; `fn register_keepours(repo_dir: &std::path::Path) -> anyhow::Result<()>`; `pub fn ensure_hooks_installed()`.
- Produces: `fn needs_merge_driver(current: Option<&str>) -> bool`; `fn merge_driver_value(repo_dir: &std::path::Path) -> Option<String>`; `fn ensure_merge_driver(repo_dir: &std::path::Path) -> anyhow::Result<bool>`; `pub fn ensure_merge_driver_installed()`.

- [x] **Step 1: Write the failing tests**

Add to the existing `mod merge_driver_tests` in `xtask/src/lib.rs` (it already provides the `git` and `git_stdout` helpers and `use super::{git_at, register_keepours};`). Add `needs_merge_driver` and `ensure_merge_driver` to the import line, then append:

```rust
    #[test]
    fn needs_merge_driver_when_unset_or_wrong() {
        assert!(needs_merge_driver(None));
        assert!(needs_merge_driver(Some("")));
        assert!(needs_merge_driver(Some("false")));
    }

    #[test]
    fn no_need_when_merge_driver_already_true() {
        assert!(!needs_merge_driver(Some("true")));
        assert!(!needs_merge_driver(Some(" true \n")));
    }

    #[test]
    fn ensure_merge_driver_registers_then_is_idempotent() {
        let tmp =
            std::env::temp_dir().join(format!("jaunder-ensure-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        git(&tmp, &["init", "-q"]);
        // First call registers and reports a change.
        assert!(ensure_merge_driver(&tmp).unwrap(), "first call registers");
        assert_eq!(
            git_stdout(&tmp, &["config", "--get", "merge.coverage-keepours.driver"]),
            "true"
        );
        // Second call is a no-op (idempotent).
        assert!(!ensure_merge_driver(&tmp).unwrap(), "second call is a no-op");

        let _ = std::fs::remove_dir_all(&tmp);
    }
```

Update the module's import line from:

```rust
    use super::{git_at, register_keepours};
```

to:

```rust
    use super::{ensure_merge_driver, git_at, needs_merge_driver, register_keepours};
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cd /home/mdorman/src/jaunder/.claude/worktrees/issue-103-merge-driver-autoregister && cargo nextest run -p xtask merge_driver_tests`
Expected: FAIL — `cannot find function/value needs_merge_driver` / `ensure_merge_driver` (not yet defined).

- [x] **Step 3: Implement the self-heal helpers**

In `xtask/src/lib.rs`, immediately after `register_keepours` (currently ending ~line 255), add:

```rust
/// Whether the `coverage-keepours` merge driver needs (re)registering, given the
/// current `merge.coverage-keepours.driver` value (`None` = unset). The driver
/// command is the shell builtin `true`; any other value (or unset) means re-register.
fn needs_merge_driver(current: Option<&str>) -> bool {
    match current {
        Some(value) => value.trim() != "true",
        None => true,
    }
}

/// Current `merge.coverage-keepours.driver` in `repo_dir`, or `None` when unset/blank.
/// `git config --get` exits non-zero (empty stdout) when the key is missing, so a
/// blank read maps to `None`. Uses `git_at` so ambient `GIT_DIR`/etc. (set when run
/// inside a hook) cannot redirect the query at another repo.
fn merge_driver_value(repo_dir: &std::path::Path) -> Option<String> {
    let out = git_at(repo_dir)
        .args(["config", "--get", "merge.coverage-keepours.driver"])
        .output()
        .ok()?;
    let value = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Ensure the keep-ours merge driver is registered in `repo_dir`; register it when
/// unset/wrong. Returns `true` when it changed config. Mirrors `git::ensure_hooks_path`.
fn ensure_merge_driver(repo_dir: &std::path::Path) -> anyhow::Result<bool> {
    if needs_merge_driver(merge_driver_value(repo_dir).as_deref()) {
        register_keepours(repo_dir)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Self-healing merge-driver registration: register the keep-ours driver for the
/// generated coverage artifacts if it is not already, so fresh clones wire up on first
/// run. Git config is shared per-clone, so this also covers every worktree. Best-effort —
/// a failure here must never block the actual command. Parallels [`ensure_hooks_installed`].
pub fn ensure_merge_driver_installed() {
    match ensure_merge_driver(std::path::Path::new(".")) {
        Ok(true) => eprintln!("xtask: registered merge.coverage-keepours (keep-ours)"),
        Ok(false) => {}
        Err(e) => {
            eprintln!("xtask: warning: could not register merge.coverage-keepours: {e:#}")
        }
    }
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cd /home/mdorman/src/jaunder/.claude/worktrees/issue-103-merge-driver-autoregister && cargo nextest run -p xtask merge_driver_tests`
Expected: PASS — all five tests in the module (`git_at_scrubs_repo_redirecting_env`, `keepours_driver_resolves_merge_to_ours_without_markers`, `needs_merge_driver_when_unset_or_wrong`, `no_need_when_merge_driver_already_true`, `ensure_merge_driver_registers_then_is_idempotent`).

- [x] **Step 5: Wire the self-heal into `main.rs`**

In `xtask/src/main.rs`, the body currently calls `xtask::ensure_hooks_installed();` then `run(cli)`. Add the merge-driver self-heal right after the hooks one. The result should read:

```rust
    // Self-healing: wire core.hooksPath -> .githooks on every run (best-effort).
    xtask::ensure_hooks_installed();
    // Self-healing: register the keep-ours coverage merge driver on every run (best-effort).
    xtask::ensure_merge_driver_installed();
```

- [x] **Step 6: Per-task gate (fmt + clippy)**

Run: `cd /home/mdorman/src/jaunder/.claude/worktrees/issue-103-merge-driver-autoregister && cargo xtask check --no-test`
Expected: exit 0 (`isError` false). No clippy warnings; nothing reformatted.

- [x] **Step 7: Commit**

```bash
git add xtask/src/lib.rs xtask/src/main.rs docs/superpowers/specs/2026-06-28-issue-103-merge-driver-autoregister-design.md docs/superpowers/plans/2026-06-28-issue-103-merge-driver-autoregister.md
git commit -m "feat(xtask): self-heal the coverage keep-ours merge driver (#103)

Register merge.coverage-keepours on every cargo xtask run, mirroring the
core.hooksPath self-heal, so a fresh clone wires the driver on first gate
run (config is shared per-clone, so all worktrees inherit it). Includes the
#103 spec and plan."
```

(The pre-commit hook runs the full `cargo xtask check`; if it reformats or heals anything it will fail-and-restage — re-`git add` and re-commit.)

---

### Task 2: Remove the redundant `install-merge-driver` subcommand

**Files:**
- Modify: `xtask/src/lib.rs` (delete the `InstallMergeDriver` enum variant, its `command_name` arm, its `run` dispatch arm, and the `install_merge_driver()` fn)

**Interfaces:**
- Consumes: nothing new.
- Produces: no public API; `register_keepours` keeps exactly one caller (`ensure_merge_driver` from Task 1), so no dead-code warning.

- [x] **Step 1: Delete the enum variant**

In `xtask/src/lib.rs`, remove the doc comment + variant (currently ~lines 72-78):

```rust
    /// Register the keep-ours git merge driver for the generated coverage
    /// artifacts. `.gitattributes` maps `coverage-baseline.json` and
    /// `crap-manifest.json` to `merge=coverage-keepours`; git config is not
    /// version-controlled, so this one-shot wires the driver into the local
    /// clone (run once per clone/worktree).
    #[command(name = "install-merge-driver")]
    InstallMergeDriver,
```

- [x] **Step 2: Delete the `command_name` match arm**

In `impl Cli::command_name`, remove the line (currently ~88):

```rust
            Command::InstallMergeDriver => "install-merge-driver",
```

- [x] **Step 3: Delete the `run` dispatch arm**

In `pub fn run`, remove the arm (currently ~140-146):

```rust
        Command::InstallMergeDriver => {
            let start = std::time::Instant::now();
            let mut result = CommandResult::new("install-merge-driver");
            result.push(install_merge_driver());
            finalize(&mut result, start);
            Ok(result)
        }
```

- [x] **Step 4: Delete the `install_merge_driver()` fn**

Remove (currently ~257-263):

```rust
fn install_merge_driver() -> StepResult {
    match register_keepours(std::path::Path::new(".")) {
        Ok(()) => StepResult::ok("install-merge-driver")
            .detail("registered merge.coverage-keepours (keep-ours)"),
        Err(e) => StepResult::fail("install-merge-driver").detail(e.to_string()),
    }
}
```

- [x] **Step 5: Verify the build and tests are clean**

Run: `cd /home/mdorman/src/jaunder/.claude/worktrees/issue-103-merge-driver-autoregister && cargo xtask check --no-test`
Expected: exit 0. The `command_name` and `run` matches over `Command` are exhaustive (no wildcard), so the compiler confirms every reference to `InstallMergeDriver` is gone; no `unused function` warning for `register_keepours` (Task 1's `ensure_merge_driver` still calls it).

Run: `cargo nextest run -p xtask`
Expected: PASS — including the `cli_tests` and `merge_driver_tests` modules; no test referenced the removed command.

- [x] **Step 6: Commit**

```bash
git add xtask/src/lib.rs
git commit -m "refactor(xtask): drop the manual install-merge-driver command (#103)

The keep-ours driver now self-heals on every cargo xtask run (Task 1), so
the one-shot subcommand is redundant. register_keepours stays as the helper
the self-heal calls."
```

---

### Task 3: Docs — `.gitattributes`, CONTRIBUTING, ADR-0029 supplement

**Files:**
- Modify: `.gitattributes` (header comment, lines 1-4)
- Modify: `CONTRIBUTING.md:192` (driver-registration sentence)
- Modify: `docs/adr/0029-git-enforced-verify-gate.md` (append a supplement section)

**Interfaces:** none (docs only).

- [ ] **Step 1: Update the `.gitattributes` header comment**

Replace lines 1-4 of `.gitattributes`:

```
# Generated coverage artifacts. Keep-ours on merge: a merge resolves to our side
# with no conflict markers, and the coverage gate's Fix-mode heal restores the
# authoritative content on the next `cargo xtask check`. Register the driver once
# per clone/worktree with: cargo xtask install-merge-driver
```

with:

```
# Generated coverage artifacts. Keep-ours on merge: a merge resolves to our side
# with no conflict markers, and the coverage gate's Fix-mode heal restores the
# authoritative content on the next `cargo xtask check`. The driver auto-registers
# on any `cargo xtask` run (self-healing, like core.hooksPath), so a fresh clone
# wires itself up on first gate run — no manual step.
```

(Leave the two `merge=coverage-keepours` lines unchanged.)

- [ ] **Step 2: Update `CONTRIBUTING.md:192`**

In the sentence at line 192, replace:

```
Register the driver once per clone/worktree with `cargo xtask install-merge-driver`.
```

with:

```
The driver auto-registers on any `cargo xtask` run (self-healing, like `core.hooksPath`), so a fresh clone wires itself up on first gate run.
```

(Leave the rest of the sentence/paragraph unchanged.)

- [ ] **Step 3: Append the ADR-0029 supplement**

At the end of `docs/adr/0029-git-enforced-verify-gate.md`, append:

```markdown

## Supplement (#103): merge-driver self-heal

The keep-ours merge driver for the generated coverage artifacts
(`coverage-baseline.json`, `crap-manifest.json`; `.gitattributes` →
`merge=coverage-keepours`) now self-heals on the same path as `core.hooksPath`: every
`cargo xtask` run calls `ensure_merge_driver_installed()`, which idempotently registers
`merge.coverage-keepours.driver=true` in the clone's local git config when unset/wrong.
This closes the last gap where local git config — not version-controlled — depended on
an operator remembering a manual one-shot: a fresh clone now wires the driver on first
gate run, and because the config is shared per-clone it covers all worktrees. The manual
`cargo xtask install-merge-driver` subcommand is removed as redundant; the reusable
`register_keepours()` helper remains and is the call the self-heal makes.

No `post-merge` re-heal hook is added (deliberately). Re-healing the baseline/manifest to
the merged tree requires a full Nix-instrumented `cargo xtask check` — there is no cheap
re-heal — and a `post-merge` hook fires on every `git merge`/`git pull`, including merges
that touch nothing coverage-related, so eager re-heal would mean a heavy coverage run after
every pull. Keep-ours already leaves a valid our-side baseline that the next pre-commit
`cargo xtask check` re-heals lazily; lazy re-heal is sufficient and the eager cost is not
justified.
```

- [ ] **Step 4: Sanity-check the docs**

Run: `cd /home/mdorman/src/jaunder/.claude/worktrees/issue-103-merge-driver-autoregister && git grep -n "install-merge-driver" -- ':!docs/archive' ':!docs/superpowers'`
Expected: no matches outside the archived #86 docs and this cycle's spec/plan (which intentionally reference the removed command in historical/decision context).

- [ ] **Step 5: Commit**

```bash
git add .gitattributes CONTRIBUTING.md docs/adr/0029-git-enforced-verify-gate.md
git commit -m "docs(#103): merge driver auto-registers; ADR-0029 supplement

.gitattributes and CONTRIBUTING no longer point at the removed one-shot;
ADR-0029 records the self-heal and the deliberate no-post-merge-hook call."
```

---

## Final gate (before ship)

- [ ] Run the full CI-faithful gate from the worktree: `cargo xtask validate` (or `validate --no-e2e` per the autonomous-gate policy). Expected: exit 0, `xtask-done: ... ok=true`.
- [ ] Review the branch diff against the fork point: `git diff wt-base-issue-103..HEAD`.

## Self-review (plan vs. spec)

- **Spec coverage:** self-heal logic → Task 1; wire-up → Task 1 Step 5; remove manual command → Task 2; `.gitattributes` + CONTRIBUTING + ADR-0029 supplement → Task 3; no post-merge hook → captured in the ADR supplement (Task 3), nothing to implement. Testing (`needs_merge_driver`, `ensure_merge_driver` idempotence, existing behavior test) → Task 1. All spec sections map to a task.
- **Placeholder scan:** none — every code/step block is concrete.
- **Type consistency:** `needs_merge_driver(Option<&str>) -> bool`, `merge_driver_value(&Path) -> Option<String>`, `ensure_merge_driver(&Path) -> anyhow::Result<bool>`, `ensure_merge_driver_installed()` are used identically in their definitions (Task 1), the tests (Task 1), the `main.rs` wire-up (Task 1 Step 5), and the ADR text (Task 3). `register_keepours(&Path) -> anyhow::Result<()>` matches its existing signature.
