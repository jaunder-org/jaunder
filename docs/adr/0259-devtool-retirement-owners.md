# ADR-0259: Devtool retirement: experiment endpoints retired, owners named, tools kept out of the product source

- Status: accepted
- Date: 2026-11-21
- Issue: [#1973](https://github.com/jaunder-org/jaunder/issues/1973)

## Context

[ADR-0246](0246-retire-xtask-devtool-endpoints.md) deferred the experiment-only
coverage emit/worker/aggregate endpoints. While they remained, it said full
devtool retirement was "not claimed or permitted" (ADR-0246:95-99). It also
assigned the in-derivation leaves to the existing coverage, doctests and
performance packages, and system-artifacts to a qualification target
(ADR-0246:72-74, :81-82).

[ADR-0251](0251-dev-runner-replaces-just.md) then dissolved `tools/devtool`. Its
leaves became `coverage-sandbox`, `doctests`, `perf-producer` and
`system-artifacts`. The experiment endpoints and their `benchmark-local` harness
went with it. No accepted record holds the decision to retire those endpoints.
The human confirmed it was intended on 2026-11-21 (#1973), which closed the
decision #1859 had left open.

Two Nix sources still tied the product to unrelated tools. As of 2026-11-21,
appending a comment to `tools/traced-run/src/main.rs` changed the derivation of
`packages.jaunder`:

- `system-artifacts` was built from the shared tools source, and every CSR
  bundle runs it.
- The product's own source admitted every `tools/` crate except a few host-tool
  implementations. That breadth was left over from when devtool shared this
  source.

[ADR-0258](0258-dev-runs-nix-built-host-tools.md) already listed
`system-artifacts` among the packages built from their own Cargo closure.

## Decision

- **The experiment-only coverage endpoints are retired.** This replaces
  ADR-0246's DEFERRED clause. `coverage emit --experiment`, `experiment-worker`,
  `experiment-aggregate` and the `benchmark-local` comparison harness are gone.
  A future partitioning experiment builds its own harness. Full devtool
  retirement is no longer blocked.
- **The in-derivation owners are the ones ADR-0251 delivered.** This replaces
  ADR-0246's owner clauses.
  - `coverage-sandbox` in `tools/coverage`: emit, validate-status, the wasm
    coverage lifecycle and diagnostic-build.
  - `doctests` in `tools/doctests`.
  - `perf-producer`: its own crate, not part of `tools/performance`.
  - `system-artifacts`: its own crate, not a qualification target.
- **No tools source outside the product's Cargo closure reaches a product
  derivation.**
  - `system-artifacts` is built as ADR-0258 describes: from its own Cargo path
    closure (`tools/system-artifacts`, `host`, `common`, `macros`) and the
    non-Cargo files `host` compiles in. This replaces, for that executable,
    ADR-0251's consequence that the in-derivation executables share the tools
    source.
  - From `tools/`, the product source admits only the crates the root
    workspace's Cargo path closure reaches. This replaces the per-crate
    host-tool exclusions.
  - Nix evaluation-time assertions enforce both boundaries.

## Consequences

- An edit to a tools crate outside the product's closure leaves
  `packages.jaunder`, the CSR bundle and every root-workspace check unchanged.
  Changes to the tools lockfile, manifests and toolchain still reach
  system-artifacts.
- `system-artifacts` gets its own dependencies-only build, like the other
  closure-built tools, instead of sharing the release tools build.
- coverage-sandbox, doctests and perf-producer still share the tools source.
  None of them reaches the product binary.
- A non-Cargo file that `host` starts compiling in must be admitted to the
  closure-built tools' source. If it is not, the build fails loudly; it never
  produces a stale binary.
