# #1074 Offline Cargo-Deny Policy Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `cargo-deny` to `devtool check` with an explicit host/sandbox
policy: full advisory checking on the host, offline-safe checks in the sandbox.

**Architecture:** Build on #1073's existing workspace-aware Cargo command model.
Represent `cargo-deny` as a product-workspace Cargo check whose arguments differ
by `CargoMode`: host mode runs `cargo deny check`; sandbox mode forces offline
Cargo routing and runs only `bans`, `licenses`, and `sources`. Keep the `xtask`
native `cargo-deny` StepSpec and crane `deny` derivation unchanged in this
issue; #276 owns their later unification.

**Tech Stack:** Rust 2024, `anyhow`, `clap`, `cargo-deny`, Nix flakes/crane,
`devtool run`, `cargo xtask check`.

## Global Constraints

- Follow the approved spec:
  `docs/superpowers/specs/2026-08-21-issue-1074-offline-cargo-deny-policy.md`.
- Preserve ADR-0028: `devtool` is allowed in the sandbox and as a host runner;
  `xtask` remains host-only.
- Preserve ADR-0052's current boundary until #276: do not rewire the host
  `xtask` `cargo-deny` StepSpec in this issue.
- Preserve ADR-0141: the root product workspace and `tools/` workspace remain
  separate.
- Sandboxed cargo-deny must skip `advisories` and check only `bans`, `licenses`,
  and `sources`.
- Do not vendor or otherwise provide the RustSec advisory database in this
  issue.
- Do not remove or change the crane `deny` derivation.
- No `Co-Authored-By` trailer.

## Review Header

**Scope in:** `tools/devtool/src/check.rs` command modeling and tests; proof
that the existing Nix `static-checks` derivation exercises `cargo-deny` through
`devtool check --all --sandbox-cargo`; the approved spec, architecture
projection, and numberless ADR draft.

**Scope out:** `xtask/src/steps/static_checks.rs` rewiring; crane `deny`
derivation changes; RustSec advisory DB vendoring;
clippy/wasm-clippy/tools-clippy migration; Cargo workspace membership changes.

**Task list:**

1. Add `cargo-deny` to `devtool check` with host and sandbox command contracts.
2. Add `cargo-deny` to the Nix `static-checks` environment and prove
   `devtool check --all --sandbox-cargo` runs it there.
3. Gate and commit the implementation plus approved lifecycle docs.

**Key risks/decisions:**

- The sandbox argument list must make advisory execution structurally absent; do
  not rely on `--disable-fetch` or network failure.
- `build_selected_commands_with_env` already validates all selected Cargo homes
  before spawning; keep `cargo-deny` inside that path.
- The ADR draft is gitignored by design. Keep the draft file present locally for
  ship promotion, but do not force-add it before `cargo xtask adr promote`.

---

## File Structure

- `tools/devtool/src/check.rs`: add `cargo-deny` to `ALL`, model mode-specific
  Cargo arguments, and test host/sandbox command shape.
- `flake.nix`: preserve crane's generated vendored-source Cargo config in
  `mkOfflineCargoHome`, and add `pkgs.cargo-deny` to the `static-checks`
  `nativeBuildInputs`. Existing `static-checks` already exports
  `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` and runs
  `devtool check --all --sandbox-cargo`; use the derivation as the sandbox proof
  after the tool is on PATH.
- `xtask/src/steps/static_checks.rs`: no intended edit. Tests/assertions should
  continue proving the native `cargo-deny` StepSpec is unchanged.
- `docs/superpowers/specs/2026-08-21-issue-1074-offline-cargo-deny-policy.md`:
  approved spec, staged with the implementation.
- `docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md`:
  this plan, staged with the implementation.
- `docs/ARCHITECTURE.md`: committed-direction projection for the ADR draft,
  staged with the implementation.
- `docs/adr/0145-sandbox-cargo-deny-skips-advisories.md`: numberless ADR draft
  kept in the gitignored drafts pen until ship.

---

### Task 1: Add cargo-deny to devtool check

**Files:**

- Modify: `tools/devtool/src/check.rs`

**Interfaces:**

- Consumes:
  - `enum CargoWorkspace { Product, Tools }`
  - `enum CargoMode { Host, Sandbox }`
  - `struct CargoCheck { workspace: CargoWorkspace, args: Vec<String> }`
  - `enum CheckSpec`
  - `fn build_selected_commands_with_env<F>(...) -> anyhow::Result<Vec<BuiltCommand>>`
- Produces:
  - `ALL` includes `"cargo-deny"` after `"byte-compile"` and before
    `"tools-fmt"`.
  - `spec("cargo-deny", false)` and `spec("cargo-deny", true)` both return a
    product-workspace Cargo check.
  - Host built command: `program = "cargo"`, `args = ["deny", "check"]`,
    `env = []`.
  - Sandbox built command: `program = "cargo"`,
    `args = ["--offline", "deny", "check", "bans", "licenses", "sources"]`, env
    contains `CARGO_HOME = $JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` and
    `CARGO_NET_OFFLINE = true`.

