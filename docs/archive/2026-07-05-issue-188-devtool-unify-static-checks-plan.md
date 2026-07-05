# Unify static checks via devtool (#188) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating a task to a subagent via **jaunder-dispatch**
> when useful). Steps use checkbox (`- [ ]`) syntax.

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-188-devtool-unify-static-checks.md`](../specs/2026-07-05-issue-188-devtool-unify-static-checks.md)
— the "what/why". This plan is the "how".

**Goal:** Move the 7 non-compiling static checks into one `devtool check`
implementation the host ladder and a nix `static-checks` derivation both call;
keep `clippy`/`deny` crane; `tools-clippy`/`xtask-*` host-only.

**Architecture:** `devtool` gains a `check` module owning the 7 checks'
tool+args (moved from `xtask/src/steps/static_checks.rs`). Host StepSpecs call
`cargo run -p devtool -- check <name>`; one nix `static-checks` `runCommand`
calls `devtool check --all`. Deletes the 5 hand-written sibling `runCommand`s
(#185 divergence class).

**Tech Stack:** Rust (devtool in `tools/`, xtask host), Nix flake.

## Global Constraints

- **Migrated (7, non-compiling):** `fmt`, `leptosfmt`, `prettier`, `tsc`,
  `elisp-fmt`, `ert`, `tools-fmt`. **Not migrated:** `clippy`/`deny` (crane,
  untouched), `tools-clippy`/`xtask-fmt`/`xtask-clippy` (host-only native
  StepSpecs).
- **`--fix` flag** (default = verify) drives the 5 formatters; `ert`/`tsc`
  ignore it.
- **Args are moved, not copied** — each migrated check's tool+args live once (in
  `devtool`); `static_checks.rs` loses them.
- **`tsc` folds in `tsc-deps`:** `devtool check tsc` runs
  `end2end/provision-node-modules.sh` (uses `E2E_TYPES_NODE_MODULES`) then
  `tsc`.
- **Nix `static-checks` must set `TZDIR=${pkgs.tzdata}/share/zoneinfo`** (the
  `ert` suite needs a zone DB — the old `ert-check` did, #160) and
  `E2E_TYPES_NODE_MODULES=${e2ePackage}/node_modules` (tsc), and run over a
  **writable copy** of a broad src (tsc provisioning writes
  `end2end/node_modules`).
- **Unified `prettier` = `--check end2end
  **/_.md`** (host superset; the old nix `prettier-check`only did`end2end`— the #185 gap this closes), so the static-checks src includes all`_.md`.
- **Commits:** run `cargo xtask check` clean first (**jaunder-commit**). **No
  `Co-Authored-By` trailer.**

---

## Review header

**Scope — in:** `tools/devtool` (new `check` module),
`xtask/src/steps/static_checks.rs` (rewire the 7), `flake.nix` (add
`static-checks`, delete 5 siblings), a new ADR draft + ADR-0031 Note, docs.
**Out:** `clippy`/`deny`/`tools-clippy`/`xtask-*`; CI; coverage/e2e. **Separable
concerns:** none.

**Tasks:**

1. `devtool check` module + CLI (the 7 checks' tool+args + runner; tsc
   node-provision). — AC1
2. Rewire `xtask` `static_checks` → `devtool check` for the 7; drop `tsc-deps`;
   keep native ones. — AC2, AC4(host)
3. Nix `static-checks` `runCommand`; delete the 5 siblings; keep
   `clippy`/`deny`. — AC3, AC4(nix), AC5
4. New ADR draft + amend ADR-0031 (path-form Note). — AC6
5. Docs: CONTRIBUTING, elisp/README, run-tests.el. — AC7

**Key risks/decisions:** the soundness review forced the compiling/non-compiling
cleave — only the 7 non-compiling checks move (a `runCommand` needs no
vendoring); `clippy`/`deny` stay crane. `TZDIR` for `ert` and the writable-src
copy for `tsc` are the two easy-to-miss nix details. The
`devtoolBin`↔`coverage` cache coupling is accepted (first gate after Task 1
rebuilds coverage once).

---

### Task 1: `devtool check` module + CLI

**Files:**

- Create: `tools/devtool/src/check.rs`
- Modify: `tools/devtool/src/main.rs` (`mod check;`, `Command::Check`, dispatch)

**Interfaces:**

- Consumes: `std::process::Command`, `anyhow` (the module idiom — cf. `pg.rs`).
- Produces: `Command::Check(CheckArgs)`;
  `check::run(name: Option<&str>, all: bool, fix: bool) -> anyhow::Result<()>`;
  pure
  `check::spec(name: &str, fix: bool) -> Result<(&'static str, Vec<String>)>`.

- [x] **Step 1: Write the failing tests** (in-file `#[cfg(test)]` in `check.rs`,
      mirroring `pg.rs`'s pure-arg tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_check_vs_fix() {
        assert_eq!(spec("fmt", false).unwrap(), ("cargo", vec!["fmt".to_string(), "--check".into()]));
        assert_eq!(spec("fmt", true).unwrap(), ("cargo", vec!["fmt".to_string()]));
    }

    #[test]
    fn prettier_covers_end2end_and_markdown() {
        // The #185 fix: unified prettier checks end2end AND all markdown.
        let (_p, args) = spec("prettier", false).unwrap();
        assert!(args.contains(&"--check".to_string()));
        assert!(args.contains(&"end2end".to_string()));
        assert!(args.contains(&"**/*.md".to_string()));
    }

    #[test]
    fn ert_and_tsc_ignore_fix() {
        assert_eq!(spec("ert", true).unwrap(), spec("ert", false).unwrap());
        assert_eq!(spec("tsc", true).unwrap(), spec("tsc", false).unwrap());
    }

    #[test]
    fn tools_fmt_targets_tools_workspace() {
        let (_p, args) = spec("tools-fmt", false).unwrap();
        assert!(args.windows(2).any(|w| w == ["--manifest-path", "tools/Cargo.toml"]));
        assert!(args.contains(&"--all".to_string()) && args.contains(&"--check".to_string()));
    }

    #[test]
    fn unknown_check_errors() {
        assert!(spec("nope", false).is_err());
    }

    #[test]
    fn all_names_have_specs() {
        for n in ALL { assert!(spec(n, false).is_ok(), "{n}"); }
    }
}
```

- [x] **Step 2: Run, verify FAIL**

Run: `cargo nextest run --manifest-path tools/Cargo.toml -p devtool check::`
Expected: FAIL — `check` module / `spec` undefined.

- [x] **Step 3: Implement**

`check.rs` — move the 7 checks' tool+args from `static_checks.rs::specs`
(verbatim, incl. the mode switch on the 5 formatters); `spec` is pure (tested);
`run` shells them out; `tsc` provisions node-deps first.

```rust
use std::process::Command;

