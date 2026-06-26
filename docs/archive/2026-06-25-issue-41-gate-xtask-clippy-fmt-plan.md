# Gate xtask clippy + fmt Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add xtask-workspace clippy + fmt to `cargo xtask check`/`validate`, refactoring the static-check step list into pure, testable data so the addition carries a regression test.

**Architecture:** Split `xtask/src/steps/static_checks.rs` into a pure `specs(mode) -> Vec<StepSpec>` builder and a thin `run` executor that shells each spec out via the existing `step()`. Existing steps keep identical names/programs/args/order; two new steps (`xtask-fmt`, `xtask-clippy`) append after `tools-clippy`.

**Tech Stack:** Rust, xshell, the xtask dev-driver. Spec source: `docs/superpowers/specs/2026-06-25-issue-41-gate-xtask-clippy-fmt.md`.

## Global Constraints

- xtask runs **host-only**; never invoked from a Nix derivation. (CLAUDE.md invariant.)
- **No coverage instrumentation for xtask** — same scope boundary as #38. In-file tests are fine here (xtask is coverage-exempt; `nix.rs`/`result.rs` already keep in-file tests).
- `xtask/` is a `[workspace]` *with* a root `[package]` (not a virtual workspace), so its fmt/clippy take **no `--all`** — unlike the `tools/` virtual workspace.
- `step()` signature is `step(sh: &Shell, name: &str, program: &str, args: &[&str]) -> StepResult`.
- `Mode` is `#[derive(Clone, Copy)]` with variants `Check` and `Fix`.
- Commit convention: conventional-commit subject, no `Co-Authored-By` trailers.
- The gate is the source of truth for done: `cargo xtask validate --no-e2e` green before committing.

---

### Task 1: Refactor static_checks to a testable spec list + add xtask clippy/fmt

**Files:**
- Modify: `xtask/src/steps/static_checks.rs` (whole file — refactor + 2 new steps + tests)

**Interfaces:**
- Consumes: `crate::result::{CommandResult, Mode}`, `crate::sh::step`.
- Produces:
  - `pub struct StepSpec { pub name: &'static str, pub program: &'static str, pub args: Vec<&'static str> }`
  - `pub fn specs(mode: Mode) -> Vec<StepSpec>` (pure)
  - `pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult)` (unchanged signature)

- [x] **Step 1: Write the failing tests**

Append to `xtask/src/steps/static_checks.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(specs: &'a [StepSpec], name: &str) -> &'a StepSpec {
        specs.iter().find(|s| s.name == name).expect("step present")
    }

    #[test]
    fn xtask_fmt_checks_in_check_mode() {
        let s = specs(Mode::Check);
        let xtask_fmt = find(&s, "xtask-fmt");
        assert_eq!(xtask_fmt.program, "cargo");
        assert_eq!(
            xtask_fmt.args,
            ["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"]
        );
    }

    #[test]
    fn xtask_fmt_writes_in_fix_mode() {
        let s = specs(Mode::Fix);
        let xtask_fmt = find(&s, "xtask-fmt");
        assert_eq!(
            xtask_fmt.args,
            ["fmt", "--manifest-path", "xtask/Cargo.toml"]
        );
    }

    #[test]
    fn xtask_clippy_denies_warnings_in_both_modes() {
        for mode in [Mode::Check, Mode::Fix] {
            let s = specs(mode);
            let xtask_clippy = find(&s, "xtask-clippy");
            assert_eq!(xtask_clippy.program, "cargo");
            assert_eq!(
                xtask_clippy.args,
                [
                    "clippy",
                    "--manifest-path",
                    "xtask/Cargo.toml",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings"
                ]
            );
        }
    }

    #[test]
    fn step_order_is_locked() {
        let expected = [
            "fmt",
            "leptosfmt",
            "prettier",
            "cargo-deny",
            "clippy",
            "tools-fmt",
            "tools-clippy",
            "xtask-fmt",
            "xtask-clippy",
        ];
        for mode in [Mode::Check, Mode::Fix] {
            let names: Vec<&str> = specs(mode).iter().map(|s| s.name).collect();
            assert_eq!(names, expected);
        }
    }
}
```

- [x] **Step 2: Run the tests to verify they fail to compile**

