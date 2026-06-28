# Design — Issue #125: shared `db-test-harness` crate for both-backend test parametrization

- Date: 2026-06-28
- Issue: [#125](https://github.com/jaunder-org/jaunder/issues/125)
- Status: approved (brainstorming)

## Context / Problem

The both-backends integration-test harness — the `Backend` enum, `TestEnv`, per-test
database provisioning, and the `backends` / `sqlite_only` / `postgres_only` rstest
templates — lives in `server/tests/helpers/mod.rs`. Because it is a module compiled
*into the server integration-test crate*, only `server` tests can use it.

Consequently the `storage` crate's own `#[cfg(test)]` unit tests cannot parametrize
over backends. They hardcode `SqlitePool::connect("sqlite::memory:")` and run
SQLite-only — even though the coverage instrumented pass already runs the **whole
workspace** under an ephemeral PostgreSQL: `cargo llvm-cov nextest` with no `-p`
filter, with `JAUNDER_PG_TEST_URL` set by `tools/devtool/src/pg.rs::with_ephemeral`
(`tools/devtool/src/coverage/emit.rs:72`). Postgres is *available* to those tests;
they simply never use it.

The harness depends only on `storage::{open_database, open_existing_database,
AppState, DbConnectOptions}` plus the `JAUNDER_PG_TEST_URL` env var — **nothing from
`server`**. `AppState` and the `open_*` functions live in `storage`. So the harness
can be relocated to a shared crate that both `storage` and `server` build on,
without inverting any dependency.

This issue is the **foundation**: it relocates the harness and proves its shape. The
actual test conversions are owned by downstream issues — #54 (server `storage.rs`),
#126 (storage-internal "Tier-2" tests), #127 (rest of `server/tests`).

## Goal

Extract a dedicated `db-test-harness` workspace crate that is the single
both-backends parametrization mechanism, consumed as a dev-dependency by both
`storage` and `server`. `server`'s test support builds on the same primitive that
`storage`'s tests use.

## Design

### New crate: `db-test-harness`

A workspace member, added as a `[dev-dependencies]` entry to both `storage` and
`server`. It owns:

- **`Backend`** enum (`Sqlite` | `Postgres`).
- **`Backend::setup() -> TestEnv`** and **`TestEnv { state: Arc<AppState>, base: TestBase }`**
  (the `AppState`-level handle).
- **Provisioning internals** moved verbatim from `server/tests/helpers/mod.rs`:
  `TestBase` (+ its `Drop` that drops the per-test Postgres clone), `sqlite_url`,
  `template_postgres_url`, `unique_postgres_url`, `drop_test_database`,
  `recorded_postgres_url`, `PG_URL_FILE`, and the `JAUNDER_PG_TEST_URL` gating
  (`postgres_available()` / equivalent).
- **rstest templates**: `backends`, `sqlite_only`, `postgres_only`.

Dependencies: `storage` (for `open_database`, `open_existing_database`, `AppState`,
`DbConnectOptions`), `sqlx`, `tempfile`, `chrono`, `rstest`, `rstest_reuse`,
`tokio`, plus the mailer/`common` types `AppState` construction needs.

### What stays in `server/tests/helpers`

The genuinely server-specific helpers remain:

- `ensure_server_fns_registered()` (registers `web::*` server fns — a `server`/`web`
  concern).
- `CapturingWebSubClient` / the `websub_capturing` module.

`helpers` re-exports `Backend`, `TestEnv`, the provisioning helpers, and the
templates from `db-test-harness`, so existing `use crate::helpers::…` sites and
`#[apply(backends)]` usages across `server/tests/**` keep compiling with minimal
churn.

### Task ordering

1. **Spike — `rstest_reuse` cross-crate templates.** Confirm a `#[template]` defined
   in `db-test-harness` can be `#[apply]`-ed in a *consumer* crate's tests
   (`rstest_reuse` templates expand to macros; cross-crate export is the known risk).
   - **Clean** → export templates from the crate and use directly.
   - **Not clean** → fall back to a tiny per-crate shim: re-declare/re-export the
     templates within each consumer, or expose `fn provision(Backend)` and use plain
     `#[rstest]` + `#[case]` at call sites. Either way the provisioning core stays
     shared; only the parametrization sugar is per-crate.
2. **Build the crate**; move the provisioning internals + templates out of `helpers`;
   rewire `helpers` to re-export.
3. **Throwaway validation** (see below).

### Throwaway validation gate

To prove the foundation's *shape* against real call sites before locking the API:
minimally convert **one** `storage` Tier-2 test (e.g. a `site_config.rs` round-trip)
**and** **one** `server` test to the new harness; confirm under the coverage PG pass
that both the `::sqlite` and `::postgres` cases run. **Revert these conversions before
committing** — they are proof, not deliverable. The actual conversions belong to #54
and #126.

### Behavior preservation

The committed change is purely additive plus a mechanical relocation. The entire
existing `server` suite stays green on both backends with **no committed test-body
changes**. `cargo xtask validate` green is the gate.

## Risks

- **`rstest_reuse` cross-crate export** — the one real unknown; the spike resolves it
  before any bulk move, and the per-crate-shim fallback de-risks it.
- **Cargo feature unification / dev-dep cycle** — `db-test-harness` depends on
  `storage`; `storage` dev-depends on `db-test-harness`. This is a *dev-dependency*
  cycle, which Cargo permits (dev-deps don't participate in the normal build graph).
  Verify it builds and that `cargo test -p storage` resolves.

## Testing / verification

- `cargo xtask validate` green: the relocated harness compiles and the full server
  suite passes on both backends.
- The throwaway validation demonstrates cross-crate `#[apply]` from both `storage`
  and `server` (then reverted).

## ADR

Records the durable architectural decision — *backend test-parametrization is a
shared crate that both `storage` and `server` build on; server-level test support
builds on storage-level support* — as **ADR-0033**, with a row added to the ADR
table in `docs/README.md`.

## Out of scope (downstream issues)

- **#54** — convert `server/tests/storage/storage.rs` SQLite-only tests to
  `#[apply(backends)]` + the uniform-pattern guard.
- **#126** — parametrize storage-crate-internal (Tier-2) common tests over both
  backends; reconcile with ADR-0019.
- **#127** — extend conversion + guard across the rest of `server/tests`.

## Acceptance

- `db-test-harness` crate exists; `Backend`, provisioning, templates, and `TestEnv`
  are moved out of `server/tests/helpers`.
- `server/tests/helpers` builds on it; the full server suite is green on both
  backends with no committed test-body changes.
- Cross-crate template usage is demonstrated (in the throwaway validation) from both
  `storage` and `server`.
- ADR-0033 added and listed in `docs/README.md`.

## Implementation note (as landed)

This foundation change adds `db-test-harness` as a dev-dependency of **`server` only**.
`storage` does **not** gain the dependency here: the throwaway storage-side validation
(a `site_config` round-trip running on both backends through `Backend::setup`) was
**stashed as the #126 seed** rather than committed, so committing the bare `storage`
dev-dep would have left it unused. The `storage ⇄ db-test-harness` dev-dependency cycle
therefore materializes when #126 lands; it was build- and run-validated during #125 via
that temporary wiring. The `rstest_reuse` cross-crate question resolved to the primary
path (`#[export]` → `#[macro_export]`); no per-crate shim was needed.