use anyhow::{bail, Context, Result};

/// The 7 non-compiling static checks devtool owns (#188). Order = the host gate order.
pub const ALL: &[&str] = &["fmt", "leptosfmt", "prettier", "tsc", "elisp-fmt", "ert", "tools-fmt"];

/// Pure: the (program, args) for `name` in the given mode. `fix` makes the formatters
/// (`fmt`, `leptosfmt`, `prettier`, `elisp-fmt`, `tools-fmt`) mutate; `ert`/`tsc` ignore it.
/// Args verbatim from the former `static_checks::specs` (single source of truth now).
fn spec(name: &str, fix: bool) -> Result<(&'static str, Vec<String>)> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    Ok(match name {
        "fmt" => ("cargo", if fix { s(&["fmt"]) } else { s(&["fmt", "--check"]) }),
        "leptosfmt" => (
            "leptosfmt",
            if fix { s(&["-x", ".direnv", "-x", ".git", "-x", "target", "**/*.rs"]) }
            else { s(&["-x", ".direnv", "-x", ".git", "-x", "target", "--check", "**/*.rs"]) },
        ),
        "prettier" => (
            "prettier",
            if fix { s(&["-w", "end2end", "**/*.md"]) } else { s(&["--check", "end2end", "**/*.md"]) },
        ),
        "tsc" => ("tsc", s(&["--noEmit", "-p", "end2end/tsconfig.json"])),
        "elisp-fmt" => (
            "emacs",
            if fix { s(&["--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-fix"]) }
            else { s(&["--batch", "-Q", "-l", "elisp/scripts/format.el", "-f", "jaunder-fmt-check"]) },
        ),
        "ert" => ("emacs", s(&["--batch", "-Q", "-l", "elisp/scripts/run-tests.el"])),
        "tools-fmt" => (
            "cargo",
            if fix { s(&["fmt", "--manifest-path", "tools/Cargo.toml", "--all"]) }
            else { s(&["fmt", "--manifest-path", "tools/Cargo.toml", "--all", "--check"]) },
        ),
        other => bail!("unknown check '{other}' (known: {ALL:?})"),
    })
}

