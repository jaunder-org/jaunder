# #1073 Devtool Offline Cargo Config Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

## Review Header

**Goal:** Give `devtool check` a typed Cargo execution path that can select the
product workspace or the `tools/` workspace and force sandboxed Cargo
invocations through the matching offline Cargo config.

**Scope:** In: `tools/devtool/src/check.rs` command modeling,
`tools/devtool/src/main.rs` check argument plumbing, and `flake.nix`
static-check sandbox wiring. Out: moving `clippy`, `wasm-clippy`, `cargo-deny`,
or `tools-clippy` behind `devtool check`; changing crane check behavior;
changing workspace membership; adding an ADR.

**Task list:**

1. Add workspace-aware Cargo command construction in `devtool check` and convert
   the existing Cargo-backed `fmt`/`tools-fmt` checks to use it.
2. Add product/tools offline Cargo-home derivations in `flake.nix` and run Nix
   `static-checks` with `devtool check --all --sandbox-cargo`.
3. Gate and commit the checked implementation.

**Key risks/decisions:**

- Cargo offline enforcement is two-part: command args include `--offline`, and
  the environment includes `CARGO_NET_OFFLINE=true` plus a workspace-specific
  `CARGO_HOME`.
- Tests must not mutate process env. Use an injected env lookup closure for
  command construction tests.
- The current migrated-check order and host command output remain unchanged
  except for the new opt-in `--sandbox-cargo` flag.

**Architecture:** Keep the public check surface boring:
`devtool check <name>|--all [--fix] [--sandbox-cargo]`. Internally, represent
command specs as either external commands or Cargo commands bound to a
`CargoWorkspace`. Only Cargo commands consult workspace-specific offline config.

**Tech Stack:** Rust 2024, `anyhow`, `clap`, Nix flakes/crane, `devtool run`,
`cargo xtask check`.

## Global Constraints

- Preserve ADR-0028: `devtool` is in-sandbox producer/runner code; `xtask`
  remains host analyzer/orchestrator.
- Preserve ADR-0141: root product workspace, `tools/`, and `xtask/` remain
  separate workspaces.
- Do not move `clippy`, `wasm-clippy`, `cargo-deny`, or `tools-clippy` behind
  `devtool check` in this issue.
- Sandboxed Cargo invocations must force offline behavior with `--offline`
  and/or `CARGO_NET_OFFLINE=true` and must use workspace-specific config.
- Host behavior remains unchanged unless `--sandbox-cargo` is explicitly
  selected.
- No `Co-Authored-By` trailer.

---

## File Structure

- `tools/devtool/src/main.rs`: add `CheckArgs::sandbox_cargo` and pass it to
  `check::run`.
- `tools/devtool/src/check.rs`: add command model types, workspace-specific
  Cargo config selection, pure tests, and use the model for `fmt` and
  `tools-fmt`.
- `flake.nix`: add product/tools offline Cargo-home derivations and pass their
  paths to `static-checks` while invoking `devtool check --all --sandbox-cargo`.
- `docs/superpowers/specs/2026-08-20-issue-1073-devtool-offline-cargo-config.md`:
  approved spec, staged with the implementation.
- `docs/superpowers/plans/2026-08-20-issue-1073-devtool-offline-cargo-config.md`:
  this plan, staged with the implementation.

---

### Task 1: Workspace-aware Cargo commands in devtool

**Files:**

- Modify: `tools/devtool/src/main.rs:54-64,150-158`
- Modify: `tools/devtool/src/check.rs:1-228`

**Interfaces:**

