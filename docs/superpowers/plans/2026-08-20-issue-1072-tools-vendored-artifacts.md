# #1072 Tools Vendored Artifacts Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose reusable Nix cargo artifacts for the separate `tools/`
workspace and prove them through the existing `devtool` package.

**Architecture:** Keep the change entirely in the system-specific `flake.nix`
`let`: rename the existing `devtoolSrc` boundary to a reusable `toolsSrc`, add
`toolsCargoArtifacts = craneLib.buildDepsOnly toolsArgs`, and make `devtoolBin`
consume those artifacts. Root application `cargoArtifacts` remains separate;
`xtask/` remains outside flake source and host-only.

**Tech Stack:** Nix flake, crane, Rust `tools/` workspace, `devtool run`,
`cargo xtask check`.

## Global Constraints

- Follow the approved spec:
  `docs/superpowers/specs/2026-08-20-issue-1072-tools-vendored-artifacts.md`.
- Preserve ADR-0028: `devtool` is in-sandbox producer/runner code; `xtask` is
  host-only analyzer/orchestrator.
- Preserve ADR-0141: `tools/` remains a separate auxiliary Cargo workspace; do
  not add it to the root workspace.
- Do not move `clippy`, `deny`, `wasm-clippy`, or `tools-clippy` behind
  `devtool check` in this issue.
- Do not implement #1073 per-workspace Cargo config selection or #1074
  cargo-deny sandbox policy.
- Verification gate: `devtool run -- cargo xtask check`.

## Review Header

**Scope in:** `flake.nix` bindings around the existing `devtool` package;
optional wording updates adjacent to those bindings only.

**Scope out:** new public flake outputs, Cargo workspace membership changes,
static-check migration, cargo-deny policy, devtool command/API changes, ADR
changes.

**Tasks:**

1. Rename the existing tools source boundary and add reusable tools artifacts.
2. Rewire `devtoolBin` to consume the reusable artifacts and verify the focused
   Nix package build.
3. Run the full check gate and commit the implementation plus approved cycle
   docs.

**Key risks/decisions:**

- `craneLib.buildDepsOnly` must be fed the same `tools/` source and inputs
  `devtoolBin` uses, otherwise this creates a second inconsistent vendoring
  path.
- The binding may stay flake-local; future checks can consume it from the same
  `let` without committing a public API now.
- Building `devtool` is the proof consumer for #1072; check unification remains
  follow-up work.

---

### Task 1: Name the tools workspace artifact boundary

**Files:**

- Modify: `flake.nix:345-362`

**Interfaces:**

- Consumes: current
  `devtoolSrc = pkgs.lib.cleanSourceWith { src = craneLib.path ./tools; filter = craneLib.filterCargoSources; };`
- Produces:
  - `toolsSrc`: cleaned source for the separate `tools/` Cargo workspace.
  - `toolsArgs`: common crane arguments for packages/checks compiling the
    `tools/` workspace.
  - `toolsCargoArtifacts`: reusable dependency artifacts built from `toolsArgs`.

- [x] **Step 1: Edit the flake bindings**

Replace the current `devtoolSrc`-only block with:

```nix
        # The auxiliary tools workspace is separate from the product workspace
        # (ADR-0141). Keep its source and cargo artifacts separate from
        # `commonArgs`/`cargoArtifacts`: `tools/Cargo.lock` owns these deps, while
        # `xtask/` remains host-only and outside the flake source (ADR-0028).
        toolsSrc = pkgs.lib.cleanSourceWith {
          src = craneLib.path ./tools;
          filter = craneLib.filterCargoSources;
        };
        toolsArgs = {
          src = toolsSrc;
          pname = "jaunder-tools";
          version = "0.1.0";
          strictDeps = true;
        };
        toolsCargoArtifacts = craneLib.buildDepsOnly toolsArgs;
```

Keep the comment factual: this binding exists for `tools/`, not for `xtask/`,
and does not imply `tools/` is absent from all Nix derivations.

- [x] **Step 2: Inspect the diff**

Run:

```bash
git diff -- flake.nix
```

Expected: one local flake binding block is renamed/expanded; no check
definitions, packages, or workspace manifests change.

### Task 2: Rewire `devtoolBin` to consume the reusable artifacts

**Files:**

- Modify: `flake.nix:355-362`

**Interfaces:**

- Consumes: `toolsArgs`, `toolsCargoArtifacts` from Task 1.
- Produces:
  `devtoolBin = craneLib.buildPackage (toolsArgs // { inherit cargoArtifacts; pname = "devtool"; cargoExtraArgs = "-p devtool"; doCheck = false; });`,
  where `cargoArtifacts` is sourced from `toolsCargoArtifacts` without colliding
  with the root `cargoArtifacts` binding.

- [x] **Step 1: Update `devtoolBin`**

Rewrite `devtoolBin` to consume `toolsArgs` and the new artifact binding. Use a
local attribute rename so the code is unambiguous:

```nix
        devtoolBin = craneLib.buildPackage (
          toolsArgs
          // {
            cargoArtifacts = toolsCargoArtifacts;
            pname = "devtool";
            cargoExtraArgs = "-p devtool";
            doCheck = false;
          }
        );
```

- [x] **Step 2: Build the proof consumer**

Run:

```bash
devtool run -- nix build .#devtool
```

Expected: PASS — JSON summary has `ok: true` and `exit_code: 0`. This proves a
Nix package compiling a `tools/` crate consumes `toolsCargoArtifacts` without
network resolution.

If this fails because crane requires package-specific `cargoExtraArgs` during
`buildDepsOnly`, adjust `toolsArgs` minimally and rerun this same command. Do
not widen the issue into #1073.

- [x] **Step 3: Inspect boundaries**

Run:

```bash
git diff -- flake.nix
```

Expected:

- `toolsSrc`, `toolsArgs`, and `toolsCargoArtifacts` exist.
- `devtoolBin` uses `cargoArtifacts = toolsCargoArtifacts`.
- Existing root `cargoArtifacts = craneLib.buildDepsOnly commonArgs` is
  unchanged.
- No `Cargo.toml` workspace membership changes.
- No `devtool check` static-check migration.

### Task 3: Gate and commit

**Files:**

- Modify: `flake.nix`
- Add:
  `docs/superpowers/specs/2026-08-20-issue-1072-tools-vendored-artifacts.md`
- Add:
  `docs/superpowers/plans/2026-08-20-issue-1072-tools-vendored-artifacts.md`

**Interfaces:**

- Consumes: passing Task 2 proof build.
- Produces: one checked commit for issue #1072.

- [x] **Step 1: Run the full check gate**

Run:

```bash
devtool run -- cargo xtask check
```

Expected: PASS — JSON summary has `ok: true` and `exit_code: 0`.

- [x] **Step 2: Inspect post-gate changes**

Run:

```bash
git status --short
```

Expected: only `flake.nix` plus the approved spec/plan docs are changed. If
formatters modified docs, include those exact changes in the commit.

- [x] **Step 3: Stage exactly the implementation and cycle docs**

Run:

```bash
git add flake.nix docs/superpowers/specs/2026-08-20-issue-1072-tools-vendored-artifacts.md docs/superpowers/plans/2026-08-20-issue-1072-tools-vendored-artifacts.md
```

- [x] **Step 4: Inspect staged diff**

Run:

```bash
git diff --cached --stat
```

Expected: `flake.nix` changed and the two cycle docs added.

- [x] **Step 5: Commit**

Run:

```bash
git commit -m "build(nix): expose tools cargo artifacts"
```

No `Co-Authored-By` trailer.
