# #276 — compiling static-check definitions share one devtool surface

## Context

ADR-0052 moved the non-compiling static checks behind `devtool check` so the
host ladder and the Nix `static-checks` derivation share one command definition.
The compiling checks stayed split because they needed offline Cargo plumbing,
tools-workspace vendoring, and an explicit cargo-deny sandbox policy.

Those prerequisites now exist: #1072 added tools-workspace vendoring artifacts,
#1073 added workspace-specific offline Cargo homes for sandboxed Cargo checks,
and #1074 / ADR-0145 defined the cargo-deny host/sandbox policy.

#1106 also clarifies the performance direction. Day-to-day work should keep
host-native lanes that run under the Nix dev shell toolchain and can reuse Cargo
target artifacts and sccache. Hermetic Nix checks remain the release authority,
and required CI must still exercise the hermetic static-check signal before this
issue removes the existing crane check derivations. This issue therefore unifies
check definitions and arguments, not execution environments.

## Decisions

| ID  | Decision                                                                                                                                                                                                                             |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| D1  | `devtool check` becomes the single definition surface for the remaining compiling project/tool static checks: product `clippy`, wasm-target `wasm-clippy`, product `cargo-deny`, and tools workspace `tools-clippy`.                 |
| D2  | Host `xtask` routes those compiling checks through `devtool check <name>` but preserves host-local Cargo execution and the existing compile-cache environment for Rust-compiling checks.                                             |
| D3  | Sandbox/Nix execution routes the same logical check definitions through `devtool check --all --sandbox-cargo`, using the matching workspace-specific offline Cargo home and forcing Cargo offline before spawning Cargo.             |
| D4  | `cargo-deny` keeps ADR-0145's split policy: host mode runs full `cargo deny check`; sandbox mode runs the offline-safe `bans`, `licenses`, and `sources` checks.                                                                     |
| D5  | `xtask-fmt` and `xtask-clippy` remain native host-only `StepSpec`s because `xtask/` is excluded from the flake source and must not be invoked from Nix derivations.                                                                  |
| D6  | Remove separate crane `clippy`, `wasm-clippy`, and `deny` check derivations as part of #276, but only with replacement wiring that proves required flake/CI validation still exercises the expanded hermetic `static-checks` signal. |

## Acceptance Criteria

### AC1 — devtool owns compiling check command definitions

`tools/devtool/src/check.rs` defines specs for these check names:

- `clippy`
- `wasm-clippy`
- `cargo-deny`
- `tools-clippy`

The host command arguments must match the pre-existing host ladder behavior:

- product `clippy`: `cargo clippy --all-targets -- -D warnings`
- `wasm-clippy`: clippy for packages `web`, `client`, and `csr` with feature
  `csr`, target `wasm32-unknown-unknown`, and `-D warnings`
- `cargo-deny`: `cargo deny check`
- `tools-clippy`: tools workspace clippy with `--all-targets` and `-D warnings`

### AC2 — host ladder delegates through devtool without losing cache behavior

`xtask/src/steps/static_checks.rs` delegates `cargo-deny`, `clippy`,
`wasm-clippy`, and `tools-clippy` to `devtool check <name>` while keeping
`xtask-fmt` and `xtask-clippy` native.

Rust-compiling delegated checks preserve the existing host compile-cache
environment:

- `RUSTC_WRAPPER=sccache`
- `CARGO_INCREMENTAL=0`
- derived `SCCACHE_BASEDIRS`

`cargo-deny` remains non-cacheable because it does not compile Rust code.

### AC3 — sandbox mode is offline and workspace-specific

For sandboxed Cargo-backed checks, `devtool check --sandbox-cargo` must:

- require the correct workspace Cargo home before spawning the command;
- set `CARGO_HOME` to that workspace's offline Cargo home;
- force offline mode with `--offline` and `CARGO_NET_OFFLINE=true`;
- route product checks through the product workspace;
- route tools checks through `tools/Cargo.toml`;
- fail before command execution when the required Cargo home is missing.

### AC4 — cargo-deny policy remains explicit

Tests lock both cargo-deny modes:

- host `devtool check cargo-deny` builds `cargo deny check`;
- sandbox `devtool check cargo-deny --sandbox-cargo` builds
  `cargo --offline deny check bans licenses sources`;
- sandbox cargo-deny does not include `advisories`.

### AC5 — hermetic flake signal is preserved

The Nix `static-checks` derivation runs `devtool check --all --sandbox-cargo`
with all required tool inputs and both product/tools offline Cargo homes.

The separate crane `clippy`, `wasm-clippy`, and `deny` check derivations must be
removed from the flake check set once the expanded `static-checks` derivation
replaces their hermetic signal.

Required CI validation must still exercise that hermetic static-check signal. If
`cargo xtask validate --no-e2e` remains the required PR/merge-queue entrypoint,
then it must build or otherwise depend on the expanded Nix `static-checks`
derivation after the crane derivations are removed. A branch that only keeps the
expanded derivation available through manual `nix flake check` but does not put
it on the required CI path does not satisfy this criterion.

### AC6 — architecture documentation reflects the new boundary

The branch must record a numberless ADR draft superseding ADR-0052's old
compile-boundary deferral. That draft must explain why compiling static checks
now share the `devtool check` definition surface while host and sandbox
execution remain separate lanes.

`docs/ARCHITECTURE.md` and `CONTRIBUTING.md` must no longer describe the
compiling project/tool checks as native `xtask`/crane-owned duplicates once they
move behind `devtool check`. They must explicitly state:

- host lanes remain host-native for performance and cache reuse;
- Nix/CI lanes remain hermetic through sandboxed `devtool check`;
- `xtask` self-lints remain native host checks.

`docs/ARCHITECTURE.md` must cite the draft ADR by its
`docs/adr/drafts/<slug>.md` path so `cargo xtask adr promote` rewrites the link
at ship.

### AC7 — verification covers both command construction and real gates

Focused tests cover:

- devtool command construction for host and sandbox compiling checks;
- missing offline Cargo-home failure paths;
- xtask step routing and cacheability;
- final step ordering.

Implementation must also verify the expanded hermetic static-check surface with
the Nix `static-checks` derivation and run the repository gate appropriate for
each commit.

## Out of Scope

- Adding the #1106 host-native product test lane.
- Vendoring or otherwise supplying the RustSec advisory database to make
  sandboxed cargo-deny run `advisories`.
- Moving `xtask-fmt` or `xtask-clippy` into `devtool check`.
- Changing the root/tools/xtask workspace boundaries.
