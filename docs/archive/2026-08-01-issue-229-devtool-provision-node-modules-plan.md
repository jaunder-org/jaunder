# `devtool provision-node-modules` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`2026-08-01-issue-229-devtool-provision-node-modules-spec.md`](2026-08-01-issue-229-devtool-provision-node-modules-spec.md)
— criteria are referenced below as A1–A20. Issue: #229.

**Goal:** Replace `end2end/provision-node-modules.sh` with a typed
`devtool provision-node-modules` subcommand, called in-process by
`devtool check tsc` and by name from the devShell `shellHook`.

**Architecture:** A new `tools/devtool/src/provision.rs` owns two functions — a
`resolve` fallback (flag → env var → error naming the variable and the devShell)
and a `run` that materializes `<root>/end2end/node_modules`. `main.rs` exposes
it as a subcommand; `check.rs` calls `run` directly rather than spawning `bash`;
`flake.nix`'s shared `shellHook` calls the subcommand, which forces `devtoolBin`
from `devOnly` into `ciInputs` (the hook is shared with `devShells.ci`). The
script is then deleted.

**Tech Stack:** Rust 2021, `clap` 4 (derive), `anyhow`, `tempfile`;
`std::os::unix::fs::symlink`. The `tools/` virtual workspace, gated by the
`tools-test` and `tools-clippy` xtask steps. Nix flake devShells + the
`static-checks` check derivation.

## Review header

**Scope — in:**
`tools/devtool/{src/provision.rs, src/main.rs, src/check.rs, tests/provision_cli.rs}`,
`flake.nix`, `docs/adr/0028-devtool-vs-xtask-boundary.md`, deletion of
`end2end/provision-node-modules.sh`.

**Scope — out:** linking dot-entries (`.bin`); pruning stale `node_modules`
entries; migrating any other shell script; changing `e2ePackage` /
`playwright-test` / the closure's contents; rewording ADR-0051. No separable
concerns were surfaced during the spec interview, so there is no issue-filing
task.

**Tasks:**

1. `provision.rs` (resolver + run) with tempdir tests, wired as a `devtool`
   subcommand.
2. `tests/provision_cli.rs` — the flag/env/cwd surface the in-process caller
   bypasses.
3. `check.rs` — `needs_provisioning` predicate + in-process call, no `bash`
   spawn.
4. `flake.nix` + ADR-0028 + delete the script; run the fresh-provision and Nix
   gates.

**Key risks / decisions:**

- **The `shellHook` is shared** by `devShells.ci` and `devShells.default`, but
  `devtoolBin` lives in `devOnly`. Task 4 moves it to `ciInputs`; skipping that
  gives `devtool: command not found` on every CI shell entry (A16).
- **Ordering is load-bearing:** the script must survive until _both_ callers
  stop using it. Task 3 (check.rs) precedes Task 4 (flake + deletion). Never
  delete earlier.
- **Deleting the script changes `e2ePackage`'s store path** (`flake.nix:488-497`
  copies all of `./end2end`), so e2e derivations rebuild and every existing
  `end2end/node_modules` goes stale on this commit. Expected, not a failure.
- **`@playwright` is written twice on purpose** — symlinked by the entry loop,
  then replaced by a real directory. That mirrors the bash and is what A7/A8
  pin.

---

## Global Constraints

Every task's requirements implicitly include these.

- **Dot-entries are skipped.** The bash glob `"$E2E_TYPES_NODE_MODULES"/*` never
  matched `.bin` or `.package-lock.json`; the port must not link them either
  (A6).
- **Unset-variable errors name the variable and the devShell** — reproducing
  `: "${E2E_TYPES_NODE_MODULES:?unset — run inside the Nix devShell}"` (A10,
  A11). There is exactly **one** message site: `provision::resolve`.
- **No `cargo` invocation may appear in the `shellHook`** (A16).
- **Commits:** the pre-commit hook runs the full `cargo xtask check`; run it
  first so it passes clean (**jaunder-commit**). Message form
  `type(scope): subject (#229)`. **No `Co-Authored-By` trailer.**
