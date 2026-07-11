# xtask git consolidation (#146) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-11-issue-146-xtask-git-consolidation.md`](../specs/2026-07-11-issue-146-xtask-git-consolidation.md)
— the "what/why." This plan is the "how"; it does not restate the spec.

**Goal:** Route every git invocation in `xtask/src` through one `git::at`-based
module, deleting the per-site wrappers and dropping `xshell` from `git.rs`.

**Architecture:** Grow `xtask/src/git.rs` with primitives
(`output`/`lines`/`run`) and typed helpers built on the existing env-scrubbed
`git::at` constructor, all `dir: &Path`-first. Migrate `adr.rs`, `coverage/`,
the `git.rs` cwd helpers, the two `steps/*` toplevel lookups, and `lib.rs`'s
call sites onto them. Behavior is preserved; the pre-existing adr
renumber/promote integration tests are the behavior-neutrality proof.

**Tech Stack:** Rust, `xtask` crate (excluded from the workspace — own
manifest), `anyhow`, shell-out to the `git` binary. No git library (see spec /
issue).

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **xtask is not in the workspace** — run its tests with
  `cargo nextest run --manifest-path xtask/Cargo.toml <filter>` (not
  `-p xtask`), and its `.rs` is **not** coverage-measured, but clippy
  `dead_code`/`unused` still apply: **every new `pub(crate)` helper must gain a
  real caller in the same commit** (the xtask pub-API dead-code commit
  boundary), so each task pairs a helper with its adoption.
- **Gate before every commit:** `cargo xtask check` (fmt + clippy + Nix
  coverage/tests) must pass clean — see **jaunder-commit**. Run it via
  `devtool run -- cargo xtask check`.
- New helpers are `pub(crate)` (xtask-internal); `git::at`, `HOOKS_PATH`, and
  the cwd helpers stay `pub` (crate root re-exports / cross-module use).
- All new helpers take `dir: &Path` and build on `git::at(dir)` — never a raw
  `Command::new("git")` and never `xshell` for git.

## Review header

**Scope (in):** `xtask/src/git.rs` (new helpers + cwd-helper rewrite),
`xtask/src/adr.rs`, `xtask/src/coverage/mod.rs`, `xtask/src/coverage/probe.rs`,
`xtask/src/steps/build_csr.rs`, `xtask/src/steps/e2e_local.rs`,
`xtask/src/lib.rs` (2 call sites).

**Scope (out):** non-git `xshell` usage (the `cargo` calls in `steps/*`,
`sh.rs`); `coverage/probe.rs` _behavior_ (refactored, not changed); the two
fire-and-forget `git::at(...).status()` cleanups in `probe.rs` (stay direct —
they ignore failure). No separable concerns surfaced — nothing to file as a
first task.

**Tasks:**

1. Add git plumbing primitives + typed helpers to `git.rs`; migrate `adr.rs`
   onto them (delete `git_out`/`git_lines`).
2. Add `git::toplevel`; migrate `coverage/mod.rs::git_repo_root` and the two
   `steps/*` `git rev-parse` lookups onto it.
3. Reduce `coverage/probe.rs::git_run` to a `git::run` call with a local
   hook-disable prefix.
4. Add `config_get`/`config_set`; rewrite the `git.rs` cwd helpers
   (`working_tree_status`/`hooks_path`/`ensure_hooks_path`) onto `git::at`;
   update `lib.rs`'s 2 call sites; drop `xshell` from `git.rs`.

**Key risks/decisions:**

