# Spec — issue #43: leaking per-test PostgreSQL databases in the test helpers

**Issue:** jaunder-org/jaunder#43 (milestone 1 "Verify-gate hardening";
`tooling`, `test-infra`). Bounded residual of #28.

## Problem

Two test helpers in `storage/src/test_support.rs` issue `CREATE DATABASE` and
return a bare `DbConnectOptions` with **no RAII owner** tracking the created
database, so callers that don't route through `TestBase` leak a database per
invocation:

- **`unique_postgres_url()`** (`test_support.rs:437`) — a bare
  `CREATE DATABASE <unique> OWNER <role>` (empty, no template). Leaking callers:
  - `server/tests/misc/commands.rs::storage_args` (Postgres arm; ~25
    `#[apply(backends)]`/`#[apply(postgres_only)]` tests).
  - `server/tests/misc/backup_interop.rs::postgres_storage_args` (2 tests).
  - `server/tests/storage/storage.rs` — 3 `#[apply(postgres_only)]` tests (lines
    862, 872, 885), each `let url = unique_postgres_url().await;` then
    `open_database`/`open_existing_database(&url)`.
- **`template_postgres_url()`** (`test_support.rs:535`) — a
  `CREATE DATABASE <unique> … TEMPLATE <migrated>` clone. #28 gave this a
  teardown owner **only** on the `Backend::setup()` path (via `TestBase`). Its
  two _direct_ callers bypass `TestBase` and leak:
  - `server/tests/storage/storage.rs:72` — `open_pg_pool()` mints a fresh clone
    per call; its sole caller is `lookup_names` (line 85).
  - `server/tests/storage/storage.rs:897` —
    `authenticate_with_corrupted_hash_returns_internal_error` mints a clone
    directly.

Every leak is bounded and non-suite-scaling (~one DB per each of a fixed set of
tests, constant regardless of suite size), consciously scoped out of #28. #43's
acceptance criterion "no remaining `CREATE DATABASE` path in the test helpers
leaks" covers **both** helpers, so both are addressed here.

The proven teardown mechanism is the free function
`drop_test_database(&db_name)` (`test_support.rs:360`,
`DROP DATABASE … WITH (FORCE)` on a joined scoped thread with its own
current-thread runtime, 10 s timeout, panic-free) that `TestBase::drop`
(`test_support.rs:142`) already calls.

## Resolved design

### Decision 1 — one teardown primitive: `PostgresDbGuard`

Introduce a single public RAII type in `storage/src/test_support.rs`,
re-exported through `server/tests/helpers/mod.rs` (per ADR-0033), that owns a
created database's name and drops it on `Drop`:

```rust
pub struct PostgresDbGuard { db_name: String }
impl Drop for PostgresDbGuard {
    fn drop(&mut self) { drop_test_database(&self.db_name); }
}
```

It reuses `drop_test_database` verbatim — no second `DROP DATABASE`
implementation — so teardown semantics are identical to #28.

### Decision 2 — both `CREATE DATABASE` helpers return the guard with the URL

Both helpers change signature so a caller cannot obtain the connect URL without
also receiving its teardown owner, making the leak structurally impossible:

```rust
pub async fn unique_postgres_url()   -> (DbConnectOptions, PostgresDbGuard)
pub async fn template_postgres_url() -> (DbConnectOptions, PostgresDbGuard)
```

Each builds the guard from the unique name it already mints locally.

### Decision 3 — `TestBase` composes the guard (unify the mechanism)

`TestBase` stops carrying a bare name + hand-written `Drop`; it holds a
`PostgresDbGuard` that drops itself:

```rust
pub struct TestBase {
    dir:  TempDir,
    pool: CloseablePool,
    _pg:  Option<PostgresDbGuard>,   // None on SQLite; drops itself on Postgres
}
// impl Drop for TestBase  -> DELETED (the field's Drop fires automatically)
```

`_pg` is declared **after** `pool` so the pool drops before the database is
dropped (field drop runs in declaration order); `DROP … WITH (FORCE)` makes the
order non-critical, but this matches the intent. `TestBase::sqlite` sets
`_pg: None`; `TestBase::postgres(dir, guard, pool)` takes the guard directly.
`Backend::setup()` (`test_support.rs:194`) becomes
`let (url, guard) = template_postgres_url().await;` — it opens its pool from
`url` and hands `guard` to `TestBase::postgres`, instead of re-deriving the name
from `options.get_database()`. Result: one "self-dropping postgres test DB"
type, used standalone by direct callers and composed inside `TestBase`.

