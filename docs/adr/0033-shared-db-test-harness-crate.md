# 0033. In-`storage` `test_support` Module for Both-Backend Test Parametrization

- Status: accepted
- Date: 2026-06-28 (amended same day — see "History")
- Deciders: Michael Alan Dorman

## Context

The both-backends test harness — the `Backend` enum, `TestEnv`, per-test
database provisioning, and the `backends` / `sqlite_only` / `postgres_only`
rstest templates — originally lived in `server/tests/helpers/mod.rs`, compiled
into the `server` integration-test crate, so only `server` tests could use it.

`storage`'s own `#[cfg(test)]` unit tests could not parametrize over backends:
they hardcode `SqlitePool::connect("sqlite::memory:")` and run SQLite-only,
leaving Postgres unexercised for backend-common contract behavior asserted
inside `storage` (e.g. `site_config` get/set semantics, registration-policy
resolution) — the same class of coverage hole ADR-0019's per-backend dialects
make possible, one crate below the integration suite. (The coverage instrumented
pass already runs the whole workspace under an ephemeral PostgreSQL with
`JAUNDER_PG_TEST_URL` set; Postgres is available to those tests — they just
never used it.)

The harness's entire job is to return `storage::AppState` (the DI composition
root, ADR-0016) on a chosen backend, so it is intrinsically _downstream_ of
`storage`.

## Decision

Host the harness **inside `storage`** as a feature-gated module,
`#[cfg(any(test, feature = "test-support"))] pub mod test_support`. `storage`'s
own tests reach it as `crate::test_support` (available via `cfg(test)`);
external test crates (`server`) reach it via `storage`'s `test-support` feature.
`server/tests/helpers` re-exports `storage::test_support::*` and keeps only the
genuinely server-specific helpers (`ensure_server_fns_registered`, websub
capturing).

### Why not a separate crate (the cycle)

A separate `db-test-harness` crate was tried first (it briefly existed, merged
in #125). Because it must return `storage::AppState`, it depends on `storage`;
and `storage`'s tests must dev-depend on _it_ — a dev-dependency cycle. Building
`storage`'s **own** unit-test target then produces **two distinct instances of
`storage`** (the `#[cfg(test)]` instance the tests live in, and the plain-lib
instance the harness links). `env.state.<handle>` is the lib instance; the
tests' crate-local functions/structs (`load_registration_policy`,
`perform_post_creation`, `PostCreation`, …) are the cfg-test instance — crossing
them yields `E0308: multiple different versions of crate storage`. Only tests
that stay entirely on the lib side (trait-method calls + `common` types, e.g.
`site_config`) compiled, which masked the problem in #125. **No separate crate
can avoid this**, because any crate returning `AppState` must depend on
`storage`. Hosting the harness _in_ `storage` makes `storage`'s tests and the
harness the same crate instance, eliminating the mismatch. #126 deleted
`db-test-harness` and moved the harness here.

The original objection to an in-`storage` harness — test scaffolding leaking
into release builds — is handled by the feature gate: `test_support` and its
optional deps (`tempfile`, `rstest_reuse`) compile only under `cfg(test)` or the
`test-support` feature, never in a normal build. This is the same family as
ADR-0026's `test-utils` feature.

### rstest template export

The `backends`/`sqlite_only`/`postgres_only` `#[template]`s carry
`rstest_reuse`'s `#[export]` (which adds `#[macro_export]` and a sibling
`pub use` alias), so they remain `#[apply]`-able both within `storage` and
cross-crate from `server` (via the `storage::test_support::…` re-export in
`helpers`). Apply sites need `rstest` + `rstest_reuse` in scope.

## Consequences

- One harness, hosted in `storage`, usable by `storage`'s in-file tests (same
  instance) and by `server` (via the feature) — closing Postgres coverage holes
  in `storage`'s own contract tests, not just the server suite.
- The conversions consuming it: #126 (`storage` contract tests), #54 (server
  `storage.rs`), #127 (rest of `server/tests`). Hosting was behavior-preserving
  — the full server suite stayed green on both backends across the move.
- The harness must stay feature/`cfg(test)`-gated; it must never be reachable
  from a normal `storage` build, or it would pull `tempfile`/`rstest_reuse` and
  the scaffolding into release artifacts.
- Complements ADR-0019 (per-backend divergence) and ADR-0021 (SQLite transaction
  discipline) by making both backends' behavior testable from the crate that
  defines them.

## History

Originally accepted (#125) as a **separate `db-test-harness` crate**. Amended
(#126) to a feature-gated module in `storage` after the dev-dependency cycle
above proved a separate crate cannot serve `storage`'s own in-file tests.
`db-test-harness` was removed in the same change.