- **Gate invocation:** run everything through `devtool run --cwd` pinned to this
  worktree, then grep the parked log — never `cmd | rg`. Worktree root:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision`.
- **`tools/` is outside the coverage derivation** (`flake.nix:1128`) and
  `devtoolBin` builds `doCheck = false`; `tools-test` / `tools-clippy` are the
  only gates on this code. No coverage evidence or `cov:ignore` obligation
  attaches to it.
- `cargo xtask check` auto-formats Markdown via prettier — expect it to reflow
  the spec/plan and include those edits in the task's commit.

---

### Task 1: The `provision` module and its subcommand

**Files:**

- Create: `tools/devtool/src/provision.rs` (implementation + in-file
  `#[cfg(test)]`, the crate's convention — see `run.rs`, `pg.rs`)
- Modify: `tools/devtool/src/main.rs:8-13` (module list), `:22-43` (`Command`
  enum), `:45-94` (args structs), `:116-128` (dispatch)

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces:
  - `pub fn resolve(flag: Option<std::path::PathBuf>, var: &str) -> anyhow::Result<std::path::PathBuf>`
    — returns `flag` when `Some`, else `std::env::var(var)`, else an error
    naming `var`. Task 3 calls it with `None`.
  - `pub fn run(root: &std::path::Path, types_node_modules: &std::path::Path, playwright_test: &std::path::Path) -> anyhow::Result<()>`
    — provisions `<root>/end2end/node_modules`. Task 3 calls it with
    `Path::new(".")`.
  - CLI:
    `devtool provision-node-modules [--types-node-modules <PATH>] [--playwright-test <PATH>] [--root <DIR>]`.
    Task 4's `shellHook` invokes the zero-argument form.

- [x] **Step 1: Write the failing tests**