### Decision 4 — every direct caller holds the guard (explicit `_pg` bindings)

Explicit bindings (over a `Deref` wrapper that hides the guard) keep the RAII
lifetime **visible** so a future refactor won't silently drop it early.

`unique_postgres_url()` callers:

- `commands.rs::storage_args(backend, base) -> (StorageArgs, Option<PostgresDbGuard>)`
  — `None` on the SQLite arm, `Some(guard)` on Postgres; ~25 call sites become
  `let (args, _pg) = storage_args(...).await;`.
- `backup_interop.rs::postgres_storage_args(base, name) -> (StorageArgs, PostgresDbGuard)`
  — always Postgres, so unconditional; its 2 tests bind
  `_pg_source`/`_pg_target`.
- `storage.rs:862/872/885` — `let (url, _pg) = unique_postgres_url().await;`.

`template_postgres_url()` callers:

- `Backend::setup()` — as in Decision 3.
- `storage.rs:72` `open_pg_pool() -> (PgPool, PostgresDbGuard)`; its caller
  `lookup_names` (line 85) binds `let (pool, _pg) = open_pg_pool().await;` and
  queries `&pool` — the guard lives until the query completes.
- `storage.rs:897` — `let (url, _pg) = template_postgres_url().await;` then the
  existing `let DbConnectOptions::Postgres { options, .. } = url else { … };`.

Correctly **excluded** (mint no database via these helpers, so unchanged):

- `uninitialized_storage_args` (`commands.rs`) — uses
  `nonexistent_postgres_url()`, which only parses a URL and issues no
  `CREATE DATABASE`.
- The inline `StorageArgs { … }` block in
  `commands.rs::cmd_init_fails_on_invalid_path` (line 187) — reuses an existing
  `args.db`.
- The inline `StorageArgs { … }` block in
  `commands.rs::cmd_create_pg_db_provisions_role_and_database` (line 263) —
  mints a DB via `cmd_create_pg_db` (not these helpers) and already drops it
  manually (lines 269–276).

### Decision 5 — regression test asserting teardown

Add a `postgres_testing_enabled()`-gated test in
`server/tests/misc/pg_teardown.rs` (which already holds the analogous
`per_test_database_is_dropped_on_teardown` for the `TestBase` path): obtain
`(options, guard)` from `unique_postgres_url()`, derive the name with the file's
existing `db_name_from_url(&options.to_string())` (`PostgresDbGuard.db_name` is
private; `DbConnectOptions` is `Display`), assert `database_exists(&db_name)` is
`true`, `drop(guard)`, assert it is `false`. `database_exists` takes a raw name
and needs no `TestBase`. Because `PostgresDbGuard::drop` joins its teardown
thread synchronously, the post-drop assertion is race-free. Add
`unique_postgres_url` to `pg_teardown.rs`'s `helpers` import. This one test
exercises the shared `PostgresDbGuard` primitive that every updated caller now
relies on.

## Out of scope (recorded, not changed)

`commands.rs` runs its Postgres test cases **unconditionally** — unlike the
gated `backup_interop.rs` / `pg_teardown.rs`. Orthogonal to the teardown leak
and not a defect under the Nix harness (PG always present); tracked separately
as **#277**.

## Acceptance criteria (observable)

1. `unique_postgres_url()` and `template_postgres_url()` each return the connect
   URL together with a `PostgresDbGuard` for the database they create; **no**
   call site of either helper discards the guard (mechanically verifiable —
   every `*_postgres_url()` call binds its guard for the test's lifetime).
2. No `CREATE DATABASE` path reachable from the test helpers yields a database
   without an accompanying teardown owner — neither helper returns a bare,
   un-owned `DbConnectOptions`, and the previously-leaking direct callers
   (`storage.rs:72`, `:862`, `:872`, `:885`, `:897`) hold guards.
3. `TestBase` owns its Postgres database via a `PostgresDbGuard` field and has
   no hand-written `impl Drop`; teardown for the `Backend::setup()` path is
   unchanged in observable behavior (the existing
   `per_test_database_is_dropped_on_teardown` still passes).
4. A `postgres_testing_enabled()`-gated regression test demonstrates a database
   from `unique_postgres_url()` exists while its guard is held and does **not**
   exist after the guard is dropped.
5. Teardown flows through the single `drop_test_database` helper (no second
   `DROP DATABASE` implementation) and never panics from `Drop`.
6. The full local gate is green: `cargo xtask validate` (static + clippy +
   coverage + e2e), with the Postgres test paths exercised under the ephemeral
   cluster.
