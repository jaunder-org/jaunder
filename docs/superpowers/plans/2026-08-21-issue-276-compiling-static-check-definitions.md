# Compiling Static-Check Definitions Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move product/tool compiling static-check command definitions into
`devtool check` while preserving host-local performance lanes and hermetic Nix
validation.

**Architecture:** `tools/devtool/src/check.rs` becomes the single command-spec
surface for `clippy`, `wasm-clippy`, `cargo-deny`, and `tools-clippy`. Host
`xtask` delegates to `devtool check <name>` but keeps Rust compile-cache env on
Rust-compiling checks; Nix `static-checks` expands to the same definitions in
sandbox Cargo mode and replaces the old crane clippy/deny check derivations on
the required validation path.

**Tech Stack:** Rust (`tools/devtool`, `xtask`), Cargo/clippy/cargo-deny,
crane/Nix `flake.nix`, GitHub CI, Markdown docs/ADR.

## Global Constraints

- Spec:
  `docs/superpowers/specs/2026-08-21-issue-276-compiling-static-check-definitions.md`.
- Keep `xtask-fmt` and `xtask-clippy` native host checks; `xtask/` is excluded
  from the flake source.
- Preserve host compile-cache env for Rust-compiling delegated checks:
  `RUSTC_WRAPPER=sccache`, `CARGO_INCREMENTAL=0`, and derived
  `SCCACHE_BASEDIRS`.
- Sandbox Cargo-backed checks require the matching offline Cargo home, set
  `CARGO_HOME`, force `--offline`, and set `CARGO_NET_OFFLINE=true`.
- Cargo-deny keeps ADR-0145: host `cargo deny check`; sandbox
  `cargo deny check bans licenses sources`.
- Remove separate crane `clippy`, `wasm-clippy`, and `deny` check derivations
  only with replacement wiring that keeps expanded hermetic `static-checks` on
  the required CI/validation path.
- Update `docs/ARCHITECTURE.md`, `CONTRIBUTING.md`, and a numberless ADR draft;
  promote the ADR only during ship.
- Commit cadence: tick the relevant checkbox, run
  `devtool run -- cargo xtask check`, stage the checked tree, then commit. No
  `Co-Authored-By` trailer.

## Review Header

**Scope in:**

- Add devtool command specs for `clippy`, `wasm-clippy`, `tools-clippy`, and
  keep/extend `cargo-deny`.
- Route host `xtask` static-check StepSpecs through `devtool check` for the
  project/tool compiling checks.
- Expand Nix `static-checks` to carry the hermetic compiling checks and remove
  replaced crane `clippy`, `wasm-clippy`, and `deny` check outputs.
- Ensure required validation/CI still exercises the expanded hermetic
  `static-checks` derivation.
- Update architecture/process docs and maintain the ADR draft/projection.

**Scope out:**

- #1106 host-native product test lane.
- RustSec advisory DB vendoring.
- Moving `xtask-fmt` or `xtask-clippy` into devtool.
- Changing Cargo workspace boundaries.

**Tasks:**

1. Add devtool compiling check specs and focused command-construction tests.
2. Route host xtask compiling checks through devtool while preserving
   cacheability.
3. Expand Nix `static-checks`, remove replaced crane check outputs, and wire the
   required validation path to the hermetic signal.
4. Update docs and ADR projection for the new boundary.
5. Run branch-level verification and prepare for ship.

**Key risks/decisions:**

- `cargo clippy --target wasm32-unknown-unknown` in sandbox mode must route
  through the product offline Cargo home and have the right toolchain target
  available.
- Removing crane outputs must not silently weaken required CI;
  `validate --no-e2e` must build or depend on the expanded `static-checks`.
- Host `sccache` env must reach the Cargo process spawned by `devtool`.
- `cargo-deny` sandbox policy must remain intentionally different from host
  policy.

---

## File Structure

- `tools/devtool/src/check.rs`: add compiling check specs, classify
  Rust-compiling checks if needed, and extend focused tests.
- `xtask/src/steps/static_checks.rs`: replace native project/tool compiling
  StepSpecs with devtool-backed StepSpecs, preserving `cache_rustc`.
- `xtask/src/lib.rs` and/or `xtask/src/steps/nix.rs`: wire required validation
  to build the expanded Nix `static-checks` derivation if it is not already on
  that path.
- `flake.nix`: add required tool inputs for expanded `static-checks`, remove
  replaced crane check outputs, and keep offline Cargo homes wired.