- Produces:
  - `enum CargoWorkspace { Product, Tools }`
  - `enum CargoMode { Host, Sandbox }`
  - `struct CargoCheck { workspace: CargoWorkspace, args: Vec<String> }`
  - `enum CheckSpec { External { program: &'static str, args: Vec<String> }, Cargo(CargoCheck) }`
  - `struct BuiltCommand { program: &'static str, args: Vec<String>, env: Vec<(&'static str, std::ffi::OsString)> }`
  - `impl CheckSpec { fn build_with_env<F>(&self, cargo_mode: CargoMode, env_lookup: F) -> anyhow::Result<BuiltCommand> where F: Fn(&'static str) -> Option<std::ffi::OsString> }`
  - `fn build_selected_commands_with_env<F>(names: &[&str], fix: bool, cargo_mode: CargoMode, env_lookup: F) -> anyhow::Result<Vec<BuiltCommand>> where F: Fn(&'static str) -> Option<std::ffi::OsString> + Copy`
  - `pub fn run(name: Option<&str>, all: bool, fix: bool, sandbox_cargo: bool) -> anyhow::Result<()>`
- Consumes: existing `spec(name, fix)` mapping and `needs_provisioning(name)`
  behavior.

- [x] **Step 1: Add failing tests for host Cargo command shape**

Add tests in `tools/devtool/src/check.rs`:

```rust
#[test]
fn fmt_uses_product_workspace_cargo_in_host_mode() {
    let cmd = spec("fmt", false)
        .unwrap()
        .build_with_env(CargoMode::Host, |_| None)
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args, vec!["fmt", "--check"]);
    assert!(cmd.env.is_empty());
}

#[test]
fn tools_fmt_uses_tools_workspace_manifest_in_host_mode() {
    let cmd = spec("tools-fmt", false)
        .unwrap()
        .build_with_env(CargoMode::Host, |_| None)
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(
        cmd.args,
        vec![
            "fmt",
            "--manifest-path",
            "tools/Cargo.toml",
            "--all",
            "--check",
        ]
    );
    assert!(cmd.env.is_empty());
}
```

Run:
`devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests:: -- --nocapture`

Expected: FAIL because `CheckSpec`, `CargoMode`, `build_with_env`, and the new
return type do not exist yet.

- [x] **Step 2: Add failing tests for sandbox offline config**

Add tests in `tools/devtool/src/check.rs`:

```rust
#[test]
fn sandbox_product_cargo_forces_offline_and_uses_product_home() {
    let cmd = spec("fmt", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => Some("/nix/store/product-cargo-home".into()),
            "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
            _ => None,
        })
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args, vec!["--offline", "fmt", "--check"]);
    assert!(cmd.env.contains(&(
        "CARGO_HOME",
        std::ffi::OsString::from("/nix/store/product-cargo-home")
    )));
    assert!(cmd.env.contains(&(
        "CARGO_NET_OFFLINE",
        std::ffi::OsString::from("true")
    )));
}

#[test]
fn sandbox_tools_cargo_forces_offline_and_uses_tools_home() {
    let cmd = spec("tools-fmt", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => Some("/nix/store/product-cargo-home".into()),
            "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME" => Some("/nix/store/tools-cargo-home".into()),
            _ => None,
        })
        .unwrap();

    assert_eq!(cmd.program, "cargo");
    assert_eq!(cmd.args[0], "--offline");
    assert!(cmd.args.windows(2).any(|w| w == ["--manifest-path", "tools/Cargo.toml"]));
    assert!(cmd.env.contains(&(
        "CARGO_HOME",
        std::ffi::OsString::from("/nix/store/tools-cargo-home")
    )));
}

#[test]
fn sandbox_cargo_errors_before_spawn_when_workspace_home_is_missing() {
    let err = spec("tools-fmt", false)
        .unwrap()
        .build_with_env(CargoMode::Sandbox, |_| None)
        .unwrap_err()
        .to_string();

    assert!(err.contains("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME"), "{err}");
}

#[test]
fn sandbox_all_checks_validates_every_cargo_home_before_running() {
    let err = build_selected_commands_with_env(
        &["fmt", "tools-fmt"],
        false,
        CargoMode::Sandbox,
        |name| match name {
            "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME" => Some("/nix/store/product-cargo-home".into()),
            _ => None,
        },
    )
    .unwrap_err()
    .to_string();

    assert!(
        err.contains("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME"),
        "{err}"
    );
}
```