/// Run one check by name, or all of them. `tsc` provisions `end2end/node_modules` first.
pub fn run(name: Option<&str>, all: bool, fix: bool) -> Result<()> {
    let names: Vec<&str> = match (name, all) {
        (Some(n), false) => vec![n],
        (None, true) => ALL.to_vec(),
        _ => bail!("pass exactly one of <name> or --all"),
    };
    for n in &names {
        if *n == "tsc" {
            let st = Command::new("bash").arg("end2end/provision-node-modules.sh").status()
                .context("provisioning end2end/node_modules for tsc")?;
            if !st.success() { bail!("tsc-deps (provision-node-modules.sh) failed ({st})"); }
        }
        let (program, args) = spec(n, fix)?;
        let st = Command::new(program).args(&args).status()
            .with_context(|| format!("spawning `{program}` for check {n}"))?;
        if !st.success() { bail!("check {n} failed ({st})"); }
    }
    Ok(())
}
```

`main.rs`: `mod check;`, add `Command::Check(CheckArgs)`
(`#[derive(clap::Args)] struct CheckArgs { name: Option<String>, #[arg(long)] all: bool, #[arg(long)] fix: bool }`),
dispatch `Command::Check(a) => check::run(a.name.as_deref(), a.all, a.fix)`.

- [x] **Step 4: Run, verify PASS** —
      `cargo nextest run --manifest-path tools/Cargo.toml -p devtool check::` →
      PASS.

- [x] **Step 5: Smoke-test the CLI** —
      `cargo run --manifest-path tools/Cargo.toml -p devtool -- check fmt` runs
      `cargo fmt --check` (exit 0 on a formatted tree); `devtool check --all`
      runs all 7.

- [x] **Step 6: Commit**

```bash
git add tools/devtool/src/check.rs tools/devtool/src/main.rs
git commit -m "feat(devtool): check subcommand — the 7 non-compiling static checks (#188)"
```

Run `cargo xtask check` first (**jaunder-commit**). Note: this commit changes
`tools/`, so the pre-commit `coverage` check rebuilds once (accepted
`devtoolBin` coupling).

---

### Task 2: Rewire `xtask` `static_checks` to `devtool check`

**Files:**

- Modify: `xtask/src/steps/static_checks.rs`

**Interfaces:**

- Consumes: the `devtool check` CLI (Task 1); `StepSpec`, `Mode`.
- Produces: `fn devtool_check(name: &'static str, mode: Mode) -> StepSpec`
  (private).

- [x] **Step 1: Adjust the tests**