Add to `tools/devtool/src/provision.rs`. The helper builds a fake store tree;
every branch of `run` and `resolve` is pinned here.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fake `E2E_TYPES_NODE_MODULES` closure: the real entries tsc needs, plus
    /// the dot-entries a real npm tree carries and an `@playwright` that must lose
    /// to `--playwright-test`.
    fn fake_types_dir(base: &Path, marker: &str) -> PathBuf {
        let dir = base.join(marker);
        for name in [
            "@types",
            "typescript",
            "undici-types",
            "playwright",
            "playwright-core",
            "@playwright",
            ".bin",
        ] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }
        fs::write(dir.join(".package-lock.json"), "{}").unwrap();
        dir
    }

    fn fake_playwright(base: &Path) -> PathBuf {
        let dir = base.join("playwright-test");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("devtool-provision-")
            .tempdir()
            .unwrap()
    }

    #[test]
    fn provisions_visible_entries_as_symlinks() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &types, &pw).unwrap();

        let dest = t.path().join("end2end/node_modules");
        for name in [
            "@types",
            "typescript",
            "undici-types",
            "playwright",
            "playwright-core",
        ] {
            let link = dest.join(name);
            assert!(
                fs::symlink_metadata(&link).unwrap().is_symlink(),
                "{name} should be a symlink"
            );
            assert_eq!(fs::read_link(&link).unwrap(), types.join(name));
        }
    }

    #[test]
    fn skips_dot_entries() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &types, &pw).unwrap();

        let dest = t.path().join("end2end/node_modules");
        for name in [".bin", ".package-lock.json"] {
            assert!(
                fs::symlink_metadata(dest.join(name)).is_err(),
                "{name} must not be provisioned"
            );
        }
    }

    #[test]
    fn pins_playwright_test_over_e2e_package_copy() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &types, &pw).unwrap();

        let at_pw = t.path().join("end2end/node_modules/@playwright");
        assert!(
            fs::symlink_metadata(&at_pw).unwrap().is_dir(),
            "@playwright must be a real directory, not a symlink to the closure's copy"
        );
        assert_eq!(fs::read_link(at_pw.join("test")).unwrap(), pw);
        let entries: Vec<_> = fs::read_dir(&at_pw).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("test")]);
    }

    #[test]
    fn is_idempotent_across_reruns() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &types, &pw).unwrap();
        run(t.path(), &types, &pw).unwrap();

        let dest = t.path().join("end2end/node_modules");
        assert_eq!(fs::read_link(dest.join("typescript")).unwrap(), types.join("typescript"));
        assert!(fs::symlink_metadata(dest.join("@playwright")).unwrap().is_dir());
        assert_eq!(fs::read_link(dest.join("@playwright/test")).unwrap(), pw);
    }

    #[test]
    fn replaces_a_stale_playwright_symlink_with_the_dir() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        let dest = t.path().join("end2end/node_modules");
        fs::create_dir_all(&dest).unwrap();
        // A previous run (or a hand-rolled tree) left @playwright as a symlink.
        std::os::unix::fs::symlink(types.join("@playwright"), dest.join("@playwright")).unwrap();

        run(t.path(), &types, &pw).unwrap();

        assert!(fs::symlink_metadata(dest.join("@playwright")).unwrap().is_dir());
        assert_eq!(fs::read_link(dest.join("@playwright/test")).unwrap(), pw);
    }

    #[test]
    fn repoints_symlinks_when_the_store_path_changes() {
        let t = tmp();
        let old = fake_types_dir(t.path(), "store-a");
        let new = fake_types_dir(t.path(), "store-b");
        let pw = fake_playwright(t.path());
        run(t.path(), &old, &pw).unwrap();
        run(t.path(), &new, &pw).unwrap();

        let dest = t.path().join("end2end/node_modules");
        assert_eq!(fs::read_link(dest.join("typescript")).unwrap(), new.join("typescript"));
    }

    #[test]
    fn errors_when_types_dir_missing() {
        let t = tmp();
        let missing = t.path().join("no-such-store");
        let pw = fake_playwright(t.path());
        let err = run(t.path(), &missing, &pw).unwrap_err();
        assert!(
            format!("{err:#}").contains(&missing.display().to_string()),
            "error should name the missing path, got: {err:#}"
        );
    }

    #[test]
    fn resolve_prefers_the_flag_over_the_environment() {
        let flag = PathBuf::from("/from/flag");
        // Deliberately a variable that is set in every environment.
        assert_eq!(resolve(Some(flag.clone()), "PATH").unwrap(), flag);
    }

    #[test]
    fn resolve_errors_name_the_variable_and_the_devshell() {
        let err = resolve(None, "JAUNDER_DEFINITELY_UNSET_FOR_TESTS").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("JAUNDER_DEFINITELY_UNSET_FOR_TESTS"), "got: {msg}");
        assert!(msg.contains("nix develop"), "got: {msg}");
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml -p devtool provision
```

Add the `mod provision;` line to `main.rs` first (otherwise the new file is not
compiled at all and the run is a vacuous pass). With the module declared and
only the test block written, expect: FAIL —
`E0425: cannot find function `run` in this scope` and the same for `resolve`,
plus unresolved `Path` / `PathBuf`.

- [x] **Step 3: Implement against the tests**

Write `tools/devtool/src/provision.rs` opening with

```rust
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
```

(the test module reaches these through `use super::*;`), to these signatures:

```rust
pub fn resolve(flag: Option<PathBuf>, var: &str) -> Result<PathBuf>
pub fn run(root: &Path, types_node_modules: &Path, playwright_test: &Path) -> Result<()>
```

`resolve` reads the fallback with `std::env::var_os` — these are **paths**,
which need not be UTF-8 — and treats an empty value as unset, matching bash's
`${VAR:?}` (which errors on empty, where `env::var` would hand back `Ok("")` and
defer the failure to a confusing `read_dir` error).

The Step 1 tests pin every branch — visible-entry symlinking, dot-entry
skipping, the `@playwright` directory replacing whatever was there, idempotence,
repointing, the missing-closure error, and both `resolve` arms — so the tests
specify the body. Two things they cannot express, which the implementation must
get right:

- **`rm -rf` semantics before each link.** Use a private helper over
  `fs::symlink_metadata` (which does **not** follow links): `is_dir()` →
  `fs::remove_dir_all`, any other `Ok` → `fs::remove_file`,
  `ErrorKind::NotFound` → no-op. Following the link instead would delete inside
  the Nix store on a rerun.
- **Order matters:** loop over the closure's visible entries first (which links
  the closure's own `@playwright`), _then_ remove `@playwright`, recreate it as
  a real directory, and symlink `test` into it. That is the bash's sequence and
  what `replaces_a_stale_playwright_symlink_with_the_dir` pins.

Dot-detection: `entry.file_name().to_string_lossy().starts_with('.')`. Symlinks:
`std::os::unix::fs::symlink`. Give the `read_dir` call
`.with_context(|| format!("reading the type-dep closure {}", types_node_modules.display()))`
so `errors_when_types_dir_missing` passes.

Then wire the subcommand. In `tools/devtool/src/main.rs`, add `mod provision;`
to the module list (alphabetical: after `pg`, before `run`), a variant on
`Command`:

```rust
    /// Symlink the tsc type-dep closure + the nix-matched Playwright into
    /// `<root>/end2end/node_modules` (gitignored, so absent in fresh checkouts and
    /// worktrees). Replaces `end2end/provision-node-modules.sh` (#229).
    ProvisionNodeModules(ProvisionArgs),