Run:
`devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests:: -- --nocapture`

Expected: FAIL for missing command model and sandbox config behavior.

- [x] **Step 3: Implement the command model and CLI plumbing**

Change `tools/devtool/src/main.rs`:

```rust
#[derive(clap::Args)]
struct CheckArgs {
    /// Which check to run (omit and pass `--all` to run every check).
    name: Option<String>,
    /// Run all the non-compiling static checks.
    #[arg(long, conflicts_with = "name")]
    all: bool,
    /// Auto-fix (the formatters) instead of verifying.
    #[arg(long)]
    fix: bool,
    /// Run Cargo-backed checks with workspace-specific offline Cargo config.
    #[arg(long)]
    sandbox_cargo: bool,
}
```

and pass `args.sandbox_cargo` into `check::run`.

Change `tools/devtool/src/check.rs` so `spec(name, fix)` returns `CheckSpec`.
Convert:

- `fmt` to
  `CheckSpec::Cargo(CargoCheck { workspace: CargoWorkspace::Product, args: ... })`
- `tools-fmt` to
  `CheckSpec::Cargo(CargoCheck { workspace: CargoWorkspace::Tools, args: ... })`
- all other existing checks to `CheckSpec::External { ... }`

`CheckSpec::build_with_env` rules:

- External commands ignore `cargo_mode` and return unchanged program/args/env.
- Cargo + `CargoMode::Host`: return `program = "cargo"`, original args, no env.
- Cargo + `CargoMode::Sandbox`:
  - Choose env var by workspace:
    - Product: `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME`
    - Tools: `JAUNDER_DEVTOOL_TOOLS_CARGO_HOME`
  - If the selected env var is absent or empty, return an error naming the env
    var and workspace.
  - Return `program = "cargo"`, args with `"--offline"` inserted before the
    cargo subcommand, env containing `("CARGO_HOME", selected_home)` and
    `("CARGO_NET_OFFLINE", "true")`.

In `run`, compute
`let cargo_mode = if sandbox_cargo { CargoMode::Sandbox } else { CargoMode::Host };`,
call
`build_selected_commands_with_env(&names, fix, cargo_mode, std::env::var_os)`
once for the whole selected set, and only then iterate over the returned
`BuiltCommand`s to spawn processes. Do not build/spawn one check at a time:
`devtool check --all --sandbox-cargo` must validate every selected Cargo
workspace home before any Cargo command can run.

- [x] **Step 4: Update existing tests for the new return type**

Update existing `fmt_check_vs_fix`, `prettier_covers_end2end_and_markdown`,
`ert_and_tsc_ignore_fix`, `byte_compile_runs_the_script_and_ignores_fix`,
`tools_fmt_targets_tools_workspace`, `unknown_check_errors`, and
`all_names_have_specs` to inspect `CheckSpec`/`BuiltCommand` instead of the old
`(&str, Vec<String>)` tuple.

Keep their asserted command shapes identical in host mode.

- [x] **Step 5: Run targeted devtool tests**

Run:
`devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture`

Expected: PASS. JSON summary has `"ok": true` and `"exit_code": 0`.

---

### Task 2: Nix static-check sandbox Cargo homes

**Files:**

- Modify: `flake.nix:290-386,1244-1283`
- Test: `tools/devtool/src/check.rs` tests from Task 1

**Interfaces:**

- Consumes from Task 1:
  - `devtool check --all --sandbox-cargo`
  - env vars `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` and
    `JAUNDER_DEVTOOL_TOOLS_CARGO_HOME`
- Produces:
  - flake-local `appCargoVendorDir = craneLib.vendorCargoDeps { inherit src; }`
  - flake-local
    `toolsCargoVendorDir = craneLib.vendorCargoDeps { src = toolsSrc; }`
  - flake-local helper
    `mkOfflineCargoHome = { name, vendorDir }: pkgs.runCommand ...`
  - flake-local `appOfflineCargoHome`
  - flake-local `toolsOfflineCargoHome`

