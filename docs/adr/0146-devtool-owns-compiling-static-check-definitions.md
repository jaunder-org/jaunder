# ADR-0146: devtool owns compiling static-check definitions across host and Nix

- Status: accepted
- Date: 2026-08-21
- Issue: [#276](https://github.com/jaunder-org/jaunder/issues/276)

## Context

[ADR-0052](0052-devtool-unifies-static-checks.md) made `devtool check` the
single definition surface for the non-compiling static checks. It deliberately
left compiling checks outside that surface because the repository did not yet
have tools-workspace vendoring, per-workspace offline Cargo configuration, or an
explicit sandbox cargo-deny policy.

Those blockers have since been resolved. The tools workspace has reusable
vendoring artifacts, sandboxed `devtool check` can select workspace-specific
offline Cargo homes, and [ADR-0145](0145-sandbox-cargo-deny-skips-advisories.md)
defines the cargo-deny host/sandbox split.

At the same time, #1106 makes the local-performance direction explicit:
day-to-day development should run more work host-native under the Nix dev shell
toolchain so host Cargo artifacts and sccache can be reused across worktrees.
Hermetic Nix builds remain the authority for CI and release checks. The
remaining decision is therefore not whether every lane should execute the same
way, but where the static-check command definitions live.

## Decision

`devtool check` owns the command definitions for the compiling project/tool
static checks as well as the non-compiling checks. That includes product
`clippy`, wasm-target `wasm-clippy`, product `cargo-deny`, and tools workspace
`tools-clippy`.

Host `xtask` lanes invoke those checks through `devtool check <name>` while
still running host-local Cargo and preserving the existing compile-cache
environment for Rust-compiling checks. Sandboxed Nix lanes invoke the same
definitions through `devtool check --all --sandbox-cargo`, using the matching
workspace-specific offline Cargo home and forcing Cargo offline before spawning
Cargo.

The separate crane `clippy`, `wasm-clippy`, and `deny` check derivations are
removed only with replacement wiring that keeps the expanded hermetic
`static-checks` derivation on the required validation path.
`cargo xtask validate --no-e2e` builds it as the `nix-static-checks` step before
the Nix test checks.

`xtask-fmt` and `xtask-clippy` remain native host checks. `xtask/` is excluded
from the flake source and must not be invoked from Nix derivations.

## Consequences

The remaining ADR-0052 duplication class closes: static-check command arguments
for product clippy, wasm clippy, cargo-deny, and tools clippy live in `devtool`
rather than being copied between `xtask` StepSpecs and flake crane derivations.

Host and sandbox lanes intentionally keep different execution environments. That
preserves the #1106 host-native performance direction without letting the host
and hermetic check policies drift.

The Nix `static-checks` derivation becomes broader and no longer means
"non-compiling only." Documentation and CI/xtask validation must distinguish the
host static-check step from the hermetic Nix static-check derivation by
execution lane, not by compile boundary.

Cargo-deny keeps [ADR-0145](0145-sandbox-cargo-deny-skips-advisories.md)'s
policy split: host mode runs full `cargo deny check`; sandbox mode skips
advisories and runs the offline-safe `bans`, `licenses`, and `sources` checks.