- `.github/workflows/ci.yml`: update only if the required CI entrypoint must
  change after the xtask wiring decision.
- `docs/ARCHITECTURE.md`: project the new static-check boundary and cite the ADR
  draft.
- `CONTRIBUTING.md`: update the Nix VM checks section to describe the expanded
  `static-checks` derivation and removed crane outputs.
- `docs/adr/drafts/devtool-owns-compiling-static-check-definitions.md`: keep the
  numberless ADR draft aligned with the implementation; promote during ship.

## Task 1: Add devtool compiling check specs

**Files:**

- Modify: `tools/devtool/src/check.rs`
- Test: `tools/devtool/src/check.rs`

**Interfaces:**

- Consumes: existing
  `CheckSpec::Cargo(CargoCheck { workspace, host_args, sandbox_args })`,
  `CargoWorkspace::{Product, Tools}`, and `CargoMode::{Host, Sandbox}`.
- Produces: `spec("clippy", false)`, `spec("wasm-clippy", false)`, and
  `spec("tools-clippy", false)` returning `CheckSpec::Cargo` commands usable by
  both host and sandbox modes.
- Produces: updated `ALL` order: `fmt`, `leptosfmt`, `prettier`, `tsc`,
  `elisp-fmt`, `ert`, `byte-compile`, `cargo-deny`, `clippy`, `wasm-clippy`,
  `tools-fmt`, `tools-clippy`.

- [x] **Step 1: Write failing devtool command-construction tests**

Add tests in `tools/devtool/src/check.rs`:

```rust
#[test]
fn clippy_matches_existing_host_ladder_args() {
    let cmd = build_host("clippy", false);

    assert_eq!(cmd.program, "cargo");
    assert_eq!(
        cmd.args,
        vec!["clippy", "--all-targets", "--", "-D", "warnings"]
    );
    assert!(cmd.env.is_empty());
    assert_eq!(build_host("clippy", true), cmd);
}

#[test]
fn wasm_clippy_matches_existing_host_ladder_args() {
    let cmd = build_host("wasm-clippy", false);

    assert_eq!(cmd.program, "cargo");
    assert_eq!(
        cmd.args,
        vec![
            "clippy",
            "-p",
            "web",
            "-p",
            "client",
            "-p",
            "csr",
            "--features",
            "csr",
            "--target",
            "wasm32-unknown-unknown",
            "--",
            "-D",
            "warnings",
        ]
    );
    assert!(cmd.env.is_empty());
    assert_eq!(build_host("wasm-clippy", true), cmd);
}

#[test]
fn tools_clippy_targets_tools_workspace() {
    let cmd = build_host("tools-clippy", false);

    assert_eq!(cmd.program, "cargo");
    assert_eq!(
        cmd.args,
        vec![
            "clippy",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ]
    );
    assert!(cmd.env.is_empty());
    assert_eq!(build_host("tools-clippy", true), cmd);
}

#[test]
fn sandbox_clippy_uses_product_offline_home() {
    let cmd = spec("clippy", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => Some("/nix/store/product-cargo-home".into()),
            "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
            _ => None,
        })
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args[0], "--offline");
    assert!(cmd.args.contains(&"clippy".to_string()));
    assert!(cmd.env.contains(&(
        "CARGO_HOME",
        OsString::from("/nix/store/product-cargo-home")
    )));
    assert!(
        cmd.env
            .contains(&("CARGO_NET_OFFLINE", OsString::from("true")))
    );
}

#[test]
fn sandbox_tools_clippy_uses_tools_offline_home() {
    let cmd = spec("tools-clippy", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => Some("/nix/store/product-cargo-home".into()),
            "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
            _ => None,
        })
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args[0], "--offline");
    assert!(
        cmd.args
            .windows(2)
            .any(|w| w == ["--manifest-path", "tools/Cargo.toml"])
    );
    assert!(
        cmd.env
            .contains(&("CARGO_HOME", OsString::from("/nix/store/tools-cargo-home")))
    );
}
```

- [x] **Step 2: Run the tests and verify they fail**

Run:

```bash
devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture
```

Expected: FAIL because `clippy`, `wasm-clippy`, and `tools-clippy` are unknown
devtool checks.

- [x] **Step 3: Implement the devtool specs**

Update `ALL` and `spec()` in `tools/devtool/src/check.rs`:

- `clippy`: product workspace, same host/sandbox args
  `["clippy", "--all-targets", "--", "-D", "warnings"]`.
