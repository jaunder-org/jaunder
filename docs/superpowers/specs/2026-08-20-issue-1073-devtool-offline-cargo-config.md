# #1073 — devtool per-workspace offline Cargo config

Issue: [#1073](https://github.com/jaunder-org/jaunder/issues/1073). Milestone:
Developer tooling & DX. Relevant decisions:
[ADR-0028](../../adr/0028-devtool-vs-xtask-boundary.md),
[ADR-0141](../../adr/0141-cargo-workspace-execution-boundaries.md). Parent
issue: [#276](https://github.com/jaunder-org/jaunder/issues/276). Prerequisite:
[#1072](https://github.com/jaunder-org/jaunder/issues/1072) is complete.

## Summary

`devtool check` needs a boring, explicit way to run future Cargo-backed checks
against the correct Cargo workspace and the correct offline Cargo source
replacement.

Today `devtool check` owns only non-compiling static checks. Host
`cargo xtask check` still runs `clippy`, `wasm-clippy`, `cargo-deny`, and
`tools-clippy` as native host commands, while Nix still uses separate crane
derivations for `clippy`, `wasm-clippy`, and `deny`. #1072 added reusable
`tools/` cargo artifacts, but `devtool` still has no model for choosing between
the product workspace and the auxiliary `tools/` workspace when a check invokes
Cargo in a sandbox.

This issue adds that model only. It does not move the compiling checks behind
`devtool check` yet.

## Decisions

| ID  | Decision                                                                                                                                                                                                                                                                                                                            |
| --- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Add an internal `devtool check` Cargo execution abstraction that names the Cargo workspace being checked instead of accepting an unstructured `cargo` command.                                                                                                                                                                      |
| D2  | Model at least two workspaces: the root product workspace and the `tools/` auxiliary workspace. Each workspace records its manifest/root identity and the sandbox Cargo-home/config inputs needed to resolve that workspace offline.                                                                                                |
| D3  | In a sandboxed Nix invocation, Cargo-backed checks must run with an explicit workspace-specific `CARGO_HOME` or equivalent Cargo config directory that points only at that workspace's vendored dependency tree, and Cargo must be forced offline with `--offline` and/or `CARGO_NET_OFFLINE=true`. Missing config is a hard error. |
| D4  | In a normal host invocation, Cargo-backed checks may continue using the developer's existing Cargo environment unless the caller explicitly selects sandbox/offline behavior.                                                                                                                                                       |
| D5  | Keep #276 behavior unchanged: `clippy`, `wasm-clippy`, `cargo-deny`, and `tools-clippy` remain native `xtask`/crane checks until follow-up issues wire them through this abstraction.                                                                                                                                               |

## Acceptance criteria

- **AC1 — workspace model exists.** `tools/devtool/src/check.rs` (or a directly
  related module) has a typed representation for Cargo-check workspaces that
  distinguishes the root product workspace from `tools/`.
- **AC2 — command construction is workspace-aware.** The new API can produce
  Cargo commands for both workspace kinds with the correct manifest/root
  arguments instead of relying on stringly ad hoc command assembly.
- **AC3 — sandbox config is explicit and offline.** A sandbox/offline Cargo
  invocation for the root workspace and for `tools/` uses separate Cargo
  config/Cargo-home inputs and forces Cargo offline with `--offline` and/or
  `CARGO_NET_OFFLINE=true`; if either selected workspace lacks the required
  input, `devtool` errors before spawning Cargo.
- **AC4 — wrong vendor tree is structurally hard to use.** The product workspace
  cannot silently use the `tools/` vendored source tree, and `tools/` cannot
  silently use the product vendored source tree, because workspace selection and
  offline config selection are coupled in the API.
- **AC5 — host behavior unchanged.** Existing non-compiling
  `devtool check --all` behavior and `cargo xtask check` step ordering stay
  unchanged.
- **AC6 — gate passes.** `devtool run -- cargo xtask check` passes.

## Out of scope

- Moving `clippy`, `wasm-clippy`, `cargo-deny`, or `tools-clippy` to
  `devtool check`; that is #276 and #1074 follow-up work.
- Changing crane `clippy`, `wasm-clippy`, or `deny` derivations.
- Adding/removing Cargo workspace members.
- Adding a new ADR unless implementation discovers a boundary change that
  contradicts ADR-0028 or ADR-0141.
