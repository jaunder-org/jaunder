# Cache every devtool Cargo bootstrap (#1757)

## Outcome

Every host static-check launch that bootstraps devtool through Cargo explicitly
receives the existing Rust compile-cache policy, independent of ambient settings
and of whether the selected check itself compiles Rust. This establishes wrapper
policy correctness, not measured performance savings or historical attribution.

## Load-bearing decisions

- Implementation and unit tests remain in `xtask/src/steps/static_checks.rs`.
- Extract a narrow execution helper accepting a compile-cache provider and
  command runner; production uses the existing provider and environment runner.
- Apply that policy to every devtool Cargo bootstrap in the real catalog.
- Preserve exact command arguments, Check/Fix modes, result failures and cache
  warning details. Direct `xtask-fmt` receives no cache overrides.
- Preserve sandbox semantics, validation populations and #1753's deliberate
  wrapper-removal exception. No global runner framework is introduced.

## Acceptance

- Recorder tests exercise the actual catalog and production execution helper in
  both modes, asserting program, arguments and explicit `RUSTC_WRAPPER=sccache`,
  `CARGO_INCREMENTAL=0` and provider-supplied `SCCACHE_BASEDIRS` overrides.
- Tests prove direct formatting exclusion and provider non-use, plus
  preservation of failed runner results and warnings with and without command
  detail.
- The focused offline, locked, one-job test command produces genuine RED before
  the policy fix and GREEN afterward using an own-checkout private target.
- Evidence stays in ignored run storage; independent review precedes any source
  commit, push or PR. This spec is the working conformance contract and is
  deleted during final lifecycle cleanup after review.

## Boundaries

Auxiliary host-test caching, Nix/dependency/global configuration changes, cache
or daemon intervention, benchmarks and historical capture attribution are out of
scope. Stop on missing offline dependencies, timeout, signal, capacity concern
or sccache reset; do not retry or bypass. Each focused invocation has an
approved 1800-second ceiling. No other owner's tree or target is used.