- `wasm-clippy`: product workspace, same host/sandbox args
  `["clippy", "-p", "web", "-p", "client", "-p", "csr", "--features", "csr", "--target", "wasm32-unknown-unknown", "--", "-D", "warnings"]`.
- `tools-clippy`: tools workspace, same host/sandbox args
  `["clippy", "--all-targets", "--", "-D", "warnings"]`; rely on
  `CargoWorkspace::Tools::cargo_args` to inject
  `--manifest-path tools/Cargo.toml`.

Keep `cargo-deny` unchanged except for its placement in the expanded `ALL`.

- [x] **Step 4: Run focused tests and verify pass**

Run:

```bash
devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture
```

Expected: PASS.

- [x] **Step 5: Commit Task 1**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
git add tools/devtool/src/check.rs docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
git commit -m "feat(devtool): define compiling static checks (#276)"
```

## Task 2: Route host xtask checks through devtool

**Files:**

- Modify: `xtask/src/steps/static_checks.rs`
- Test: `xtask/src/steps/static_checks.rs`

**Interfaces:**

- Consumes: Task 1 devtool check names.
- Produces: host `StepSpec`s for `cargo-deny`, `clippy`, `wasm-clippy`, and
  `tools-clippy` whose `program` is `cargo` and whose args invoke
  `cargo run --quiet --manifest-path tools/Cargo.toml -p devtool -- check <name>`.
- Produces: `cache_rustc = true` for `clippy`, `wasm-clippy`, and
  `tools-clippy`; `cache_rustc = false` for `cargo-deny`.

- [x] **Step 1: Replace native-check tests with devtool-routing tests**

In `xtask/src/steps/static_checks.rs`, replace or rewrite
`native_checks_stay_native` so it asserts:

```rust
#[test]
fn compiling_project_checks_delegate_to_devtool_with_cacheability() {
    let s = specs(Mode::Check);

    for name in ["cargo-deny", "clippy", "wasm-clippy", "tools-clippy"] {
        let step = find(&s, name);
        assert_eq!(step.program, "cargo");
        assert_eq!(
            step.args,
            [
                "run",
                "--quiet",
                "--manifest-path",
                "tools/Cargo.toml",
                "-p",
                "devtool",
                "--",
                "check",
                name,
            ]
        );
    }

    assert!(!find(&s, "cargo-deny").cache_rustc);
    assert!(find(&s, "clippy").cache_rustc);
    assert!(find(&s, "wasm-clippy").cache_rustc);
    assert!(find(&s, "tools-clippy").cache_rustc);

    assert_eq!(find(&s, "xtask-clippy").program, "cargo");
    assert!(find(&s, "xtask-clippy").cache_rustc);
}
```

Keep existing tests for `xtask-fmt`, `xtask-clippy`, compile-cache env, and step
ordering, but update any comments/counts that still describe compiling checks as
native.

- [x] **Step 2: Run tests and verify fail**

Run:

```bash
devtool run -- cargo test --manifest-path xtask/Cargo.toml static_checks::tests -- --nocapture
```

Expected: FAIL because the four checks still use native Cargo StepSpecs.

- [x] **Step 3: Implement routing**

Refactor `devtool_check` if needed so it can set `cache_rustc`. One acceptable
shape:

```rust
fn devtool_check_with_cache(name: &'static str, mode: Mode, cache_rustc: bool) -> StepSpec
```

Use it for:

- existing non-compiling checks with `cache_rustc = false`;
- `cargo-deny` with `cache_rustc = false`;
- `clippy`, `wasm-clippy`, and `tools-clippy` with `cache_rustc = true`.

Delete the native StepSpecs for product `clippy`, `wasm-clippy`, `cargo-deny`,
and `tools-clippy`. Leave `xtask-fmt` and `xtask-clippy` native.

- [x] **Step 4: Run focused tests and verify pass**

Run:

```bash
devtool run -- cargo test --manifest-path xtask/Cargo.toml static_checks::tests -- --nocapture
```

Expected: PASS.

- [x] **Step 5: Verify host delegation reaches real tools**

Run at least:

```bash
devtool run -- cargo xtask check --no-test
```

Expected: PASS. If the environment blocks `cargo-deny` advisory access or
sccache, rerun with escalation and record the reason in the implementation log.

- [x] **Step 6: Commit Task 2**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
git add xtask/src/steps/static_checks.rs docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
git commit -m "refactor(xtask): route compiling checks through devtool (#276)"
```

## Task 3: Replace flake crane check outputs with expanded static-checks

**Files:**

- Modify: `flake.nix`
- Modify: `xtask/src/lib.rs` and/or `xtask/src/steps/nix.rs` if required
- Test: existing xtask static/nix step tests if the Nix step list changes

**Interfaces:**

- Consumes: Task 1 expanded `devtool check --all --sandbox-cargo`.
- Produces: Nix `checks.x86_64-linux.static-checks` as the hermetic signal for
  `clippy`, `wasm-clippy`, `cargo-deny`, and `tools-clippy`.
- Produces: required `cargo xtask validate --no-e2e` path that builds or depends
  on the expanded Nix `static-checks` derivation after the crane outputs are
  removed.

- [x] **Step 1: Inspect current required Nix step wiring**

Read the Nix step list and tests:

```bash
rg -n "static-checks|clippy|wasm-clippy|deny|validate" xtask/src flake.nix .github/workflows/ci.yml
```

Record which `cargo xtask validate --no-e2e` step currently realizes the
hermetic static checks. If no step realizes `checks.*.static-checks`, add one in
this task.

- [x] **Step 2: Update expected Nix surface tests**

If xtask has tests that lock Nix step names, update them so the required
validation path includes the expanded `static-checks` derivation and no longer
expects separate `clippy`, `wasm-clippy`, or `deny` Nix checks.

Run the targeted tests. Use the specific test names found in Step 1; likely
shape:

```bash
devtool run -- cargo test --manifest-path xtask/Cargo.toml nix -- --nocapture
```

Expected before implementation: FAIL if tests are updated first.

- [x] **Step 3: Update `flake.nix`**

In `checks`:

- Remove separate `clippy = craneLib.cargoClippy (...)`.
- Remove separate `wasm-clippy = craneLib.cargoClippy (...)`.
- Remove `deny = craneLib.cargoDeny { ... }`.
- Expand the `static-checks` comment: it is no longer non-compiling only.
- Ensure `static-checks.nativeBuildInputs` includes every tool needed by
  expanded `devtool check --all --sandbox-cargo`:
  - `devtoolBin`
  - `toolchain`
  - `pkgs.cargo-deny`
  - `leptosfmt`
  - `pkgs.prettier`
  - `pkgs.nodejs`
  - `pkgs.typescript`
  - `emacsForCi`
- Keep both `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` and
  `JAUNDER_DEVTOOL_TOOLS_CARGO_HOME`.

If wasm clippy requires a target/tool input not already available through
`toolchain`, add the exact pinned input the existing flake uses rather than
fetching through a wrapper.

- [x] **Step 4: Wire required validation to hermetic static-checks**

If `cargo xtask validate --no-e2e` does not already build `static-checks`, add
or adjust the Nix step so it does. The resulting required CI job
(`Validate (no e2e)`) must fail if the expanded Nix `static-checks` derivation
fails.

Do not make CI call `nix flake check` directly unless the xtask gate model
requires it; prefer preserving the current `cargo xtask validate --no-e2e`
entrypoint and changing what it realizes.

- [x] **Step 5: Run focused local proofs**

Run:

```bash
devtool run -- nix build .#checks.x86_64-linux.static-checks -L
```

Expected: PASS; this proves the expanded hermetic check derivation works.

Run:

```bash
devtool run -- cargo xtask validate --no-e2e
```

Expected: PASS; this proves the required CI entrypoint exercises the hermetic
static-check signal.

Implementation note: the direct `static-checks` build passed. Local
`validate --no-e2e --allow-dirty` reached and passed `nix-static-checks`, then
failed on ignored local `.agents/` prettier inputs that are not part of a clean
CI checkout.

- [x] **Step 6: Commit Task 3**

Tick this task checkbox, then run:

```bash
devtool run -- cargo xtask check
git add flake.nix xtask/src docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
git commit -m "build(nix): route compiling checks through static-checks (#276)"
```

Only include `xtask/src` in the `git add` if Step 4 changed it.

## Task 4: Update docs and ADR projection

**Files:**

- Modify: `docs/ARCHITECTURE.md`
- Modify: `CONTRIBUTING.md`
- Modify: `docs/adr/drafts/devtool-owns-compiling-static-check-definitions.md`
- Modify:
  `docs/superpowers/specs/2026-08-21-issue-276-compiling-static-check-definitions.md`
  only if implementation discovers a spec correction.
- Test: Markdown formatting and doc-link gates through `cargo xtask check`.

**Interfaces:**

- Consumes: final implementation decisions from Tasks 1-3.
- Produces: docs that describe host-vs-sandbox lanes, expanded `static-checks`,
  removed crane outputs, and native `xtask` self-lints.

- [x] **Step 1: Reconcile architecture committed-direction prose**

Update the two `docs/ARCHITECTURE.md` areas already touched during the spec
step:

- `Verification gates` / `What the ladder actually runs`.
- `Development tooling` / `devtool check`.

After implementation, remove any stale "Committed direction" language that is
now current reality. Keep the draft ADR citation path:

```markdown
(...[devtool owns compiling static-check definitions across host and Nix](adr/drafts/devtool-owns-compiling-static-check-definitions.md))
```

- [x] **Step 2: Update `CONTRIBUTING.md` Nix VM checks**

In the `### Nix VM checks` list, replace the old bullets:

- separate `checks.x86_64-linux.clippy` / `.wasm-clippy`;
- `static-checks` as eight non-compiling checks only;
- separate `checks.x86_64-linux.deny`.

With prose that says `checks.x86_64-linux.static-checks` runs the shared
`devtool check --all --sandbox-cargo` surface for formatting, TypeScript/elisp,
product/wasm/tools clippy, and cargo-deny's sandbox-safe policy.

- [x] **Step 3: Reconcile ADR draft**

Read:

```bash
sed -n '1,220p' docs/adr/drafts/devtool-owns-compiling-static-check-definitions.md
```

Ensure the draft matches the actual Task 3 wiring. Keep line 1
`# ADR-DRAFT: ...`, status `proposed`, and issue `#276`.

- [x] **Step 4: Format and run doc-focused checks through the normal gate**

Run:

```bash
devtool run -- prettier -w docs/ARCHITECTURE.md CONTRIBUTING.md docs/adr/drafts/devtool-owns-compiling-static-check-definitions.md docs/superpowers/specs/2026-08-21-issue-276-compiling-static-check-definitions.md docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
```

Then run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS, including `adr-format`, `adr-view-parity`, and `doc-links` after
any required implementation state. The ADR draft remains numberless and ignored
until ship promotion.

- [x] **Step 5: Commit Task 4**

Tick this task checkbox, then run:

```bash
git add docs/ARCHITECTURE.md CONTRIBUTING.md docs/superpowers/specs/2026-08-21-issue-276-compiling-static-check-definitions.md docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
git commit -m "docs: record compiling static-check boundary (#276)"
```

Do not add the ADR draft; it is intentionally gitignored until ship promotion.

## Task 5: Branch verification and handoff to ship

**Files:**

- Modify:
  `docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md`
- Test: whole branch

**Interfaces:**

- Consumes: all prior task commits.
- Produces: a clean branch ready for `jaunder-ship`.

- [ ] **Step 1: Run final focused proofs**

Run:

```bash
devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture
devtool run -- cargo test --manifest-path xtask/Cargo.toml static_checks::tests -- --nocapture
devtool run -- nix build .#checks.x86_64-linux.static-checks -L
devtool run -- cargo xtask validate --no-e2e
```

Expected: PASS for all.

- [ ] **Step 2: Run the branch gate**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

- [ ] **Step 3: Inspect branch diff for stale policy language**

Run:

```bash
rg "non-compiling|crane.*clippy|cargoDeny|cargo-deny.*StepSpec|native.*tools-clippy" docs CONTRIBUTING.md flake.nix xtask/src tools/devtool/src
```

Expected: remaining hits are either historical ADR/archive text, accurately
qualified old context, or current implementation references. Fix stale current
docs/comments before proceeding.

- [ ] **Step 4: Commit final plan state if needed**

If only the plan checkbox state changed since the previous commit, run the
normal gate and commit:

```bash
devtool run -- cargo xtask check
git add docs/superpowers/plans/2026-08-21-issue-276-compiling-static-check-definitions.md
git commit -m "docs: finish compiling static-check plan (#276)"
```

- [ ] **Step 5: Handoff to ship**

Confirm:

```bash
git status --short
git log --oneline origin/main..HEAD
```

Expected: clean tree, focused commits, ignored ADR draft still present under
`docs/adr/drafts/`. Continue with `jaunder-ship`, which will archive the
spec/plan, rebase, promote the ADR draft, run full `cargo xtask validate`, push,
open the PR, watch CI, and halt before merge.
