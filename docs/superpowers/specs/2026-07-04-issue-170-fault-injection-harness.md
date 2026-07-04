# Spec — #170: backend-generic fault-injection harness for storage-error-propagation tests

- Issue: jaunder-org/jaunder#170
- Date: 2026-07-04
- Milestone: 5 (Backend-parity test coverage)
- Status: draft (awaiting approval)

## Problem

Two `server/tests/web/web_backup.rs` tests assert **backend-agnostic** behavior
— a web handler returns `500 INTERNAL_SERVER_ERROR` when `authenticate()` hits a
storage error (the `Err(non-Unauthorized)` branch of the
storage-error-propagation path):

- `backup_warning_visible_propagates_storage_error_during_auth`
- `current_user_is_operator_propagates_storage_error_during_auth`

They induce the fault with a **SQLite-specific** mechanism:
`test_sqlite_state_with_pool(&base)` returns the raw `SqlitePool`, which the
test `pool.close()`s so the next query fails. Because the helper and the close
are SQLite-only, both tests are tagged `#[apply(sqlite_only)]` (suite-wide
guard, #127) with a `// reason:` pointing at this issue. But the behavior under
test is **not** SQLite-specific and should run on Postgres too — otherwise
Postgres has a backend-parity coverage hole (parity is achieved by _adding_ the
Postgres case, never by leaving one backend untested).

The root cause is an asymmetry: the connection pool is **not reachable from
`AppState`**, so fault injection was only ever wired for SQLite
(`test_sqlite_state_with_pool`), while Postgres raw access uses a _different_
workaround (`recorded_postgres_url` / `PG_URL_FILE`, which reconnects a fresh
pool).

## Decision / design

Make the pool a first-class, backend-agnostic part of the test environment, so
fault injection works identically on both backends.

### `CloseablePool`

A new enum in `storage::test_support`:

```rust
pub enum CloseablePool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}
```

- `pub async fn close(&self)` — closes the underlying pool. The mechanism is
  backend-agnostic: `sqlx::Pool::close()` marks the pool closed, and the next
  query through any storage handle backed by that pool returns
  `sqlx::Error::PoolClosed`, which the storage layer maps to its `Internal`
  error variant. Identical on SQLite and Postgres.
- Accessor(s) for raw-SQL use (e.g. `sqlite_pool()`/`postgres_pool()` or a typed
  getter) so the migrated raw-SQL sites (see below) can seed/inspect through the
  live pool instead of reconnecting.

### Pool lives in `TestBase`

`CloseablePool` is stored in the existing **`TestBase`** struct (the holder
behind `TestEnv.base`), whose fields are **private** — so none of the ~330
`let TestEnv { state, base } = …` destructures across the suite break. `TestEnv`
keeps its exact `{ state, base }` shape.

`Backend::setup()` retains the pool it opens and stores it in `TestBase`. To
thread the pool out of the open path (which today returns `Arc<AppState>` only),
add a pool-returning variant —
`open_database_with_pool(opts) -> sqlx::Result<(Arc<AppState>, CloseablePool)>`
— and make the existing `open_database`/`open_existing_database` thin wrappers
that drop the pool. The pool stored in `TestBase` is a clone of the same `sqlx`
pool the storages hold (pools are `Arc`-shared), so closing it faults every
storage handle in `AppState`.

Fault-injection tests then do:

```rust
let TestEnv { state, base } = backend.setup().await;
// … create a session …
base.close_pool().await;           // faults the shared pool for both backends
// … assert 500 / Internal …
```

### `AppState` stays

`AppState` is **not** removed. It remains the ergonomic DI construction bundle
(ADR-0016): production fans it into Leptos context via
`provide_app_state_contexts` (web server functions consume granular
`Arc<dyn XStorage>`), and CLI commands (`server/src/commands.rs`) consume its
fields directly. Tests keep using `env.state` — a test is a driver, not a
capability-scoped consumer, so the bundle mirrors the real
`open_database → AppState → create_router` construction seam.

### In scope

1. Add `CloseablePool` + store it in `TestBase`; `Backend::setup()` retains the
   pool.
2. Convert the two `web_backup.rs` tests to `#[apply(backends)]`, closing the
   pool via `base`; drop their `#[apply(sqlite_only)]` + `// reason:` markers.
3. Remove `test_sqlite_state_with_pool`, migrating **all six** of its callers:
   - the two `web_backup.rs` fault-injection tests → `Backend::setup()` +
     `base.close_pool()` (item 2, now dual-backend);
   - **four `#[cfg(test)]` smoke tests in `test-support/src/lib.rs`** (`:184`,
     `:213`, `:238`, `:272`) that discard the pool (`let (state, _pool) = …`) →
     replace with `Backend::Sqlite.setup().await.state`. These stay
     **SQLite-only by design** (their module docs state the dual-backend path is
     proven by the e2e matrix; they smoke the logic on SQLite for speed) and are
     not under the `server/tests` `test-backend-pattern` guard, so no `#[apply]`
     tagging applies.
4. Rename `make_app_state` → `make_sqlite_app_state` (symmetry with
   `make_postgres_app_state`).
5. Migrate the raw-SQL Postgres sites that need only a `Pool<DB>` (not a URL
   string) from `recorded_postgres_url` + reconnect to the live pool in
   `TestBase`. `recorded_postgres_url`/`PG_URL_FILE` stay for the sites that
   genuinely need the URL (db-name parsing, `DbConnectOptions` construction —
   e.g. `pg_teardown.rs:50`, `storage.rs:620`). Candidate pool-only sites to
   migrate: `storage.rs:193`, `:2577`, `:2607`, `:7171` (each
   `PgPool::connect(&recorded_postgres_url(&env.base))` for seed/inspect). The
   plan enumerates the exact final migrate/keep split.

### How #135 reuses this (no separate standalone helper)

The issue proposed a standalone
`state_with_closeable_pool(backend, base) -> (Arc<AppState>, CloseablePool)` so
#135's storage-crate in-file tests could reuse it. That standalone constructor
is **superseded** by the `TestBase` design: storage's own `#[cfg(test)]` tests
already drive `Backend::setup()` (e.g. `storage/src/site_config.rs`,
`storage/src/post_service.rs` use `#[apply(backends)]` + `Backend::setup()`), so
#135's ~30 closed-pool tests consume the **same** API —
`let TestEnv { state, base } = backend.setup().await; base.close_pool().await; state.sessions.authenticate(…)`
— with no extra surface. `CloseablePool` stays a `pub` type; `base.close_pool()`
is the single reuse seam.

### Non-goals / deferred (separable concerns — filed as the plan's first task)

- **Unify `make_sqlite_app_state`/`make_postgres_app_state`** into one generic
  `make_app_state<DB>(pool)`. Today the two builders differ in every field by
  backend prefix (`SqliteXStorage` vs `PostgresXStorage`); once expressed as the
  ADR-0019 generic `XStore<DB>` form, only the `atomic` field
  (`SqliteAtomicOps`/ `PostgresAtomicOps`, divergent tx discipline per ADR-0021)
  stays backend-specific, needing a small hook. Touches the ADR-0019
  generic-store bound surface; filed as a separate test-infra/refactor issue.
- **The storage-layer closed-pool conversions (#135)** — #135 consumes this
  harness to convert the ~30 SQLite in-file closed-pool tests to dual-backend.
  Out of scope here; #170 is the prerequisite.

## Acceptance criteria

Each is observable so ship-time conformance can tell delivered from not:

- **AC1** `CloseablePool` exists in `storage::test_support` with an
  `async close(&self)` and raw-pool accessors, re-exported through
  `server/tests/helpers`.
- **AC2** `Backend::setup()` returns a `TestEnv` whose `base` holds a
  `CloseablePool` for **both** backends; `TestEnv`'s `{ state, base }`
  destructure shape is unchanged (the full suite still compiles without edits to
  existing destructures).
- **AC3** `backup_warning_visible_propagates_storage_error_during_auth` and
  `current_user_is_operator_propagates_storage_error_during_auth` are
  `#[apply(backends)]` and assert `500 INTERNAL_SERVER_ERROR` on **both** SQLite
  and Postgres (verified: each expands to `::case_1_sqlite` and
  `::case_2_postgres`, both pass under
  `devtool pg run -- cargo nextest run -p jaunder`).
- **AC4** `test_sqlite_state_with_pool` no longer exists;
  `rg -n test_sqlite_state_with_pool` finds no references in any source crate
  (all six callers — 2 in `web_backup.rs`, 4 in `test-support/src/lib.rs` —
  migrated). Generated/historical hits (`crap-manifest.json`, `docs/archive/*`)
  don't count.
- **AC5** No `make_app_state` (unqualified) remains; the SQLite builder is
  `make_sqlite_app_state`.
- **AC6** The enumerated pool-only raw-SQL Postgres sites (`storage.rs:193`,
  `:2577`, `:2607`, `:7171`) use `TestBase`'s live pool instead of
  `PgPool::connect(&recorded_postgres_url(…))`. After migration, every remaining
  `recorded_postgres_url` caller is one that consumes the URL _string_ (db-name
  parse / `DbConnectOptions` build): `pg_teardown.rs:50`, `storage.rs:620`, and
  the `helpers`/`storage.rs` re-export/import lines. (Observable: the plan lists
  the final migrate/keep split; a reviewer confirms each remaining caller parses
  a URL rather than acquiring a pool.)
- **AC7** `cargo xtask validate --no-e2e` is green (static + clippy
  `-D warnings` + coverage ratchet), and the storage/web integration suites pass
  on both backends. No new uncovered lines outside the committed baseline.

## Risks

- **PG pool-close semantics.** Closing a `PgPool` must make the _next_ storage
  call in the same request error (not reconnect/hang). Mitigated by
  `sqlx::Pool::close()` being backend-generic (`PoolClosed` on next acquire);
  AC3 verifies the Postgres case actually yields 500. If a converted test flakes
  on Postgres (pool drain timing), the fix is to await `close()` fully before
  issuing the request (already the shape above).
- **Drop-order vs. per-test PG teardown.** `TestBase::Drop` runs
  `drop_test_database` in its body — which executes _before_ the struct's fields
  drop. Storing a `CloseablePool` (an `Arc`-clone of the live pool) in
  `TestBase` means that for a normal test that never calls `close_pool()`, the
  per-test pool still holds connections when `DROP DATABASE` fires. This is safe
  **only** because `drop_test_database` uses `DROP DATABASE … WITH (FORCE)`
  (`test_support.rs:314`), which terminates those backends. The plan must (a)
  not reintroduce a non-FORCE drop, and (b) verify no PG test regresses on
  teardown. If a future change drops `WITH (FORCE)`, `TestBase::Drop` must first
  drop/close its pool (the scoped-thread + current-thread-runtime pattern
  already in `drop_test_database` is available).
- **Coverage baseline drift.** Removing `test_sqlite_state_with_pool`, migrating
  its 6 callers, and adding the PG case will move lines; may require a
  coverage-baseline reanchor (separate, approved step) — AC7 gates it.
