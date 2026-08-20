# Speed up xtask checks with compile caching and lean profiles

- Issue: [#1071](https://github.com/jaunder-org/jaunder/issues/1071)

## Problem

`cargo xtask check` and `cargo xtask validate` compile broad Rust surfaces even
when a small change lands in one crate. Nix already caches derivation outputs,
but the repository's gate derivations are intentionally whole-contract checks,
so small source edits can still rebuild large graphs. Agents also run in
separate checkouts and target directories, so Cargo's local incremental cache
often cannot help them share compiler work.

Several compile-only or test regimes do not need full debugger metadata.
Coverage already disables DWARF with `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0`; the same principle can apply to lint/test gates
whose diagnostics do not depend on full debug info.

`sccache` only helps Rust when two conditions hold: Cargo incremental is off,
and every checkout root that should share cached artifacts appears in
`SCCACHE_BASEDIRS`. A two-copy probe of this repo showed the shape works: after
priming copy A, copy B hit 453 Rust compile entries and missed 131, a 77.57%
Rust hit rate.

## Decision

Use two boring speed levers before changing the check contract:

1. Add a pinned `sccache` binary to the dev shell and route host cargo
   invocations through it only for gate steps that compile Rust. xtask reads
   `git worktree list --porcelain` and passes the existing linked-worktree roots
   as `SCCACHE_BASEDIRS`; `CARGO_INCREMENTAL=0` makes the rustc calls cacheable.
2. Add per-derivation debug-info reductions for Nix-backed compile/test gates
   where semantics remain intact.

Do not add a changed-crate-only gate in this issue. Such a mode would be a
preflight, not a replacement for `cargo xtask check` or `validate`.

## Acceptance criteria

- The dev shell supplies `sccache`.
- Host compiling static-check steps use `RUSTC_WRAPPER=sccache`,
  `CARGO_INCREMENTAL=0`, and linked-worktree roots in `SCCACHE_BASEDIRS` without
  applying the wrapper to non-compiling checks or production Nix packages.
- Nix-backed clippy, wasm test, and doctest derivations disable or reduce debug
  info with local env overrides, not a global workspace/profile change.
- Coverage keeps its existing no-DWARF setup and behavior.
- The code documents why each cache/profile override is scoped where it is.
- Focused tests cover static-check command construction/environment behavior.
- `cargo xtask check` passes.