```

its args struct:

```rust
#[derive(clap::Args)]
struct ProvisionArgs {
    /// The tsc type-dep closure to symlink. Defaults to $E2E_TYPES_NODE_MODULES,
    /// exported by the Nix devShell.
    #[arg(long)]
    types_node_modules: Option<std::path::PathBuf>,
    /// The nix-matched @playwright/test to pin. Defaults to $E2E_PLAYWRIGHT_TEST,
    /// exported by the Nix devShell.
    #[arg(long)]
    playwright_test: Option<std::path::PathBuf>,
    /// Repo or worktree root; provisions <root>/end2end/node_modules.
    #[arg(long, default_value = ".")]
    root: std::path::PathBuf,
}
```

and the dispatch arm:

```rust
        Command::ProvisionNodeModules(args) => {
            let types = provision::resolve(args.types_node_modules, "E2E_TYPES_NODE_MODULES")?;
            let playwright = provision::resolve(args.playwright_test, "E2E_PLAYWRIGHT_TEST")?;
            provision::run(&args.root, &types, &playwright)
        }
```

- [x] **Step 4: Run the tests, verify they pass**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml -p devtool provision
```

Expected: PASS — 9 tests.

- [x] **Step 5: Commit**

Run `cargo xtask check` first (**jaunder-commit**), then:

```bash
git add tools/devtool/src/provision.rs tools/devtool/src/main.rs \
        docs/superpowers/specs/2026-08-01-issue-229-devtool-provision-node-modules.md \
        docs/superpowers/plans/2026-08-01-issue-229-devtool-provision-node-modules.md
git commit -m "feat(devtool): add a provision-node-modules subcommand (#229)"
```

The two Markdown paths are here because this is the first gate run of the cycle,
so prettier reflows the spec and plan now; leaving them unstaged would leave the
tree dirty at the commit boundary. Check `git status` before committing in the
later tasks too, and stage any further reflow with that task's files.

---

### Task 2: CLI-surface tests

**Files:**

- Create: `tools/devtool/tests/provision_cli.rs`

**Interfaces:**

- Consumes: the `devtool provision-node-modules` CLI from Task 1; `tempfile`
  (already a `[dependencies]` entry in `tools/devtool/Cargo.toml`, so it is
  available to integration tests without a `[dev-dependencies]` change).
- Produces: nothing later tasks depend on.

