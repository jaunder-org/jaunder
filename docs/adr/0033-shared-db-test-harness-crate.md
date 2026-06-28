# 0033. Shared `db-test-harness` Crate for Both-Backend Test Parametrization

- Status: accepted
- Date: 2026-06-28
- Deciders: Michael Alan Dorman

## Context

The both-backends integration-test harness — the `Backend` enum, `TestEnv`, per-test
database provisioning, and the `backends` / `sqlite_only` / `postgres_only` rstest
templates — lived in `server/tests/helpers/mod.rs`, compiled into the `server`
integration-test crate. Only `server` tests could use it.

So the `storage` crate's own `#[cfg(test)]` unit tests could not parametrize over
backends: they hardcode `SqlitePool::connect("sqlite::memory:")` and run SQLite-only.
This left Postgres unexercised for backend-common contract behavior asserted inside
`storage` (e.g. `site_config` get/set semantics, registration-policy resolution) —
the same class of coverage hole ADR-0019's per-backend dialects make possible, but
one crate below the integration suite. The gap persists even though the coverage
instrumented pass already runs the *whole workspace* (`cargo llvm-cov nextest`, no
`-p` filter) under an ephemeral PostgreSQL with `JAUNDER_PG_TEST_URL` set
(`tools/devtool/src/pg.rs`): Postgres is available to those tests; they never use it.

The harness depends only on `storage::{open_database, open_existing_database,
AppState, DbConnectOptions}` and the `JAUNDER_PG_TEST_URL` env var — nothing from
`server`. `AppState` and the `open_*` functions are defined in `storage`.

## Decision

Relocate the harness into a dedicated workspace crate, **`db-test-harness`**, consumed
as a `[dev-dependencies]` entry by the test crates that parametrize over both backends —
`server` in this foundation change, and `storage` when #126 converts its tests. It owns
`Backend`,
`Backend::setup() -> TestEnv` (the `AppState`-level handle), the per-test SQLite/Postgres
provisioning (tempdir; clone-from-template via `JAUNDER_PG_TEST_URL` with per-test
drop on `Drop`), and the rstest templates.

`server`'s test support **builds on** this crate rather than owning the primitive:
`server/tests/helpers` re-exports `Backend`/`TestEnv`/templates/provisioning and keeps
only genuinely server-specific helpers (`ensure_server_fns_registered`, websub
capturing). Storage-level test support is the foundation; server-level support is a
thin layer over it.

The dependency direction is `db-test-harness -> storage` (normal) and, once `storage`
consumes it (#126), `storage -> db-test-harness` (**dev-dependency only**). Cargo permits
the resulting cycle because dev-dependencies do not participate in the normal build graph.
This was validated during #125 against a temporary `storage` wiring (a `site_config` test
ran on both backends through the crate), preserved as the #126 seed rather than committed.

### rstest template export

`rstest_reuse` `#[template]`s expand to macros, so cross-crate export was the one real
unknown. **Resolved:** annotate each `#[template]` with `rstest_reuse`'s `#[export]`
(which adds `#[macro_export]`), making the macro reachable at the crate root and
`#[apply]`-able from other crates; consumer crates need `rstest` + `rstest_reuse` in scope
at the apply site. A spike confirmed this end-to-end (a `storage` test ran both the
`::sqlite` and `::postgres` cases via the crate's `backends` template), so no per-crate
shim was needed. The provisioning core is shared regardless.

## Consequences

- One mechanism for "run this test on both backends," usable from any crate at or
  above `storage` — closing Postgres coverage holes in `storage`'s own tests, not
  just the server integration suite.
- The conversions themselves are deliberately out of scope of the extraction: #54
  (server `storage.rs`), #126 (storage Tier-2 tests), #127 (rest of `server/tests`)
  consume the crate. The extraction is behavior-preserving — no committed test-body
  changes; the full server suite stays green on both backends.
- The `storage` ⇄ `db-test-harness` dev-dependency cycle materializes when `storage`
  consumes the crate (#126); it is sound (validated in #125 — see Decision) but must be
  kept dev-only — the harness must never become a normal dependency of `storage`, or it
  would pull test scaffolding into release builds.
- Complements ADR-0019 (the dialects create per-backend divergence) and ADR-0021
  (SQLite transaction discipline) by making both backends' behavior testable from the
  crate that defines them.
- Distinct from ADR-0026's `test-utils` *feature* approach: a separate crate is
  chosen over a feature on `storage` to keep test scaffolding out of the normal build
  graph entirely and avoid feature-unification leakage.
