# Implement xtask compile caching and lean profiles

- Issue: [#1071](https://github.com/jaunder-org/jaunder/issues/1071)
- Spec:
  [`2026-08-20-issue-1071-xtask-compile-cache-spec.md`](2026-08-20-issue-1071-xtask-compile-cache-spec.md)

## Tasks

- [x] Inspect current xtask static-check command construction and flake check
      derivations.
- [x] Validate multi-root `SCCACHE_BASEDIRS`: copy B hit 453 Rust compile
      entries after copy A primed the cache.
- [x] Add `sccache` to the dev shell inputs.
- [x] Route host compiling static-check steps through `RUSTC_WRAPPER=sccache`
      with `CARGO_INCREMENTAL=0` and linked-worktree `SCCACHE_BASEDIRS`.
- [x] Add debug-info reductions to safe Nix clippy/test derivations.
- [x] Document scope and rationale beside the changed build seams.
- [x] Add focused xtask tests for compiling-step cache behavior.
- [x] Run focused tests.
- [x] Run `cargo xtask check`.