- [x] **Step 1: Write failing tests for cargo-deny command shape**

Add these tests to `tools/devtool/src/check.rs`'s existing `tests` module:

```rust
#[test]
fn cargo_deny_uses_full_host_policy() {
    let cmd = build_host("cargo-deny", false);

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args, vec!["deny", "check"]);
    assert!(cmd.env.is_empty());
    assert_eq!(build_host("cargo-deny", true), cmd);
}

#[test]
fn sandbox_cargo_deny_skips_advisories_and_uses_product_home() {
    let cmd = spec("cargo-deny", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => {
                Some("/nix/store/product-cargo-home".into())
            }
            "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
            _ => None,
        })
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(
        cmd.args,
        vec!["--offline", "deny", "check", "bans", "licenses", "sources"]
    );
    assert!(!cmd.args.iter().any(|arg| arg == "advisories"));
    assert!(cmd.env.contains(&(
        "CARGO_HOME",
        std::ffi::OsString::from("/nix/store/product-cargo-home")
    )));
    assert!(
        cmd.env
            .contains(&("CARGO_NET_OFFLINE", std::ffi::OsString::from("true")))
    );
}

#[test]
fn sandbox_cargo_deny_requires_product_home() {
    let err = spec("cargo-deny", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |_| None)
        .unwrap_err()
        .to_string();

    assert!(err.contains("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME"), "{err}");
}
```

Run:

```bash
devtool run -- cargo test --manifest-path tools/Cargo.toml cargo_deny -- --nocapture
```

Expected: FAIL. The output should show `unknown check 'cargo-deny'` or missing
test behavior because `ALL`/`spec` do not yet include the check.

- [x] **Step 2: Implement cargo-deny in the command model**

Edit `tools/devtool/src/check.rs`:

- Add `"cargo-deny"` to `ALL` after `"byte-compile"` and before `"tools-fmt"`.
- The current `spec(name, fix)` does not receive `CargoMode`; do not thread mode
  through `spec`. Instead, extend the command model minimally so mode-specific
  arguments live where commands are built. One acceptable shape is:

```rust
struct CargoCheck {
    workspace: CargoWorkspace,
    host_args: Vec<String>,
    sandbox_args: Vec<String>,
}
```

Then update existing `fmt` and `tools-fmt` construction so `host_args` and
`sandbox_args` are identical for those checks, while `cargo-deny` differs:

```rust
"cargo-deny" => CheckSpec::Cargo(CargoCheck {
    workspace: CargoWorkspace::Product,
    host_args: owned(&["deny", "check"]),
    sandbox_args: owned(&["deny", "check", "bans", "licenses", "sources"]),
})
```

Keep the manifest-routing invariant: no check may supply `--manifest-path`
inside either arg vector; `CargoWorkspace` owns that.

- [x] **Step 3: Update existing tests for the new `CargoCheck` shape**

Adjust any existing tests that directly construct `CargoCheck` so they provide
both host and sandbox arg vectors. Preserve these existing assertions:

- `fmt_uses_product_workspace_cargo_in_host_mode`
- `tools_fmt_uses_tools_workspace_manifest_in_host_mode`
- `sandbox_product_cargo_forces_offline_and_uses_product_home`
- `workspace_selection_owns_manifest_routing`
- `sandbox_tools_cargo_forces_offline_and_uses_tools_home`
- `sandbox_all_checks_validates_every_cargo_home_before_running`
- `all_names_have_specs`

- [x] **Step 4: Run targeted devtool tests**

Run:

```bash
devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture
```

Expected: PASS.

- [x] **Step 5: Verify xtask native cargo-deny remains unchanged**

Run:

```bash
devtool run -- cargo test --manifest-path xtask/Cargo.toml static_checks::tests::native_checks_stay_native -- --nocapture
```

Expected: PASS. The test must still assert `["deny", "check"]` for the native
`xtask` StepSpec.

- [x] **Step 6: Commit Task 1**

Tick completed checkboxes for Task 1, then run:

```bash
devtool run -- cargo xtask check --no-test
```

Expected: PASS. Inspect any formatter changes.

Stage only Task 1 implementation and this plan checkbox update:

```bash
git add tools/devtool/src/check.rs docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md
git commit -m "feat(devtool): add offline cargo-deny check policy (#1074)"
```

---

### Task 2: Add cargo-deny to Nix static-checks and prove the sandbox path

**Files:**

- Modify: `flake.nix:306-319,1334-1342`
- Test/proof: existing `checks.x86_64-linux.static-checks`

**Interfaces:**

- Consumes:
  - `devtool check --all --sandbox-cargo`
  - `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME = "${appOfflineCargoHome}"`
  - `JAUNDER_DEVTOOL_TOOLS_CARGO_HOME = "${toolsOfflineCargoHome}"`
