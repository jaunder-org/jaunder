# ADR-0026: Test-only fault-injection hooks behind a `test-utils` feature, not `#[cfg(test)]`

- Status: accepted
- Date: 2026-06-26
- Deciders: Michael Alan Dorman

## Context

Some `storage` functions have **test-only fault-injection hooks** so their
error-handling branches can be covered. For example, `helpers::hash_password`
returns a forced error when given the sentinel password
`"force-hash-error-for-test-coverage"`, letting a test exercise the `Internal`
(hash-failed) path and — after ADR-0022 — the validate-before-hash ordering.

These hooks were gated on `#[cfg(test)]`. That attribute compiles **only in the
owning crate's own test build**. When `storage` is consumed as a normal
dependency — as it is by the `server` integration tests (`server/tests/…`) — the
`cfg(test)` hook is _absent_, so those hooks fire only in `storage`'s in-crate
unit tests.

The integration suite is where backend parity is tested: a storage behavior is
written once and annotated `#[apply(backends)]`, expanding into a SQLite case
and a Postgres case (ADR-0019; `server/tests/helpers/mod.rs`). Because the
fault-injection hooks were invisible there, any error path reachable _only_
through injection (e.g. hash failure) could be tested only as a **single-store**
SQLite in-crate test — leaving the Postgres implementation of that path
uncovered. That is the coverage hole tracked by #54.

## Decision

Gate test-only fault-injection hooks on
**`#[cfg(any(test, feature = "test-utils"))]`** rather than `#[cfg(test)]`.

`test-utils` is a marker feature in `storage/Cargo.toml`; downstream crates
enable it from their `[dev-dependencies]` (`server` already does). The hook
therefore compiles in exactly two situations:

- `storage`'s own unit tests (`cfg(test)`), and
- any build that enables `test-utils` — i.e. the integration/coverage test
  build, where `server`'s dev-dependency turns it on.

It is **absent from production**: a release build enables neither `test` nor
`test-utils` (dev-dependencies are not part of the production dependency graph),
so the sentinel check is compiled out.

## Consequences

- Error paths reachable only via fault injection can now be covered by
  **dual-backend** integration tests, eliminating the need for single-store
  in-crate tests for them. First applied to `hash_password`'s hook for
  `confirm_password_reset` (#60); the remaining single-store storage tests are
  converted under #54.
- The hooks stay out of production builds by construction — the safety property
  that made `#[cfg(test)]` attractive is preserved, just widened to the
  test/coverage build.
- New fault-injection hooks follow this pattern (gate on
  `any(test, feature = "test-utils")`), not bare `#[cfg(test)]`, so they remain
  usable from cross-crate integration tests.
- Builds on ADR-0019 (dual-backend parity) and the integration-suite test
  conventions in `CONTRIBUTING.md`.