Why a separate file: Task 3 makes `check tsc` call `provision::run`
**in-process**, so the clap parser, the env fallback and the `--root` default
are never exercised by the library tests or by the gate. This is the only
automated cover for A1–A4.

- [x] **Step 1: Write the failing tests**

```rust
//! CLI-surface tests for `devtool provision-node-modules` — the flag parsing, env
//! fallback and cwd handling that `devtool check tsc`'s in-process call bypasses
//! (#229, spec A1-A4).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_devtool");

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("devtool-provision-cli-")
        .tempdir()
        .unwrap()
}

fn fake_types_dir(base: &Path, marker: &str) -> PathBuf {
    let dir = base.join(marker);
    fs::create_dir_all(dir.join("typescript")).unwrap();
    dir
}

fn fake_playwright(base: &Path) -> PathBuf {
    let dir = base.join("playwright-test");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn help_lists_the_flags_and_their_env_fallback() {
    let out = Command::new(EXE)
        .args(["provision-node-modules", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    for needle in [
        "--types-node-modules",
        "--playwright-test",
        "--root",
        "E2E_TYPES_NODE_MODULES",
        "E2E_PLAYWRIGHT_TEST",
    ] {
        assert!(help.contains(needle), "help should mention {needle}:\n{help}");
    }
}

#[test]
fn defaults_from_the_environment_and_the_current_directory() {
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let st = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env("E2E_PLAYWRIGHT_TEST", &pw)
        .status()
        .unwrap();

    assert!(st.success());
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/typescript")).unwrap(),
        types.join("typescript")
    );
}

#[test]
fn root_flag_targets_another_tree_and_leaves_cwd_alone() {
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    let target = t.path().join("target");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&target).unwrap();

    let st = Command::new(EXE)
        .args(["provision-node-modules", "--root"])
        .arg(&target)
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env("E2E_PLAYWRIGHT_TEST", &pw)
        .status()
        .unwrap();

    assert!(st.success());
    assert!(target.join("end2end/node_modules/typescript").exists());
    assert!(
        !work.join("end2end").exists(),
        "--root must not provision the current directory"
    );
}

#[test]
fn explicit_flags_beat_the_environment() {
    let t = tmp();
    let decoy = fake_types_dir(t.path(), "decoy");
    let real = fake_types_dir(t.path(), "real");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let st = Command::new(EXE)
        .args(["provision-node-modules", "--types-node-modules"])
        .arg(&real)
        .arg("--playwright-test")
        .arg(&pw)
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &decoy)
        .env("E2E_PLAYWRIGHT_TEST", t.path().join("decoy-playwright"))
        .status()
        .unwrap();

    assert!(st.success());
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/typescript")).unwrap(),
        real.join("typescript")
    );
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/@playwright/test")).unwrap(),
        pw
    );
}

#[test]
fn unset_environment_names_the_variable_and_the_devshell() {
    let t = tmp();
    let out = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(t.path())
        .env_remove("E2E_TYPES_NODE_MODULES")
        .env_remove("E2E_PLAYWRIGHT_TEST")
        .output()
        .unwrap();

    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("E2E_TYPES_NODE_MODULES"), "got: {err}");
    assert!(err.contains("nix develop"), "got: {err}");
}

#[test]
fn unset_playwright_alone_names_its_own_variable() {
    // The types var resolving first must not mask the playwright one (spec A11).
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let out = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(t.path())
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env_remove("E2E_PLAYWRIGHT_TEST")
        .output()
        .unwrap();

    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("E2E_PLAYWRIGHT_TEST"), "got: {err}");
    assert!(err.contains("nix develop"), "got: {err}");
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml -p devtool --test provision_cli
```

Expected: PASS if Task 1 is complete and correct — these tests are written
against an already-implemented CLI, so this is a **verification** step, not a
red step. If any fail, fix Task 1's wiring (help text wording, `default_value`,
the dispatch arm) rather than weakening the test. In particular
`help_lists_the_flags_and_their_env_fallback` fails unless the doc comments on
`ProvisionArgs` name the two variables.