Run: `cargo test --manifest-path xtask/Cargo.toml static_checks`
Expected: FAIL — `cannot find function specs` / `cannot find type StepSpec` (they don't exist yet).

- [x] **Step 3: Refactor `run` into `specs` + executor and add the xtask steps**

Replace the body of `xtask/src/steps/static_checks.rs` *above* the test module with:

```rust
use xshell::Shell;

use crate::result::{CommandResult, Mode};
use crate::sh::step;

/// A single static-check step: a named command and its arguments, already
/// resolved for the active `Mode`.
pub struct StepSpec {
    pub name: &'static str,
    pub program: &'static str,
    pub args: Vec<&'static str>,
}

/// The ordered static-check steps for `mode`. Pure (no I/O) so the step list
/// and its mode-dependent arguments can be unit-tested without shelling out.
///
/// Command invocations are kept verbatim with `scripts/verify` Phase 1 + 2,
/// adjusted only for the Fix/Check switch on the formatting tools. `tools/` is a
/// virtual workspace (needs `--all`); `xtask/` has a root package (no `--all`).
pub fn specs(mode: Mode) -> Vec<StepSpec> {
    let fmt_args = match mode {
        Mode::Check => vec!["fmt", "--check"],
        Mode::Fix => vec!["fmt"],
    };
    let leptos_args = match mode {
        Mode::Check => vec![
            "-x", ".direnv", "-x", ".git", "-x", "target", "--check", "**/*.rs",
        ],
        Mode::Fix => vec!["-x", ".direnv", "-x", ".git", "-x", "target", "**/*.rs"],
    };
    let prettier_args = match mode {
        Mode::Check => vec!["--check", "end2end"],
        Mode::Fix => vec!["-w", "end2end"],
    };
    let tools_fmt_args = match mode {
        Mode::Check => vec![
            "fmt",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all",
            "--check",
        ],
        Mode::Fix => vec!["fmt", "--manifest-path", "tools/Cargo.toml", "--all"],
    };
    let xtask_fmt_args = match mode {
        Mode::Check => vec!["fmt", "--manifest-path", "xtask/Cargo.toml", "--check"],
        Mode::Fix => vec!["fmt", "--manifest-path", "xtask/Cargo.toml"],
    };

    vec![
        StepSpec {
            name: "fmt",
            program: "cargo",
            args: fmt_args,
        },
        StepSpec {
            name: "leptosfmt",
            program: "leptosfmt",
            args: leptos_args,
        },
        StepSpec {
            name: "prettier",
            program: "prettier",
            args: prettier_args,
        },
        StepSpec {
            name: "cargo-deny",
            program: "cargo",
            args: vec!["deny", "check"],
        },
        StepSpec {
            name: "clippy",
            program: "cargo",
            args: vec!["clippy", "--all-targets", "--", "-D", "warnings"],
        },
        StepSpec {
            name: "tools-fmt",
            program: "cargo",
            args: tools_fmt_args,
        },
        StepSpec {
            name: "tools-clippy",
            program: "cargo",
            args: vec![
                "clippy",
                "--manifest-path",
                "tools/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
        StepSpec {
            name: "xtask-fmt",
            program: "cargo",
            args: xtask_fmt_args,
        },
        StepSpec {
            name: "xtask-clippy",
            program: "cargo",
            args: vec![
                "clippy",
                "--manifest-path",
                "xtask/Cargo.toml",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        },
    ]
}

/// Run the static check suite. In `Mode::Fix`, formatting commands auto-fix in
/// place; in `Mode::Check`, every command is read-only — safe for CI.
pub fn run(sh: &Shell, mode: Mode, result: &mut CommandResult) {
    for spec in specs(mode) {
        result.push(step(sh, spec.name, spec.program, &spec.args));
    }
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test --manifest-path xtask/Cargo.toml static_checks`
Expected: PASS — all four tests green.

- [x] **Step 5: Run the full gate to confirm xtask is itself clippy/fmt-clean now that it's gated**

Run (from the worktree): `cargo xtask validate --no-e2e`
Expected: exit 0 (the new `xtask-fmt`/`xtask-clippy` steps pass on the current tree). If clippy flags anything in `xtask/`, fix it — the gate now catches what it previously missed.

- [x] **Step 6: Acceptance verification — fault injection**

Confirm the gate actually goes red on xtask drift (revert each injection after observing):

1. fmt drift: add stray indentation to a line in `xtask/src/main.rs`, run `cargo xtask validate --no-e2e` → expect RED at `xtask-fmt`; run `cargo xtask check --no-test` → expect it auto-fixes (tree clean again). 
2. clippy lint: add an obviously-lintable construct (e.g. `let _x = (0..1).len();` — `clippy::range_zip_with_len`/`needless_range_loop`-style, or a simpler `let v: Vec<i32> = vec![]; if v.len() == 0 {}` for `len_zero`) to an xtask source file, run `cargo xtask check --no-test` → expect RED at `xtask-clippy`. Revert.

Expected: both injections turn the gate red at the right step; reverting restores green.

- [x] **Step 7: Commit**

```bash
git add xtask/src/steps/static_checks.rs \
        docs/superpowers/specs/2026-06-25-issue-41-gate-xtask-clippy-fmt.md \
        docs/superpowers/plans/2026-06-25-issue-41-gate-xtask-clippy-fmt.md
git commit -m "feat(xtask): gate xtask workspace clippy + fmt in check/validate (#41)"
```

(Commit only after Step 5's `cargo xtask validate --no-e2e` is green and Step 6's injections are reverted.)

---

## Self-Review

**Spec coverage:**
- Refactor to `specs`/`run` + `StepSpec` → Step 3. ✓
- xtask-fmt (mode toggle, no `--all`) → Step 3 + tests 1–2. ✓
- xtask-clippy (both modes, `-D warnings`) → Step 3 + test 3. ✓
- Order-lock regression test → test 4 (Step 1). ✓
- Existing steps' behavior unchanged → preserved verbatim in `specs`; order-lock test guards order. ✓
- Manual acceptance (check red on lint, validate red + check auto-fix on fmt) → Step 6. ✓
- Out-of-scope (coverage instrumentation, app-clippy-via-Nix) → not touched. ✓

**Placeholder scan:** No TBD/TODO; all code blocks complete; commands have expected output.

**Type consistency:** `StepSpec`/`specs`/`run` names and the `step()` call (`&spec.args` → `&[&str]`) match across steps; step names in test 4 match the `StepSpec` literals in Step 3 exactly.
