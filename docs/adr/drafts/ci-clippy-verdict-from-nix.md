# ADR-DRAFT: CI takes its Clippy verdict from Nix

- Status: proposed
- Date: 2026-11-20
- Issue: [#2010](https://github.com/jaunder-org/jaunder/issues/2010)

## Context

Every CI run linted the six Clippy units (`product-core-clippy`,
`web-server-clippy`, `web-no-server-clippy`, `client-host-clippy`,
`wasm-clippy`, `tools-clippy`) twice:

- **Host lane:** `dev ci-validate-host` ran them directly on the runner. Nothing
  from that run is cached, so it ran on every run.
- **Hermetic lane:** `dev ci-validate-hermetic` builds `static-code`, which
  requires `clippy-aggregate` and its six unit derivations. Each derivation runs
  the same `dev static <unit>` arguments with `--sandbox`.

The two copies use the same arguments, pinned toolchain and lock file. In PR run
44837736998 (2026-11-20) the host copy took 621 s of the host lane's 935 s, and
the Nix copy took about 700 s. On merge-group and `main` runs the Nix verdict
was substituted from Cachix
([substitutable static verdicts](../0252-substitutable-static-verdicts.md)), but
the host copy still ran. In merge-group run 44840288640 the host lane (21.6 min)
outlasted every other job.

The #1864 verification-family disposition kept the target-specific lint units
and host Clippy as required. It deferred further sharing "unless a concrete
maintenance defect, consumer need or attributable workflow cost justifies a
separately scoped decision". Linting the same units twice on every run is that
attributable cost.

## Decision

CI takes its Clippy verdict only from the hermetic lane's `static-code` check,
through `clippy-aggregate`. `dev ci-validate-host` runs the host check groups
without the six Clippy units. It still runs `tsc` and `cargo-deny`. This narrows
the `host` lane's ownership of "the verify-only host/static gate" in
[distributed CI lane topology](../0203-distribute-ci-validation-and-firefox-e2e-lanes.md):
Clippy's CI verdict moves to `hermetic`.

Host Clippy stays required everywhere outside CI: `dev check`,
`dev check-no-test`, the pre-commit and pre-push hooks, `dev compile-checks` and
local `dev validate-no-e2e`. Local `dev validate-no-e2e` therefore runs both
copies and is a superset of the CI lanes.

We rejected the alternative of keeping host Clippy and deleting the Nix copy.
That would remove more code, but it saves runner time only on cache-miss runs
and leaves the host lane as the slowest job.

## Consequences

- Every CI run saves about 10 runner-minutes. Merge-queue latency drops by at
  most about 2 min, because the e2e chain then bounds it. `main`-push and PR
  wall clock do not change.
- A CI Clippy failure appears in the hermetic lane, against the `clippySource`
  filtered copy. A file the filter omits but a crate needs fails that build; it
  cannot pass silently.
- CI no longer runs `cargo` on the product workspace outside Nix. This changes
  no caching: host Clippy built only lint metadata in the runner's discarded
  `target/`. CI no longer covers host-only build paths. One example is the
  git-aware branch of `test-support/build.rs`, which the Nix copy skips because
  it has no `.git`; the sccache/host-environment handling is another. The
  pre-commit and pre-push hooks still exercise those paths, though `--no-verify`
  bypasses them.
- A PR that changes Rust still spends about 12 min on Nix Clippy. Making those
  leaves cheaper is separate work.