- [x] **Step 1: Add flake bindings for offline Cargo homes**

Near the existing `cargoArtifacts` / `toolsCargoArtifacts` bindings, add
explicit vendor-dir and cargo-home derivations. The config body must include
both source replacement and Cargo's offline setting:

```nix
mkOfflineCargoHome =
  { name, vendorDir }:
  pkgs.runCommand "${name}-cargo-home" { } ''
    mkdir -p $out
    cat > $out/config.toml <<'EOF'
    [source.crates-io]
    replace-with = "vendored-sources"

    [source.vendored-sources]
    directory = "${vendorDir}"

    [net]
    offline = true
    EOF
  '';
```

If shell quoting requires adjustment, keep the resulting `$out/config.toml`
semantically identical: one config per workspace, each pointing at only its own
`vendorDir`.

- [x] **Step 2: Wire static-checks to sandbox Cargo mode**

In the `static-checks` `runCommand` attributes, set:

```nix
JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME = "${appOfflineCargoHome}";
JAUNDER_DEVTOOL_TOOLS_CARGO_HOME = "${toolsOfflineCargoHome}";
```

Change the script from:

```sh
devtool check --all
```

to:

```sh
devtool check --all --sandbox-cargo
```

This makes the Nix sandbox path exercise the new offline enforcement for the
existing Cargo-backed `fmt` and `tools-fmt` migrated checks without moving
compiling checks into devtool.

- [x] **Step 3: Run the static-check derivation**

Run: `devtool run -- nix build .#checks.x86_64-linux.static-checks`

Expected: PASS. JSON summary has `"ok": true` and `"exit_code": 0`.

- [x] **Step 4: Re-run targeted devtool tests**

Run:
`devtool run -- cargo test --manifest-path tools/Cargo.toml check::tests -- --nocapture`

Expected: PASS. JSON summary has `"ok": true` and `"exit_code": 0`.

---

### Task 3: Gate and commit

**Files:**

- Modify: `tools/devtool/src/main.rs`
- Modify: `tools/devtool/src/check.rs`
- Modify: `flake.nix`
- Add:
  `docs/superpowers/specs/2026-08-20-issue-1073-devtool-offline-cargo-config.md`
- Add:
  `docs/superpowers/plans/2026-08-20-issue-1073-devtool-offline-cargo-config.md`

**Interfaces:**

- Consumes: Task 1 and Task 2 completed code and docs.
- Produces: one checked commit for issue #1073.

- [x] **Step 1: Inspect the diff**

Run:
`git diff -- tools/devtool/src/main.rs tools/devtool/src/check.rs flake.nix docs/superpowers/specs/2026-08-20-issue-1073-devtool-offline-cargo-config.md docs/superpowers/plans/2026-08-20-issue-1073-devtool-offline-cargo-config.md`

Expected: changes are limited to the planned files; no compiling checks moved
behind `devtool check`; no workspace membership changed.

- [x] **Step 2: Run the full check gate**

Run: `devtool run -- cargo xtask check`

Expected: PASS. JSON summary has `"ok": true` and `"exit_code": 0`. If
formatters modify docs/code, inspect and stage those exact changes before
committing.

- [x] **Step 3: Stage exactly the implementation and cycle docs**

Run:

```bash
git add tools/devtool/src/main.rs tools/devtool/src/check.rs flake.nix docs/superpowers/specs/2026-08-20-issue-1073-devtool-offline-cargo-config.md docs/superpowers/plans/2026-08-20-issue-1073-devtool-offline-cargo-config.md
```

- [x] **Step 4: Inspect staged diff**

Run: `git diff --cached --stat`

Expected: staged files are exactly the five files listed above unless the gate
formatter made a necessary formatting-only adjustment in those files.

- [x] **Step 5: Commit**

Run:

```bash
git commit -m "build(devtool): add offline cargo workspace config"
```

Before committing, tick all completed task checkboxes in this plan. Do not add a
`Co-Authored-By` trailer.
