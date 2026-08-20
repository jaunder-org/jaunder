# #1072 — reusable tools workspace cargo artifacts

Issue: [#1072](https://github.com/jaunder-org/jaunder/issues/1072). Milestone:
Developer tooling & DX. Relevant decisions:
[ADR-0028](../../adr/0028-devtool-vs-xtask-boundary.md),
[ADR-0141](../../adr/0141-cargo-workspace-execution-boundaries.md).

## Summary

`flake.nix` already builds `devtool` from the separate `tools/` workspace using
`tools/Cargo.lock`, but that vendoring is private to `devtoolBin`. Future
compiling checks need a reusable Nix artifact set for the same workspace without
merging `tools/` into the product workspace or letting sandboxed Cargo resolve
dependencies from the network.

This change exposes the tools workspace cargo-dependency/artifact boundary as a
named flake-local binding and rewires the existing `devtool` package to consume
it. The result is intentionally infrastructure only: it prepares the Nix side
for later `devtool check tools-clippy` work, but does not move `clippy`, `deny`,
or `tools-clippy` behind `devtool check` in this issue.

## Decisions

| ID  | Decision                                                                                                                                                                                                                                                                     |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| D1  | Keep `tools/` a separate Cargo workspace. Do not add it to the root workspace and do not share the root `cargoArtifacts`.                                                                                                                                                    |
| D2  | Introduce explicit flake-local names for the `tools/` workspace source and dependency artifacts, e.g. `toolsSrc` and `toolsCargoArtifacts`.                                                                                                                                  |
| D3  | Build `toolsCargoArtifacts` with `craneLib.buildDepsOnly` over `toolsSrc`, so consumers can compile `tools/` crates in Nix without network dependency resolution.                                                                                                            |
| D4  | Rewire `devtoolBin` to inherit the new tools cargo artifacts rather than relying on an implicit per-package vendoring path.                                                                                                                                                  |
| D5  | Keep the new artifacts internal to the system-specific flake `let` unless implementation proves a public `packages`/`checks` output is required for verification. The acceptance criterion is reusable by future Nix checks inside the flake, not a user-facing package API. |
| D6  | Document the boundary next to the new bindings: root app artifacts, tools artifacts, and host-only `xtask` remain separate because of ADR-0028 and ADR-0141.                                                                                                                 |

## Acceptance criteria

- **AC1 — tools artifacts exist.** `flake.nix` has a named reusable
  tools-workspace cargo artifact binding derived from
  `tools/Cargo.lock`/`toolsSrc`.
- **AC2 — existing consumer uses them.** `devtoolBin` consumes that binding,
  proving it is usable by a Nix package that compiles a `tools/` workspace
  crate.
- **AC3 — workspace boundaries preserved.** `tools/` remains a separate
  workspace; `xtask/` remains host-only and excluded from the flake source; root
  application cargo artifacts are not reused for `tools/`.
- **AC4 — no premature check migration.** The branch does not move `clippy`,
  `deny`, `wasm-clippy`, or `tools-clippy` behind `devtool check`;
  #276/#1073/#1074 remain separate follow-up work.
- **AC5 — verification.** `devtool run -- cargo xtask check` passes.

## Out of scope

- Implementing per-workspace offline Cargo config selection in `devtool check`
  (#1073).
- Choosing the sandboxed `cargo-deny` advisory policy (#1074).
- Unifying compiling static checks behind `devtool check` (#276).
- Adding or removing Cargo workspace members.
- Adding a new ADR; this issue realizes the existing ADR-0028/ADR-0141 boundary
  rather than changing it.