- [x] **Step 3: Run the whole tools suite, verify nothing regressed**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml
```

Expected: PASS — this is exactly what the `tools-test` gate step runs.

- [x] **Step 4: Commit**

Run `cargo xtask check` first, then:

```bash
git add tools/devtool/tests/provision_cli.rs
git commit -m "test(devtool): cover the provision-node-modules CLI surface (#229)"
```

---

### Task 3: `devtool check tsc` provisions in-process

**Files:**

- Modify: `tools/devtool/src/check.rs:110-138` (the doc comment and `run`), plus
  its `#[cfg(test)] mod tests` at `:140`

**Interfaces:**

- Consumes: `provision::resolve` and `provision::run` from Task 1.
- Produces: `pub fn needs_provisioning(name: &str) -> bool` — the pure predicate
  that makes "provisioning is `tsc`-only" testable without executing `tsc`.

- [x] **Step 1: Write the failing test**

Add to `check.rs`'s existing `mod tests`:

```rust
    #[test]
    fn only_tsc_needs_provisioning() {
        assert!(needs_provisioning("tsc"));
        for name in ALL.iter().filter(|n| **n != "tsc") {
            assert!(
                !needs_provisioning(name),
                "{name} must not provision end2end/node_modules"
            );
        }
    }
```

- [x] **Step 2: Run it, verify it fails**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml -p devtool only_tsc_needs_provisioning
```

Expected: FAIL — `cannot find function `needs_provisioning` in this scope`.

- [x] **Step 3: Implement against the test**

In `check.rs`, add above `run`:

```rust
/// Whether a check needs `end2end/node_modules` provisioned before it runs. Only
/// `tsc` type-checks against that closure. Kept as a pure predicate so the rule is
/// testable without executing tsc (#229).
pub fn needs_provisioning(name: &str) -> bool {
    name == "tsc"
}
```

Replace the `bash` spawn in `run` (currently `check.rs:119-127`) with:

```rust
        if needs_provisioning(n) {
            let types = crate::provision::resolve(None, "E2E_TYPES_NODE_MODULES")?;
            let playwright = crate::provision::resolve(None, "E2E_PLAYWRIGHT_TEST")?;
            crate::provision::run(std::path::Path::new("."), &types, &playwright)
                .context("provisioning end2end/node_modules for tsc")?;
        }
```

Passing `None` routes both callers through the single `resolve` message site
(Global Constraints). Update `run`'s doc comment at `:110-111` — it currently
says provisioning happens "via the shared script"; say it calls `provision::run`
in-process (A17). Drop the now-unused `Command`/`bash` import only if nothing
else in the file uses it — `run` still spawns each check's own program, so
`std::process::Command` stays.

- [x] **Step 4: Run the test, verify it passes**

Run:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo test --manifest-path tools/Cargo.toml -p devtool
```

Expected: PASS — the whole `tools/` suite.

- [x] **Step 5: Prove the in-process path really provisions**

```bash
rm -rf /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision/end2end/node_modules
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check tsc
```

Expected: exit 0, and `end2end/node_modules/@playwright/test` exists as a
symlink into the Nix store. (This is A13/A19's mechanism, exercised before the
flake changes land.)

- [x] **Step 6: Commit**

Run `cargo xtask check` first, then:

```bash
git add tools/devtool/src/check.rs
git commit -m "refactor(devtool): provision node_modules in-process for check tsc (#229)"
```

---

### Task 4: Retire the script — flake, ADR, deletion

**Files:**

- Modify: `flake.nix:1097-1098` (static-checks comment), `:1228-1251`
  (`ciInputs`), `:1252-1268` (`devOnly`), `:1280-1294` (env comment +
  `shellHook`)
- Modify: `docs/adr/0028-devtool-vs-xtask-boundary.md:109`
- Delete: `end2end/provision-node-modules.sh`

**Interfaces:**

- Consumes: the `devtool provision-node-modules` CLI (Task 1) and the in-process
  `check tsc` path (Task 3) — both callers must already be migrated before the
  script can go.