First **delete/update the existing tests that assert the migrated checks' args
or the dropped `tsc-deps`** (they move to `devtool`):
`tsc_deps_provisions_before_tsc_in_both_modes`, `tsc_typechecks_in_both_modes`,
`elisp_fmt_checks_in_check_writes_in_fix`,
`ert_runs_the_batch_runner_in_both_modes` (delete — now devtool's), and any
`step_order`/ order-locking test whose `expected` array lists `tsc-deps` or the
migrated names as `cargo fmt`/`leptosfmt`/etc. (update to the new
`devtool check` order, tsc-deps removed). **Keep**
`xtask_fmt_checks_in_check_mode` (xtask-fmt stays native). Then add:

```rust
#[test]
fn migrated_checks_delegate_to_devtool() {
    let s = specs(Mode::Check);
    let fmt = find(&s, "fmt");
    assert_eq!(fmt.program, "cargo");
    assert_eq!(
        fmt.args,
        ["run", "--quiet", "--manifest-path", "tools/Cargo.toml", "-p", "devtool", "--", "check", "fmt"]
    );
    let fix = find(&specs(Mode::Fix), "prettier");
    assert!(fix.args.contains(&"--fix"), "fix mode passes --fix: {:?}", fix.args);
    // tsc-deps is gone (folded into `devtool check tsc`).
    assert!(specs(Mode::Check).iter().all(|s| s.name != "tsc-deps"));
}

#[test]
fn native_checks_stay_native() {
    // clippy / cargo-deny / tools-clippy / xtask-fmt / xtask-clippy still run cargo directly.
    let s = specs(Mode::Check);
    assert_eq!(find(&s, "clippy").args, ["clippy", "--all-targets", "--", "-D", "warnings"]);
    assert_eq!(find(&s, "xtask-clippy").program, "cargo");
}
```

(Keep the existing `xtask_fmt_checks_in_check_mode` test — xtask-fmt stays
native.)

- [x] **Step 2: Run, verify FAIL** — `cargo nextest run -p xtask static_checks`
      → FAIL (fmt still runs cargo directly; tsc-deps still present).

- [x] **Step 3: Implement**

Add the helper; replace the 7 migrated `StepSpec`s with
`devtool_check(name, mode)`; delete the `tsc-deps` step and the now-unused arg
builders (`fmt_args`, `leptos_args`, `prettier_args`, `elisp_fmt_args`,
`tools_fmt_args`); keep `xtask_fmt_args` and the native
`clippy`/`cargo-deny`/`tools-clippy`/`xtask-*` StepSpecs.

```rust
/// A migrated static check: run it through `devtool check <name>` (single source of
/// truth for its tool+args) via `cargo run` so a local `tools/` edit is reflected. The
/// nix `static-checks` derivation runs the same `devtool check` from `devtoolBin`.
fn devtool_check(name: &'static str, mode: Mode) -> StepSpec {
    let mut args = vec![
        "run", "--quiet", "--manifest-path", "tools/Cargo.toml", "-p", "devtool", "--",
        "check", name,
    ];
    if matches!(mode, Mode::Fix) {
        args.push("--fix");
    }
    StepSpec { name, program: "cargo", args }
}
```

New `specs(mode)` vec order (unchanged relative order; tsc-deps removed):
`devtool_check("fmt")`, `devtool_check("leptosfmt")`,
`devtool_check("prettier")`, `devtool_check("tsc")`,
`devtool_check("elisp-fmt")`, `devtool_check("ert")`, native `cargo-deny`,
native `clippy`, `devtool_check("tools-fmt")`, native `tools-clippy`, native
`xtask-fmt`, native `xtask-clippy`.

- [x] **Step 4: Run, verify PASS** — `cargo nextest run -p xtask static_checks`
      → PASS.

- [x] **Step 5: Verify the host gate end-to-end** —
      `cargo xtask check --no-test` runs green (the migrated steps now shell
      `devtool check`; first run builds devtool). Confirms AC2 on the host.

- [x] **Step 6: Commit**

```bash
git add xtask/src/steps/static_checks.rs
git commit -m "build(xtask): route the 7 non-compiling static checks through devtool check (#188)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 3: Nix `static-checks` derivation; delete the 5 siblings

**Files:**

- Modify: `flake.nix`

**Interfaces:**

- Consumes: `devtoolBin`, `e2ePackage`, `emacsForCi`, the rust toolchain the
  crane derivations use (cargo + rustfmt),
  `pkgs.{leptosfmt,prettier,nodejs,typescript,tzdata}`.

- [x] **Step 1: Add `static-checks`, delete the 5 siblings**

In the `checks` attrset: **delete** `rustfmt` (1225), `leptosfmt-check` (1230),
`prettier-check` (1342), `ert-check` (1351), `elisp-fmt-check` (1365). **Keep**
`clippy` (1218) and `deny` (1240). **Also delete the now-dead `let`-binding
`end2endSrc`** (defined ~666, was used only by the deleted `prettier-check`;
`static-checks` builds its own broad src). Leave `emacsSrc` (still used by
`e2e-elisp-integration`).

The rust-toolchain binding is **`toolchain`** (flake.nix:270 —
`fenix … fromToolchainFile ./rust-toolchain.toml`, whose `components` include
`rustfmt`, so it provides both `cargo` and `rustfmt`); the tsc env vars are the
two the devShell sets (flake.nix:1435–1436) — the provision script **requires
both**. Add:

```nix
static-checks =
  let
    staticCheckSrc = pkgs.lib.cleanSourceWith {
      src = craneLib.path ./.;
      # The 7 non-compiling checks need rust + end2end/ + elisp/ + tools/ + all *.md +
      # the prettier config (.prettierrc.json/.prettierignore). Broad, but this derivation
      # is cheap (no compile), so a whole-tree bust is fine. Exclusion-only (unlike the
      # coverage src's admission allowlist), so keep the working tree clean when building
      # locally — a stray untracked .rs/.md would enter the src.
      filter = path: _type:
        !(pkgs.lib.hasInfix "/xtask/" path)
        && !(pkgs.lib.hasInfix "/node_modules" path)
        && !(pkgs.lib.hasInfix "/target/" path)
        && !(pkgs.lib.hasInfix "/.direnv/" path);
    };
  in
  pkgs.runCommand "static-checks"
    {
      nativeBuildInputs = [
        devtoolBin
        toolchain              # fenix toolchain (cargo + rustfmt) — flake.nix:270
        pkgs.leptosfmt
        pkgs.prettier
        pkgs.nodejs
        pkgs.typescript
        emacsForCi
      ];
      # ert needs a zone DB (#160, as the old ert-check set); tsc needs BOTH node-dep envs
      # (provision-node-modules.sh guards on each with `${VAR:?}`).
      TZDIR = "${pkgs.tzdata}/share/zoneinfo";
      E2E_TYPES_NODE_MODULES = "${e2ePackage}/node_modules";
      E2E_PLAYWRIGHT_TEST = "${pkgs.playwright-test}/lib/node_modules/@playwright/test";
    }
    ''
      # Writable copy: `devtool check tsc` provisions end2end/node_modules (symlink).
      cp --no-preserve=mode -r ${staticCheckSrc} src
      cd src
      devtool check --all
      touch $out
    '';
```

Confirm `.prettierrc.json` + `.prettierignore` (tracked root files) land in
`staticCheckSrc` (the exclusion filter keeps them) so the sandbox prettier
checks the same file set / `proseWrap` as the host.

- [x] **Step 2: Verify the attr-set delta (AC3/AC5)**

Run: `devtool run -- nix eval .#checks.x86_64-linux --apply builtins.attrNames`
Expected: contains `static-checks`, `clippy`, `deny`, `coverage`, `e2e-*`; NOT
`rustfmt`/`leptosfmt-check`/`prettier-check`/`ert-check`/`elisp-fmt-check`. Run:
`devtool run -- nix flake check --no-build` → clean eval (no dangling ref).

- [x] **Step 3: Build it (AC3)**

Run (background — first build compiles nothing but provisions tools):
`devtool run -- nix build .#checks.x86_64-linux.static-checks -L` Expected:
passes; the log shows `devtool check --all` running
fmt/leptosfmt/prettier/tsc/elisp-fmt/ert/tools-fmt. If `tsc` can't find types →
`E2E_TYPES_NODE_MODULES`/provision wiring; if `ert` zone errors → `TZDIR`.

- [x] **Step 4: Commit**

```bash
git add flake.nix
git commit -m "build(nix): one static-checks derivation via devtool; drop the 5 redundant siblings (#188)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 4: New ADR draft + amend ADR-0031

**Files:**

- Create: `docs/adr/0052-devtool-unifies-static-checks.md` (numberless)
- Modify: `docs/adr/0031-elisp-separately-tested-subproject.md` (add a Note)

**Interfaces:** the **jaunder-adr** draft-out-of-git flow
(`docs/adr/template.md`; `# ADR-DRAFT:` heading; numbered at ship by
`cargo xtask adr promote`, which rewrites the draft-path references repo-wide).

