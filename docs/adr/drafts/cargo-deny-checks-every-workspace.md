# ADR-DRAFT: cargo-deny checks every Cargo workspace

- Status: proposed
- Date: 2026-11-27
- Issue: [#2163](https://github.com/jaunder-org/jaunder/issues/2163)

## Context

[CI checks advisories only on a dependency change](../drafts/ci-checks-advisories-on-dependency-change.md)
left the `tools/` workspace outside cargo-deny in every lane: the local gates,
both CI lanes and the daily scan ran `cargo deny check` from the root, which
resolves the product workspace alone. As of 2026-11-27, `tools/Cargo.lock` held
`zip` 9.0.4 (RUSTSEC-2026-0548), which Dependabot had reported since 2026-10-30
and none of our gates had. The tools workspace runs on developer machines and on
CI runners that hold tokens, so its dependencies matter as much as the
product's. Measured on `main`, `tools/` passed bans, licenses and sources under
the root `deny.toml`.

## Decision

cargo-deny checks both Cargo workspaces, the product and `tools/`, under the one
root `deny.toml`. The `tools-cargo-deny` static check runs
`cargo deny --manifest-path tools/Cargo.toml check` beside the product's
`cargo-deny` wherever that runs:

- the hermetic `static-code` check: bans, licenses and sources, offline;
- the local gates (`compile-checks`): the full check, advisories included;
- the CI host lane with `--advisories`: the full check.

A change to any `Cargo.toml` or `Cargo.lock`, in either workspace, or to
`deny.toml` is a dependency change for `dev.scope`. The daily `advisory-scan`
checks both workspaces and keeps one issue per advisory ID, listing each crate
version and workspace it was found in.

There is no separate `tools/deny.toml`: the tools workspace passes the root
policy, and two policy files could drift apart.

This narrows the earlier decision's exclusion of `tools/` and leaves the rest of
it in place.

## Consequences

An advisory, banned crate, disallowed license or disallowed source in `tools/`
is caught where the same problem in the product would be. A pull request that
changes either workspace's dependencies can fail on an advisory against a crate
it didn't touch, in either workspace.

The decision lands with `zip` still at 9.0.4, so that the first scan after it
files that advisory's issue from Actions. Until `zip` is upgraded, every local
gate and every dependency-changing pull request fails on RUSTSEC-2026-0548. Both
cargo-deny checks run last wherever they run, so that failure hides no other
check's verdict.
