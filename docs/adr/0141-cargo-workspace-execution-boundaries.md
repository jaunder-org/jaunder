# ADR-0141: Cargo Workspace Execution Boundaries and Compensating Host Tests

- Status: accepted
- Date: 2026-08-14
- Issue: [#938](https://github.com/jaunder-org/jaunder/issues/938)
- Follow-up: [#1061](https://github.com/jaunder-org/jaunder/issues/1061)

## Context

The repository deliberately has more than one Cargo workspace. The root
workspace is the product workspace. `xtask` is a separate host-only workspace,
and `tools/` is a separate workspace containing auxiliary crates such as
`devtool`, `coverage`, and `doctests`. This shape exists because those crates
have different execution contracts from the product crates.

The root workspace excludes `xtask` from membership, so application coverage and
Nix test gates do not naturally execute `xtask` unit tests. The `tools/`
workspace owns helper binaries used by the development and gate machinery, not
product code. Some tool crates still run inside Nix derivations as part of
static checks, but their unit-test coverage is also outside the application
coverage target.

`xtask/src/steps/host_tests.rs` compensates for that boundary by adding explicit
host-side unit-test steps:

- `cargo test --manifest-path xtask/Cargo.toml`
- `cargo test --manifest-path tools/Cargo.toml`

A stale source comment overstated the invariant as though both `xtask` and all
of `tools/` were excluded from every Nix check.
[#1061](https://github.com/jaunder-org/jaunder/issues/1061) tracks correcting
that comment. The architecture decision here is narrower: the workspace split is
real, and explicit host tests compensate for unit suites that application
coverage/Nix test gates do not execute.

## Decision

The root Cargo workspace is the product workspace. It contains the product and
shared application crates: `client`, `common`, `csr`, `host`, `macros`,
`server`, `storage`, `test-support`, and `web`.

`xtask` remains outside the root workspace as a host-only orchestration crate.
It drives the development and CI gate from the live checkout and must not become
part of the product workspace just to make its tests discoverable by root
workspace commands. This preserves
[ADR-0028](0028-devtool-vs-xtask-boundary.md)'s execution litmus: `xtask` is the
host analyzer/orchestrator, never something Nix derivations invoke.

`tools/` remains a separate auxiliary workspace for gate/development helper
crates. It owns crates such as `devtool`, `coverage`, and `doctests`, and its
workspace boundary is a declaration of ownership and execution model rather than
a claim that no tool crate ever appears in a Nix derivation. This is compatible
with ADR-0028's `devtool` role: helper code that must run inside a Nix sandbox
belongs under `tools/`, not in `xtask`.

Because the root coverage and Nix test gates do not execute the unit suites for
these auxiliary workspaces, every reached `cargo xtask check` or
`cargo xtask validate` ladder run must include explicit host unit-test steps for
both manifests:

- `xtask/Cargo.toml`
- `tools/Cargo.toml`

These steps add test execution, not application coverage. Coverage remains about
the product workspace. If an auxiliary workspace grows behavior that must be
covered by a different gate, that gate must be added deliberately rather than
folding the crate into the product workspace by accident.

## Consequences

The workspace split is an architectural boundary. Moving `xtask` or `tools/`
into the root workspace, removing the explicit host-test steps, or changing what
the root product workspace owns requires design review.

`host_tests` is the compensating gate for uncovered auxiliary unit suites. Its
comments and acceptance criteria should describe that exact rationale: root
application coverage and Nix test gates do not run these unit suites. They must
not claim that `tools/` code is absent from all Nix checks.

The tradeoff is duplicated-looking test commands in the gate. That is preferable
to hiding ownership boundaries inside one wide workspace or silently skipping
host-only/unit helper behavior.

Rejected alternatives:

- Adding `xtask` to the root workspace. That would mix product crates with the
  host-only gate driver and make derivation source boundaries harder to reason
  about.
- Collapsing `tools/` into the root workspace. Auxiliary gate helper crates have
  a different lifecycle from product crates.
- Relying on clippy/static checks as a substitute for auxiliary unit tests.
  Static checks can compile paths without exercising behavior.
- Widening the statement to "`tools/` is excluded from every Nix check." That is
  false for the current static-check source and is tracked separately by #1061.