- Produces:
  - `mkOfflineCargoHome` copies crane's generated `vendorDir/config.toml` so git
    patch source replacements remain available offline, then appends
    `[net] offline = true`.
  - `pkgs.cargo-deny` is in the `static-checks` `nativeBuildInputs`.
  - A passing Nix `static-checks` derivation that now includes
    `cargo-deny bans licenses sources` through `devtool`.

- [x] **Step 1: Write the flake input change**

In `flake.nix`, preserve the generated vendor config in `mkOfflineCargoHome`:

```nix
cp ${vendorDir}/config.toml $out/config.toml
chmod u+w $out/config.toml
cat >> $out/config.toml <<EOF

[net]
offline = true
EOF
```

Then add `pkgs.cargo-deny` to the `static-checks` `nativeBuildInputs` list after
`toolchain`:

```nix
nativeBuildInputs = [
  devtoolBin
  toolchain
  pkgs.cargo-deny
  leptosfmt
  pkgs.prettier
  pkgs.nodejs
  pkgs.typescript
  emacsForCi
];
```

Do not edit the crane `deny` derivation.

- [x] **Step 2: Inspect the static-checks derivation**

Run:

```bash
sed -n '1318,1362p' flake.nix
```

Expected: The derivation includes `pkgs.cargo-deny`, exports both
`JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` and `JAUNDER_DEVTOOL_TOOLS_CARGO_HOME`, and
runs `devtool check --all --sandbox-cargo`.

- [x] **Step 3: Build the sandbox proof**

Run:

```bash
devtool run -- nix build .#checks.x86_64-linux.static-checks -L
```

Expected: PASS. The derivation must finish without network access and without
running `advisories`.

- [x] **Step 4: If the proof fails, inspect the parked log**

Read the `.xtask/run/<id>.err` and `.xtask/run/<id>.out` paths from Step 2's
JSON result. Search the parked log for these strings in separate commands:

```bash
rg "cargo-deny|advisories|bans|licenses|sources|JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" .xtask/run/<id>.err
rg "cargo-deny|advisories|bans|licenses|sources|JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" .xtask/run/<id>.out
```

Expected on failure: enough detail to fix command shape or missing sandbox env.
Do not pipe the `nix build` command through a filter.

- [x] **Step 5: Commit Task 2**

Tick completed checkboxes for Task 2, then run:

```bash
devtool run -- cargo xtask check --no-test
```

Expected: PASS.

Then stage and commit only the Task 2 changes:

```bash
git add flake.nix docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md
git commit -m "build(nix): exercise sandbox cargo-deny through devtool (#1074)"
```

---

### Task 3: Gate and commit lifecycle docs

**Files:**

- Add:
  `docs/superpowers/specs/2026-08-21-issue-1074-offline-cargo-deny-policy.md`
- Add:
  `docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md`
- Modify: `docs/ARCHITECTURE.md`
- Keep local only until ship:
  `docs/adr/0145-sandbox-cargo-deny-skips-advisories.md`

**Interfaces:**

- Consumes: approved spec, approved plan, Task 1 implementation, Task 2 sandbox
  proof.
- Produces: one checked docs commit containing tracked lifecycle docs and
  architecture projection. The ADR draft remains in the gitignored drafts pen
  until `jaunder-ship` runs `cargo xtask adr promote`.

- [x] **Step 1: Format docs**

Run:

```bash
devtool run -- prettier -w docs/superpowers/specs/2026-08-21-issue-1074-offline-cargo-deny-policy.md docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md docs/ARCHITECTURE.md docs/adr/0145-sandbox-cargo-deny-skips-advisories.md
```

Expected: PASS.

- [x] **Step 2: Run the full check gate**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS.

- [x] **Step 3: Inspect status**

Run:

```bash
git status --short
```

Expected tracked changes are limited to implementation files from earlier tasks,
`docs/ARCHITECTURE.md`, the spec, and this plan. The ADR draft should not appear
unless someone force-added ignored files; if it appears as staged, unstage it
and keep it local for promotion.

- [x] **Step 4: Commit lifecycle docs**

Tick completed checkboxes for Task 3, then stage:

```bash
git add docs/ARCHITECTURE.md docs/superpowers/specs/2026-08-21-issue-1074-offline-cargo-deny-policy.md docs/superpowers/plans/2026-08-21-issue-1074-offline-cargo-deny-policy.md
git commit -m "docs: record cargo-deny sandbox policy (#1074)"
```

Expected: commit succeeds with the pre-commit hook. No `Co-Authored-By` trailer.

## Self-Review

**Spec coverage:** AC1-AC5 are covered by Task 1 tests and Task 2 Nix proof. AC6
is covered by Task 1 Step 5. AC7 is covered by Task 3. AC8 is covered by Task 1
targeted tests, Task 2 Nix proof, and Task 3 full gate.

**Placeholder scan:** No task contains TODO/TBD placeholders; every run command
has an expected result.

**Type consistency:** `CargoWorkspace`, `CargoMode`, `CargoCheck`, `CheckSpec`,
`BuiltCommand`, `ALL`, `spec`, and `build_selected_commands_with_env` match the
current `tools/devtool/src/check.rs` interfaces from #1073.