- **`config_get` diverges from `hooks_path`'s old `.read().ok()`** (bails on
  exit 128 instead of swallowing to `None`). Intentional, fail-fast; only the
  corrupt- config path changes (spec AC#7). Pin unset→`None` and set→`Some` with
  tests.
- **Dead-code boundary:** `run` lands in Task 1 (consumed by `mv`/`add`), reused
  in Task 3 — never introduced without a caller.
- **`toplevel` adds env-scrubbing** the raw `git_repo_root` lacked — behavior-
  neutral for `cargo xtask`, more correct under the hook env (spec).
- Task 4 changes signatures (`Shell` → `&Path`) and is landed last.

---

### Task 1: git plumbing primitives + typed helpers; migrate `adr.rs`

**Files:**

- Modify: `xtask/src/git.rs` (add helpers + unit tests)
- Modify: `xtask/src/adr.rs` (delete `git_out`/`git_lines`, lines 76–100; rewire
  `run_renumber`/`run_promote`, lines 129–326)

**Interfaces:**

- Consumes: `git::at(dir: &Path) -> Command` (exists, git.rs:18).
- Produces (all `pub(crate)`, in `git.rs`):
  - `fn output(dir: &Path, args: &[&str]) -> Result<String>`
  - `fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>>`
  - `fn run(dir: &Path, args: &[&str]) -> Result<()>`
  - `fn merge_base(dir: &Path, a: &str, b: &str) -> Result<String>`
  - `fn diff_names(dir: &Path, range: &str) -> Result<Vec<String>>`
  - `fn diff_added(dir: &Path, range: &str, pathspec: &str) -> Result<Vec<String>>`
  - `fn grep_files(dir: &Path, pattern: &str) -> Result<Vec<String>>`
  - `fn mv(dir: &Path, from: &str, to: &str) -> Result<()>`
  - `fn add(dir: &Path, path: &str) -> Result<()>`

- [x] **Step 1: Add the helpers to `git.rs`** (above the `#[cfg(test)]` module).
      Add `use std::path::Path;` if not already imported (it is — git.rs:4).

```rust
/// Trimmed stdout of a git command in `dir`; bail on any non-zero exit.
pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<String> {
    let out = at(dir)
        .args(args)
        .output()
        .with_context(|| format!("running git {args:?}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Non-empty lines of [`output`].
pub(crate) fn lines(dir: &Path, args: &[&str]) -> Result<Vec<String>> {
    Ok(output(dir, args)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// Run a git command in `dir` for effect (no capture); bail on non-zero exit.
pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<()> {
    let ok = at(dir)
        .args(args)
        .status()
        .with_context(|| format!("running git {args:?}"))?
        .success();
    if !ok {
        anyhow::bail!("git {args:?} failed");
    }
    Ok(())
}

/// `git merge-base <a> <b>`.
pub(crate) fn merge_base(dir: &Path, a: &str, b: &str) -> Result<String> {
    output(dir, &["merge-base", a, b])
}

/// `git diff --name-only <range>` — every file touched in the range.
pub(crate) fn diff_names(dir: &Path, range: &str) -> Result<Vec<String>> {
    lines(dir, &["diff", "--name-only", range])
}

/// `git diff --diff-filter=A --name-only <range> -- <pathspec>` — files ADDED in
/// the range, scoped to `pathspec`.
pub(crate) fn diff_added(dir: &Path, range: &str, pathspec: &str) -> Result<Vec<String>> {
    lines(
        dir,
        &["diff", "--diff-filter=A", "--name-only", range, "--", pathspec],
    )
}

/// `git grep -l --fixed-strings <pattern>` — files containing `pattern`.
/// Encapsulates grep's exit-code contract: exit 1 = no match (`Ok(vec![])`),
/// exit 128 (or any other non-zero) = real error (`Err`). This is the only
/// tolerate-non-zero helper, so it wraps `at` directly rather than [`output`].
pub(crate) fn grep_files(dir: &Path, pattern: &str) -> Result<Vec<String>> {
    let out = at(dir)
        .args(["grep", "-l", "--fixed-strings", pattern])
        .output()
        .with_context(|| format!("running git grep -l {pattern:?}"))?;
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(str::to_string)
            .collect()),
        Some(1) => Ok(Vec::new()),
        _ => anyhow::bail!(
            "git grep -l {pattern:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// `git mv <from> <to>`.
pub(crate) fn mv(dir: &Path, from: &str, to: &str) -> Result<()> {
    run(dir, &["mv", from, to])
}

/// `git add <path>`.
pub(crate) fn add(dir: &Path, path: &str) -> Result<()> {
    run(dir, &["add", path])
}
```

- [x] **Step 2: Add unit tests to `git.rs`'s `#[cfg(test)] mod tests`** (pins
      every branch the adr integration tests don't reach — notably grep_files's
      exit-128 path). Add a temp-repo helper alongside the existing
      pure-predicate tests:

