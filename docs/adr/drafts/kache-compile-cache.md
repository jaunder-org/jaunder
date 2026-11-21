# ADR-DRAFT: kache replaces sccache as the host compile cache

- Status: proposed
- Date: 2026-11-20
- Issue: [#1972](https://github.com/jaunder-org/jaunder/issues/1972)

## Context

[ADR-0146](../0146-devtool-owns-compiling-static-check-definitions.md) kept host
Cargo artifacts and sccache reusable across worktrees, and
[ADR-0237](../0237-just-owned-gate-workflows.md) made the gate runner set the
host compile-cache policy, including a `SCCACHE_BASEDIRS` list computed from
`git worktree list`. [ADR-0251](../0251-dev-runner-replaces-just.md) now owns
that environment. CI ran sccache with an empty store in every job.

Measured on #1972 with kache 2.1.1: sccache missed about a quarter of Rust
compiles in a second worktree of the same commit, even with both roots in
`SCCACHE_BASEDIRS`, and `dev check-no-test` took 669–682 s there against 267–268
s with kache. In paired CI runs, kache with executables uncached beat sccache
and a persisted `target/` on every change. With executables cached, the store
outgrew the runner's default cap and lost hits to pruning. A `target/` shared
across worktrees gave stale Clippy results through Cargo's mtime freshness. Full
debug info made each worktree's `target/` about twice the size of line tables
only.

Those CI runs measured a host lane that still ran Clippy. Since
[CI's Clippy verdict comes from Nix](../0255-ci-clippy-verdict-from-nix.md), the
host lane compiles only the tools workspace, so its store is smaller than
measured.

## Decision

Host Cargo compilation runs under kache with one configuration locally and in
CI. `dev` sets `RUSTC_WRAPPER=kache`, `CARGO_INCREMENTAL=0` and
`KACHE_CACHE_EXECUTABLES=0` for every host step; everything else is kache's
default. No worktree discovery is computed, and nothing writes kache, Cargo or
shell configuration into users' files. `KACHE_DISABLED=1` is the documented off
switch.

Locally one per-user store serves every worktree. In CI only `validate-host`,
which builds and tests the tools workspace on the host, persists its store
through the Actions cache: pushes to `main` save it under a run-stamped key
after running `kache gc` and stopping the daemon, and pull requests and the
merge queue restore the newest entry. Other jobs run the wrapper with an empty,
unpersisted store.

Dev and test builds of both Cargo workspaces emit line-table debug info only
(`[profile.dev] debug = "line-tables-only"`).

This amends the sccache clauses of ADR-0146 and ADR-0237.

## Consequences

- The dev shells carry kache in place of sccache, and the mutants shell has it
  on `PATH` without a wrapper. It comes from a second nixpkgs input,
  `nixpkgs-kache`, because the main input's revision has no kache and bumping it
  broke `otelcol-contrib`; that input goes once the main one carries kache.
- Worktrees share disk only on a reflink filesystem; on ext4 the store is extra
  disk on top of each worktree's `target/`.
- `validate-host` downloads the store on every run, and each `main` save evicts
  the least recently used Actions cache entries.
- Executables, including build scripts, are relinked or recompiled rather than
  restored.
- Full debug info is no longer the default for dev and test builds.
- Not decided here: building host tools with Nix instead of from the checkout,
  which retires CI's `tools/target` cache (#1968).
