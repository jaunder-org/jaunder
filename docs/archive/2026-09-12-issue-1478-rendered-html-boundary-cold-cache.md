# Cold-cache-safe rendered HTML compiler boundary

## Outcome

`rendered-html-compiler-boundary` resolves its isolated downstream fixture from
the repository's Nix-vendored product dependency closure. The required host gate
therefore succeeds on a clean runner without depending on an incidental Cargo
registry cache or network access.

## Load-bearing decisions

- Preserve ADR-0079's isolated downstream crate, disabled default features,
  positive dependency-resolution fixture, raw-construction rejection, and
  test-support rejection.
- Keep Cargo offline during the fixture checks. Dependency availability comes
  from the same `appOfflineCargoHome` generated from the reviewed root
  `Cargo.lock`; the check does not fetch from the network.
- Expose that existing product Cargo home through the CI and default development
  shells. The compiler boundary seeds a fresh temporary Cargo home with only its
  vendored source configuration, then explicitly selects that home for the
  subprocess. The ambient user or runner registry therefore cannot affect
  resolution.
- Reuse the established `JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME` contract rather
  than adding a second name for the same Nix store path. The variable denotes
  the product workspace's sandbox Cargo home even when xtask is its consumer.
- Treat absence of the configured product Cargo home as a clear gate failure,
  not as permission to fall back to the ambient cache.
- No ADR is required: this restores deterministic execution of the compiler
  proof already required by ADR-0079 and changes no product architecture.

## Acceptance

- The boundary's nested Cargo command retains `--offline` and receives a fresh
  Cargo home containing only the Nix-vendored product source configuration.
- An ambient Cargo registry or cache cannot change the positive or negative
  fixture outcomes.
- The positive fixture resolves and compiles; both privacy fixtures continue to
  fail for their expected compiler reasons.
- The ordinary local check and GitHub `Validate (no e2e)` gate pass.

## Boundaries

- Do not weaken or replace the compiler-backed boundary with source inspection.
- Do not special-case `jiff` or any other dependency.
- Do not make the gate order-dependent on an earlier fetch or compilation step.
- Do not rename the existing devtool Cargo-home environment contract or change
  unrelated Cargo cache policy.