- [x] **Step 1: Write the draft** — from `docs/adr/template.md`, heading
      `# ADR-DRAFT: devtool is the single implementation of the non-compiling static checks`.
      Record the **compile/non-compile cleave** (per spec Decision 6):
      non-compiling checks unified in `devtool` (host + `nix flake check`);
      `clippy`/`deny` stay crane; `tools-clippy`/`xtask-*` host-only; the
      `devtoolBin`↔coverage cache tradeoff (accepted). Amends ADR-0031.

- [x] **Step 2: Amend ADR-0031** — keep `- Status: accepted`; add under the
      status:
      `- Note: decisions #2–#3's hermetic `ert-check`/`elisp-fmt-check`nix siblings are retired by [ADR-DRAFT](0052-devtool-unifies-static-checks.md) (#188) — those checks now run via`devtool
      check`(host + nix); the host StepSpecs stand. The Consequences line "…and by`nix
      flake check`" now holds *through `devtool`*, not a hermetic sibling.`
      (Path-form reference so `adr promote` renumbers it at ship.)

- [x] **Step 3: Verify** — `cargo xtask check --no-test` →
      `adr-format`/`adr-readme-parity` green (a numberless draft is invisible to
      the gates; the 0031 edit keeps its canonical heading/status).

- [x] **Step 4: Commit**