- Produces: nothing later tasks depend on. This is the last task.

- [x] **Step 1: Move `devtoolBin` into `ciInputs`**

The `shellHook` lives in `shellEnv`, which is spread into **both**
`devShells.ci` and `devShells.default`; `devtoolBin` is currently only in
`devOnly`, so a hook that calls `devtool` would break every CI shell entry
(A16). In `flake.nix`, delete these two lines plus their comment from `devOnly`
(`:1265-1267`):

```nix
              # `devtool run -- <cmd>` etc. on the interactive PATH. Already built
              # for the coverage sandbox; here it serves humans/agents directly.
              devtoolBin
```

and add to `ciInputs`, in the list's alphabetical slot (after `pkgs.curl`,
before `emacsForCi`):

```nix
              # `devtool run -- <cmd>` for humans/agents, and the `shellHook`'s
              # `devtool provision-node-modules` (#229) — so it must be on PATH in the
              # CI shell too, not just the interactive one. Already built for the
              # coverage and static-checks derivations, so this adds no new build.
              devtoolBin
```

- [x] **Step 2: Point the `shellHook` at the subcommand and reword the
      comments**

Replace `flake.nix:1289-1293` with:

```nix
                # Provision end2end/node_modules (the tsc type-dep closure) so the
                # devShell `tsc` and IDEs can type-check end2end/ offline in this
                # checkout. The same subcommand runs in-process from `devtool check
                # tsc`, so worktrees self-heal there; see tools/devtool/src/provision.rs
                # for the full rationale.
                devtool provision-node-modules
```

Reword `:1280-1283` to say the store paths are for
`devtool provision-node-modules` (not "for end2end/provision-node-modules.sh"),
keeping the existing point about why they are env vars rather than baked into
the hook. Reword `:1097-1098` — "tsc needs BOTH node-dep envs (the provision
script guards on each with `${VAR:?}`)" — to name
`devtool provision-node-modules` and its resolver instead of a script (A17).

- [x] **Step 3: Delete the script and amend ADR-0028**

```bash
git rm end2end/provision-node-modules.sh
```

In `docs/adr/0028-devtool-vs-xtask-boundary.md:109`, change "`devtoolBin` is
therefore exposed in the **default devShell** (direnv) in addition to the
coverage sandbox's `nativeBuildInputs`." to name **both** devShells, with a
short "(#229 put it in `ciInputs` too, because the shared `shellHook` invokes
`devtool provision-node-modules`)" clause (A18).

- [x] **Step 4: Verify nothing still references the script**

```bash
rg -n 'provision-node-modules\.sh|provision script|shared script' --glob '!docs/adr/0051*' --glob '!docs/archive' --glob '!docs/superpowers' .
rg -n 'cargo' flake.nix --glob flake.nix -A0 -e 'shellHook' || true
```

Expected from the first: **no hits at all** (A17). The excluded paths are
`docs/adr/0051-single-playwright-config.md` (historical, out of scope),
`docs/archive/`, and `docs/superpowers/` — this cycle's own spec and plan
necessarily name the script they retire, and **jaunder-ship** archives them into
`docs/archive/` at ship time, so A17's literal "no file outside `docs/adr/` and
`docs/archive/`" only becomes true after that archive step. Note that in the
ship review rather than trying to satisfy it here.

Second command is a manual read of the `shellHook` block: confirm it contains no
`cargo` invocation (A16's second half, otherwise true only by construction).

- [x] **Step 5: Run the fresh-provision gate**

```bash
rm -rf /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision/end2end/node_modules
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- cargo xtask check
```

Expected: PASS (`ok: true`). Read failures from the parked log with Grep, not a
pipe. Then check the tree was actually repopulated, rather than assuming it
(A19):

```bash
ls -l /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision/end2end/node_modules/@playwright/test /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision/end2end/node_modules/typescript
```

Expected: both listed as symlinks into `/nix/store/…`.

- [x] **Step 6: Run the Nix gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- nix build .#checks.x86_64-linux.static-checks
```

