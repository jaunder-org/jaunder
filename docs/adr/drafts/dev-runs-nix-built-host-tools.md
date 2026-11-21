# ADR-DRAFT: `dev` runs Nix-built host tools

- Status: proposed
- Date: 2026-11-20
- Issue: [#1968](https://github.com/jaunder-org/jaunder/issues/1968)

## Context

[ADR-0225](../0225-nix-cached-host-tooling.md) packaged host tooling through Nix
and ran it on the host, so unchanged tool inputs reuse one output across
checkouts. [ADR-0251](../0251-dev-runner-replaces-just.md) then had `dev` build
each tools-workspace binary from the checkout with locked Cargo into
`tools/target`. Only the E2E verifier ran a cached Nix output
(`packages.host-proof`).

In CI that meant every job compiled the tools it ran. The Actions cache meant to
prevent this was keyed only on lockfiles. It was saved on a key miss by
whichever job finished first, normally one that runs no Cargo, so it never held
what the validation lanes build. On a warm main run, tool compiles cost 27–56 s
per validation lane. Seventeen E2E jobs restored the 1 GB cache without using
it. Per-lane shells without Cargo
([#2008](https://github.com/jaunder-org/jaunder/issues/2008)) need the tools
from somewhere other than the checkout's Cargo.

## Decision

`dev` gets every Rust host tool by building its flake package with
`nix build .#<package>` and running `<out>/bin/<binary>`. It builds each package
once per process, as the traced step `<package>-build`. A failed build fails the
step, and there is no Cargo fallback. This is the same locally and in CI. The
packages:

- `repository-policy`: source-contracts, repository-docs, org-timezone-manifest,
  scoped-leptosfmt
- `host-proof`: every host-proof binary
- `coverage-check` (the `coverage` crate with its `host` feature)
- `perf-data`
- `pr-observer`
- `system-artifacts`

Each host-tool package takes a dependencies-only build as its Cargo artifacts.
That build changes only with the lockfile, the toolchain or the features. A
source edit inside the package's closure (`common/`, `host/` or the tool)
recompiles the workspace crates alone.

The flake source is the Git working tree. An edited tool therefore always runs
as edited. A file Git does not track is not in the source:

- `repository-policy` keeps its compiled refusal while an admitted input is
  untracked, per ADR-0225.
- The other host tools do not refuse. A missing module fails their build, and an
  untracked data file is absent from it, as for every Nix check of the working
  tree. This narrows ADR-0225's untracked-input refusal to repository-policy.

The setup-ci `tools/target` cache is removed. Jobs that still run host Cargo
(`validate-host` and the scheduled mutants scan) opt in to a cache of the Cargo
registry and Git checkouts only, keyed on both lockfiles.

## Consequences

- A CI job whose tool packages are unchanged substitutes them from Cachix and
  compiles no tool. A job whose change touches a tool's closure rebuilds that
  package once per job, workspace crates only.
- Each tool package costs `dev` one flake evaluation per process, about 5 s warm
  on a clean tree and more on a dirty one, against about 0.5 s for a no-op Cargo
  build.
- Editing a tool locally means a Nix rebuild of its package before `dev` runs
  it. The tool's own `cargo test` and `cargo clippy` stay the inner loop, and
  the host lane's `tools-clippy` and tools tests still compile the tools
  workspace with Cargo.
- Host-tool package names must stay clear of setup-ci's Cachix `pushFilter`, or
  they are never substituted.
- This amends ADR-0251's statement that host workflows reach tools through Cargo
  from the checkout. It also amends the
  [ADR-0254](../0254-dev-trace-step-accounting.md) rule that a tool is built
  with Cargo as `<bin>-build` and run from `tools/target`. Tools are still built
  before they run, now as `<package>-build`. ADR-0225's host-only execution and
  packaging boundaries stand.