```rust
fn temp_repo(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jaunder-git-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.email", "t@t"],
        &["config", "user.name", "t"],
    ] {
        assert!(at(&dir).args(args).status().unwrap().success());
    }
    dir
}

fn commit(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
    assert!(at(dir).args(["add", rel]).status().unwrap().success());
    assert!(at(dir).args(["commit", "-qm", "c"]).status().unwrap().success());
}

#[test]
fn output_returns_trimmed_stdout_and_bails_on_error() {
    let dir = temp_repo("output");
    commit(&dir, "a.txt", "x\n");
    let head = output(&dir, &["rev-parse", "HEAD"]).unwrap();
    assert_eq!(head.len(), 40, "full sha, trimmed: {head:?}");
    assert!(output(&dir, &["not-a-subcommand"]).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lines_drops_blank_lines() {
    let dir = temp_repo("lines");
    commit(&dir, "a.txt", "1\n");
    commit(&dir, "b.txt", "2\n");
    let subjects = lines(&dir, &["log", "--format=%s"]).unwrap();
    assert_eq!(subjects, vec!["c".to_string(), "c".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_ok_on_success_err_on_failure() {
    let dir = temp_repo("run");
    commit(&dir, "a.txt", "x\n");
    assert!(run(&dir, &["status", "--porcelain"]).is_ok());
    assert!(run(&dir, &["mv", "nope", "nowhere"]).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn grep_files_match_no_match_and_error() {
    let dir = temp_repo("grep");
    commit(&dir, "hay.txt", "a needle here\n");
    commit(&dir, "other.txt", "nothing\n");
    assert_eq!(grep_files(&dir, "needle").unwrap(), vec!["hay.txt".to_string()]);
    assert!(grep_files(&dir, "absent-token").unwrap().is_empty()); // exit 1
    // Outside a repo, `git grep` exits 128 → Err (NOT an empty match).
    let non_repo = std::env::temp_dir().join(format!("jaunder-git-nonrepo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&non_repo);
    std::fs::create_dir_all(&non_repo).unwrap();
    assert!(grep_files(&non_repo, "x").is_err());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&non_repo);
}

#[test]
fn merge_base_diff_added_and_diff_names() {
    let dir = temp_repo("diff");
    commit(&dir, "base.txt", "b\n");
    let base = output(&dir, &["rev-parse", "HEAD"]).unwrap();
    assert!(at(&dir).args(["checkout", "-q", "-b", "feature"]).status().unwrap().success());
    commit(&dir, "docs/new.md", "n\n");
    let range = format!("{base}..HEAD");
    assert_eq!(merge_base(&dir, "main", "HEAD").unwrap(), base);
    assert_eq!(diff_names(&dir, &range).unwrap(), vec!["docs/new.md".to_string()]);
    assert_eq!(diff_added(&dir, &range, "docs").unwrap(), vec!["docs/new.md".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [x] **Step 3: Run the git.rs tests, verify they fail** (helpers don't compile
      yet if Step 1 skipped, else fail to link):

Run: `cargo nextest run --manifest-path xtask/Cargo.toml git::tests` Expected:
FAIL before Step 1's helpers exist; PASS once they do — run after Step 1 to
confirm green, then proceed.

- [x] **Step 4: Migrate `adr.rs` onto the helpers.** Delete `git_out` (76–91)
      and `git_lines` (94–100). Rewrite each call site (the mapping is 1:1, no
      logic change — the helpers absorb the exit-code handling `git_out` did):
  - `git_out(repo, &["merge-base", main_ref, "HEAD"], false)` (130) →
    `git::merge_base(repo, main_ref, "HEAD")`.
  - `git_lines(repo, &["diff","--diff-filter=A","--name-only",&range,"--",ADR_DIR], false)`
    (135) → `git::diff_added(repo, &range, ADR_DIR)`.
  - `git_lines(repo, &["diff","--name-only",&range], false)` (152) →
    `git::diff_names(repo, &range)`.
  - `git_out(repo, &["mv", &old_rel, &new_rel], false)` (179) →
    `git::mv(repo, &old_rel, &new_rel)`.
  - `git_lines(repo, &["grep","-l","--fixed-strings",&old_stem], true)` (182) →
    `git::grep_files(repo, &old_stem)`.
  - `git_lines(repo, &["grep","-l","--fixed-strings",&bare_token], true)` (189)
    → `git::grep_files(repo, &bare_token)`.
  - `git_out(repo, &["add", &new_rel], false)` (297) →
    `git::add(repo, &new_rel)`.
  - `git_lines(repo, &["grep","-l","--fixed-strings",&draft_stem], true)` (308)
    → `git::grep_files(repo, &draft_stem)`.
  - `git_out(repo, &["add", &file], false)` (310) → `git::add(repo, &file)`.
  - `git_out(repo, &["add", crate::adr_readme::README], false)` (319) →
    `git::add(repo, crate::adr_readme::README)`.

  The adr test helpers (`git`/`git_stdout` using `crate::git::at`, lines
  365–382) stay unchanged. Leave adr's basename post-processing
  (`.filter_map(|p| p.rsplit('/')…)`, 147–149) as-is — it consumes
  `diff_added`'s output.

- [x] **Step 5: Verify behavior is preserved.** The adr integration tests
      exercise every migrated path.

Run: `cargo nextest run --manifest-path xtask/Cargo.toml adr::` Expected: PASS —
`renumber_*` (3), `promote_*` (7), `pad_*`/`replace_*`/`rewrite_*` all green,
unmodified.

- [x] **Step 6: Gate + commit.**

Run: `devtool run -- cargo xtask check` — expect green (also confirms no
`dead_code`: every new helper now has a caller).

```bash
git add xtask/src/git.rs xtask/src/adr.rs
git commit -m "refactor(xtask): add shared git plumbing helpers; migrate adr onto them"
```

---

### Task 2: `git::toplevel`; migrate `coverage/mod.rs` + `steps/*`

**Files:**

- Modify: `xtask/src/git.rs` (add `toplevel` + unit test)
- Modify: `xtask/src/coverage/mod.rs` (delete `git_repo_root` 230–236; caller
  90; drop `use std::process::Command;` 13)
- Modify: `xtask/src/steps/build_csr.rs:18`
- Modify: `xtask/src/steps/e2e_local.rs:63`

**Interfaces:**

- Consumes: `git::output` (Task 1).
- Produces: `pub(crate) fn toplevel(dir: &Path) -> Result<String>` in `git.rs`.

- [x] **Step 1: Add `toplevel` to `git.rs`.**

```rust
/// `git rev-parse --show-toplevel` — the working tree's root.
pub(crate) fn toplevel(dir: &Path) -> Result<String> {
    output(dir, &["rev-parse", "--show-toplevel"])
}
```

- [x] **Step 2: Add a `git.rs` unit test.**

```rust
#[test]
fn toplevel_returns_repo_root() {
    let dir = temp_repo("toplevel");
    commit(&dir, "a.txt", "x\n");
    let root = toplevel(&dir).unwrap();
    // Compare canonically — /tmp may be a symlink.
    assert_eq!(
        std::fs::canonicalize(&root).unwrap(),
        std::fs::canonicalize(&dir).unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [x] **Step 3: Migrate `coverage/mod.rs`.** Delete `git_repo_root` (230–236).
      Replace its caller at line 90 — `let repo_root = git_repo_root()?;` →
      `let repo_root =     crate::git::toplevel(std::path::Path::new("."))?;`.
      Remove the now-unused `use std::process::Command;` (line 13).
      (`anyhow::{Context, Result}` stays — used elsewhere.)

- [x] **Step 4: Migrate the two `steps/*` toplevel lookups.** In each of
      `steps/build_csr.rs:18` and `steps/e2e_local.rs:63`, replace

```rust
    let Ok(root) = cmd!(sh, "git rev-parse --show-toplevel").quiet().read() else {
        result.push(StepResult::fail(<label>).detail("cannot locate repo root".to_owned()));
        return;
    };
    let root = root.trim().to_owned();
```

with (`toplevel` already trims → drop the `.trim()` line):

```rust
    let Ok(root) = crate::git::toplevel(std::path::Path::new(".")) else {
        result.push(StepResult::fail(<label>).detail("cannot locate repo root".to_owned()));
        return;
    };
```

Keep `<label>` as-is (`"build-csr"` / `"e2e-local"`), keep the following
`sh.change_dir(&root)` in `build_csr`, and keep `use xshell::{cmd, Shell}` in
both (still used for the `cargo` invocations).

- [x] **Step 5: Verify.**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml git::tests::toplevel`
Expected: PASS. Run: `devtool run -- cargo xtask check` Expected: green —
compiles (no unused `Command` import), clippy clean.

- [x] **Step 6: Commit.**

```bash
git add xtask/src/git.rs xtask/src/coverage/mod.rs xtask/src/steps/build_csr.rs xtask/src/steps/e2e_local.rs
git commit -m "refactor(xtask): route toplevel lookups through git::toplevel"
```

---

### Task 3: reduce `coverage/probe.rs::git_run` to `git::run`

**Files:**

- Modify: `xtask/src/coverage/probe.rs` (`git_run` 97–111; add a smoke test)

**Interfaces:**

- Consumes: `git::run` (Task 1). No new public surface.

- [x] **Step 1: Rewrite `git_run` to delegate.** Keep the probe-specific
      `-c core.hooksPath=` hook-disable local; drop the reinvented bail logic:

```rust
/// Run a git subcommand in `dir` with hooks disabled; bail on a non-zero exit.
/// Hooks are disabled defensively — `worktree add` can fire a `post-checkout`
/// hook, and we never want the repo's gate hooks running inside the probe.
fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}
```

The two fire-and-forget cleanups (`WorktreeGuard::drop` 88–94, the pre-run
leftover-remove 136–141) stay direct `git::at(...)` calls — they intentionally
ignore failure, so the bail-on-error `git::run` is the wrong shape.

- [x] **Step 2: Add a smoke test** to `probe.rs`'s `#[cfg(test)] mod tests`
      (proves the delegating wrapper still runs a subcommand and reports
      failure):

```rust
#[test]
fn git_run_succeeds_and_fails() {
    let dir = std::env::temp_dir().join(format!("jaunder-probe-gitrun-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(git::at(&dir).args(["init", "-q"]).status().unwrap().success());
    assert!(git_run(&dir, &["status", "--porcelain"]).is_ok());
    assert!(git_run(&dir, &["mv", "nope", "nowhere"]).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
```

Ensure `use crate::git;` (or the existing `git::at` import path) is in scope in
the test module; `probe.rs` already references `git::at`.

- [x] **Step 3: Verify.**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml probe::tests` Expected:
PASS (the pure `probe_verdict` tests plus `git_run_succeeds_and_fails`). Run:
`devtool run -- cargo xtask check` Expected: green.

- [x] **Step 4: Commit.**

```bash
git add xtask/src/coverage/probe.rs
git commit -m "refactor(xtask): reduce coverage probe git_run to git::run"
```

---

### Task 4: `config_get`/`config_set`; rewrite cwd helpers; drop `xshell` from `git.rs`

**Files:**

- Modify: `xtask/src/git.rs` (add `config_get`/`config_set` + tests; rewrite
  `working_tree_status`/`hooks_path`/`ensure_hooks_path` 52–83; remove
  `use xshell::{cmd, Shell};` line 8)
- Modify: `xtask/src/lib.rs` (`ensure_hooks_installed` 448–457;
  `clean_tree_precheck` signature 472 + caller 304; add `use std::path::Path;`)

**Interfaces:**

- Consumes: `git::output`, `git::at` (Task 1), `needs_hooks_path`, `HOOKS_PATH`
  (exist).
- Produces:
  - `pub(crate) fn config_get(dir: &Path, key: &str) -> Result<Option<String>>`
  - `pub(crate) fn config_set(dir: &Path, key: &str, value: &str) -> Result<()>`
  - **Changed signatures** (breaking within-crate):
    `pub fn working_tree_status(dir: &Path) -> Result<String>`;
    `pub fn hooks_path(dir: &Path) -> Result<Option<String>>` (was
    `Option<String>`); `pub fn ensure_hooks_path(dir: &Path) -> Result<bool>`.

- [x] **Step 1: Add `config_get`/`config_set` to `git.rs`.**

```rust
/// `git config --get <key>` → the value, or `None` when unset (exit 1) or blank.
/// Bails on any other non-zero (e.g. exit 128 = corrupt config): a broken config
/// surfaces as an error rather than being silently treated as "unset".
pub(crate) fn config_get(dir: &Path, key: &str) -> Result<Option<String>> {
    let out = at(dir)
        .args(["config", "--get", key])
        .output()
        .with_context(|| format!("running git config --get {key}"))?;
    match out.status.code() {
        Some(0) => {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!v.is_empty()).then_some(v))
        }
        Some(1) => Ok(None),
        _ => anyhow::bail!(
            "git config --get {key} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// `git config <key> <value>`.
pub(crate) fn config_set(dir: &Path, key: &str, value: &str) -> Result<()> {
    run(dir, &["config", key, value])
}
```

- [x] **Step 2: Rewrite the cwd helpers in `git.rs`** (replace 52–83), and
      delete `use xshell::{cmd, Shell};` (line 8). The pure `porcelain_is_dirty`
      / `needs_hooks_path` / `HOOKS_PATH` stay.

```rust
/// `git status --porcelain` text. Errors only if git itself cannot run.
pub fn working_tree_status(dir: &Path) -> Result<String> {
    output(dir, &["status", "--porcelain"])
}

/// Current `core.hooksPath`, or `None` when unset/blank.
pub fn hooks_path(dir: &Path) -> Result<Option<String>> {
    config_get(dir, "core.hooksPath")
}

/// Ensure `core.hooksPath` points at [`HOOKS_PATH`]; set it if unset/wrong.
/// Returns `true` when it changed the config.
pub fn ensure_hooks_path(dir: &Path) -> Result<bool> {
    if needs_hooks_path(hooks_path(dir)?.as_deref()) {
        config_set(dir, "core.hooksPath", HOOKS_PATH)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
```

- [x] **Step 3: Add `git.rs` unit tests** for the config helpers (pins the
      unset→`None` and set→`Some` paths AC#7 names):

```rust
#[test]
fn config_get_none_when_unset_some_after_set() {
    let dir = temp_repo("config");
    assert_eq!(config_get(&dir, "core.hooksPath").unwrap(), None);
    config_set(&dir, "core.hooksPath", ".githooks").unwrap();
    assert_eq!(config_get(&dir, "core.hooksPath").unwrap(), Some(".githooks".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ensure_hooks_path_sets_then_is_noop() {
    let dir = temp_repo("ensure-hooks");
    assert!(ensure_hooks_path(&dir).unwrap(), "first call sets it");
    assert!(!ensure_hooks_path(&dir).unwrap(), "second call is a no-op");
    assert_eq!(hooks_path(&dir).unwrap(), Some(HOOKS_PATH.to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [x] **Step 4: Update `lib.rs` call sites.** Add `use std::path::Path;` near
      the top imports.
  - `ensure_hooks_installed` (448–457): drop the `xshell::Shell::new()` guard;
    call with a path:

```rust
pub fn ensure_hooks_installed() {
    match git::ensure_hooks_path(Path::new(".")) {
        Ok(true) => eprintln!("xtask: set core.hooksPath = {}", git::HOOKS_PATH),
        Ok(false) => {}
        Err(e) => eprintln!("xtask: warning: could not set core.hooksPath: {e:#}"),
    }
}
```

- `clean_tree_precheck` (472): drop the now-unused `sh: &xshell::Shell` param
  and call `git::working_tree_status(Path::new("."))`; update the caller at line
  304 from `clean_tree_precheck(&sh, allow_dirty)` to
  `clean_tree_precheck(allow_dirty)`. (`sh` remains used elsewhere in that
  scope, so it is not otherwise removed.)

```rust
fn clean_tree_precheck(allow_dirty: bool) -> StepResult {
    if allow_dirty {
        return StepResult::skip("clean-tree").detail("--allow-dirty");
    }
    match git::working_tree_status(Path::new(".")) {
        Ok(status) if git::porcelain_is_dirty(&status) => StepResult::fail("clean-tree").detail(
            format!("working tree is dirty — commit/stash or pass --allow-dirty:\n{}", status.trim()),
        ),
        Ok(_) => StepResult::ok("clean-tree"),
        Err(e) => StepResult::fail("clean-tree").detail(format!("could not determine cleanliness: {e:#}")),
    }
}
```

- [x] **Step 5: Verify.**

Run: `cargo nextest run --manifest-path xtask/Cargo.toml git::tests` Expected:
PASS (all prior git tests + `config_*` + `ensure_hooks_path_*`). Run:
`rg 'xshell' xtask/src/git.rs` → **no match**; `rg 'cmd!\([^)]*"git ' xtask/src`
→ **no match**; `rg 'Command::new\("git"\)' xtask/src` → **one** hit (git.rs
`at`). Run: `devtool run -- cargo xtask check` Expected: green — this also
exercises the hooks-path self-healing on the real tree.

- [x] **Step 6: Commit.**

```bash
git add xtask/src/git.rs xtask/src/lib.rs
git commit -m "refactor(xtask): migrate hooks/status helpers onto git::at, drop xshell from git.rs"
```

---

## Self-review notes

- **Spec coverage:** AC#1 → Task 4 Step 5 greps; AC#2 → Task 1
  (git_out/git_lines deleted); AC#3 → `grep_files` + Task 1 Step 2 3-branch
  test; AC#4 → helper list across Tasks 1/2/4, each with a caller; AC#5 → Task 1
  Step 5 (adr tests unmodified); AC#6 → per-task git.rs unit tests; AC#7 → Task
  4 Steps 2–3 (behavior + `config_get` divergence pinned); AC#8 → each task's
  `cargo xtask check`.
- **No placeholders:** every step carries real Rust + exact commands.
- **Type consistency:** helper names/signatures in the Interfaces blocks match
  their call sites (`toplevel`, `merge_base`, `diff_added`, `diff_names`,
  `grep_files`, `mv`, `add`, `run`, `config_get`, `config_set`,
  `working_tree_status`, `hooks_path`, `ensure_hooks_path`).