Expected: PASS — this is the prebuilt-`devtoolBin` path CI uses, which
`cargo xtask check` does not exercise (A20). Expect a rebuild: deleting the
script changes both `staticCheckSrc` and `e2ePackage`'s store path.

- [x] **Step 7: Manual devShell check (both shells)**

Two things block the obvious `nix develop --command true`: `devtool run` refuses
shell re-entry by design (ADR-0028), and a session hook rejects `nix develop -c`
as a wrapper anti-pattern. Verify the same two facts declaratively instead —
which is strictly more precise, since it reads the evaluated shell rather than
inferring from the absence of an error message:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- nix eval --raw .#devShells.x86_64-linux.ci.shellHook
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- nix eval --json .#devShells.x86_64-linux.ci.buildInputs --apply 'ps: builtins.filter (n: builtins.match ".*devtool.*" n != null) (map (p: p.name) ps)'
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-229-devtool-provision -- nix eval --json .#devShells.x86_64-linux.default.buildInputs --apply 'ps: builtins.filter (n: builtins.match ".*devtool.*" n != null) (map (p: p.name) ps)'
```

Expected: the hook body is `devtool provision-node-modules` with no `cargo`
anywhere (A15, A16 second half), and both `buildInputs` queries return
`["devtool-0.1.0"]`, so `devtool` is on PATH in each shell (A16 first half).

A live entry is still worth one human confirmation — the reviewer can run
`! nix develop --command true` in a session without that hook.

Expected: both exit 0 with **no** `devtool: command not found` on stderr —
A15/A16. Check the parked `.err` log for that message with Grep.

**Run these against the worktree, never `/home/mdorman/src/jaunder`.** The main
checkout is on a different branch and still has the old `flake.nix`, so a hook
that still says `bash end2end/provision-node-modules.sh` would pass this step
vacuously — it cannot fail for the reason it exists. Nix evaluates the
worktree's own `flake.nix` from `--cwd`, exactly as Step 6 relies on.

- [x] **Step 8: Commit**

Run `cargo xtask check` first, then:

```bash
git add flake.nix docs/adr/0028-devtool-vs-xtask-boundary.md end2end/provision-node-modules.sh
git commit -m "tooling(dx): retire provision-node-modules.sh for the devtool subcommand (#229)"
```

---

## Self-review

**Spec coverage.** A1–A4 → Task 2's CLI tests (help text, env defaulting,
`--root`, flag-beats-env). A5–A9 → Task 1's tempdir tests. A10 → Task 1's
`resolve` test plus Task 2's
`unset_environment_names_the_variable_and_the_devshell`; A11 → Task 2's
`unset_playwright_alone_names_its_own_variable`. A12 → Task 1's
`errors_when_types_dir_missing`. A13 → Task 3 steps 3 and 5. A14 → Task 3's
`only_tsc_needs_provisioning`. A15 → Task 4 step 2, exercised by step 7's
worktree-pinned `nix develop`. A16 → Task 4 step 1 (`ciInputs`) and step 7 for
the PATH half, step 4's second command for the no-`cargo` half. A17 → Task 4
steps 2–4, with the caveat recorded in step 4 that the literal criterion only
holds once **jaunder-ship** archives this spec and plan. A18 → Task 4 step 3.
A19 → Task 4 step 5, including the `ls -l` that checks the tree instead of
assuming it. A20 → Task 4 step 6. No spec criterion is unmapped.

**Type consistency.** `resolve(Option<PathBuf>, &str) -> Result<PathBuf>` and
`run(&Path, &Path, &Path) -> Result<()>` are used identically in Task 1's
dispatch arm, Task 1's tests, and Task 3's call site.
`needs_provisioning(&str) -> bool` is defined and consumed only in Task 3. Flag
names (`--types-node-modules`, `--playwright-test`, `--root`) match between the
`ProvisionArgs` derive and Task 2's assertions.