```bash
git add docs/adr/0052-devtool-unifies-static-checks.md docs/adr/0031-elisp-separately-tested-subproject.md
git commit -m "docs(adr): devtool unifies the non-compiling static checks; amend ADR-0031 (#188)"
```

Run `cargo xtask check` first.

---

### Task 5: Docs

**Files:**

- Modify: `CONTRIBUTING.md`, `elisp/README.md`, `elisp/scripts/run-tests.el`

- [x] **Step 1: Update the prose**
  - `CONTRIBUTING.md` "Nix VM checks" list: replace the
    `rustfmt`/`leptosfmt-check`/`prettier-check`/`ert-check`/`elisp-fmt-check`
    entries with a single `static-checks` (`devtool check --all`); keep
    `clippy`/`deny`. Describe the verify ladder as the enforced path and
    `devtool` as the shared static-check runner.
  - `elisp/README.md` (~36–38): the `ert`/`elisp-fmt` checks run via
    `devtool check` (host + `nix flake check`'s `static-checks`), not hermetic
    `*-check` siblings.
  - `elisp/scripts/run-tests.el`: fix the comment that references the hermetic
    nix check.

- [x] **Step 2: Verify** — `cargo xtask check --no-test` → `prettier`/`adr-*`
      green.

- [x] **Step 3: Commit**

```bash
git add CONTRIBUTING.md elisp/README.md elisp/scripts/run-tests.el
git commit -m "docs: describe the devtool-unified static checks (#188)"
```

Run `cargo xtask check` first.

---

## Self-review

- **Spec coverage:** AC1 → T1; AC2 → T2; AC3 → T3; AC4 → T1 (single source) +
  T2/T3 (no residual for the 7); AC5 → T3/S2; AC6 → T4; AC7 → T5. All mapped.
- **Placeholders:** none — real Rust/Nix + exact commands. The one lookup left
  explicit is the rust-toolchain binding name in T3 (grep the flake).
- **Type consistency:** `spec(&str,bool)->Result<(&'static str,Vec<String>)>`
  and `run(Option<&str>,bool,bool)` (T1) match the `main.rs` dispatch and the
  `devtool check <name> [--fix]`/`--all` CLI that T2's `devtool_check` and T3's
  `devtool check --all` invoke.
