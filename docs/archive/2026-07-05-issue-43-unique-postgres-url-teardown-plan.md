# Issue #43 — PostgreSQL test-database teardown Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Every `CREATE DATABASE` path in the test helpers hands back an RAII
owner that drops the database on teardown, closing the bounded per-test
PostgreSQL leak (bounded residual of #28).

**Architecture:** Introduce one teardown primitive, `PostgresDbGuard` (owns a db
name; `Drop` calls the existing `drop_test_database`). Both `CREATE DATABASE`
helpers (`unique_postgres_url`, `template_postgres_url`) return
`(DbConnectOptions, PostgresDbGuard)`; every direct caller binds the guard for
the test's lifetime; `TestBase` composes the guard instead of hand-rolling its
own `Drop`.

**Tech Stack:** Rust, `sqlx` (Postgres), `rstest`/`rstest_reuse`,
`cargo nextest`, `cargo xtask` gate.

**Spec:**
`docs/superpowers/specs/2026-07-05-issue-43-unique-postgres-url-teardown.md` —
this plan is "how"; see the spec for "what/why". Decisions referenced by number
below.

## Review header

- **Goal:** Close the `unique_postgres_url()` leak and the folded-in
  `template_postgres_url()` direct-caller leaks with a single shared teardown
  guard.
- **Scope — in:** `PostgresDbGuard` primitive; tuple return for both helpers;
  guard-binding at all direct callers (`commands.rs`, `backup_interop.rs`,
  `storage.rs`); `TestBase` unified onto the guard (its bespoke `Drop` deleted);
  one gated regression test; generalize the stale `"issue #28"` teardown log
  strings.
- **Scope — out:** `commands.rs` unconditional-Postgres gating → filed as
  **#277**; any change to `nonexistent_postgres_url` / the two inline
  `StorageArgs {…}` blocks / `uninitialized_storage_args` (they mint no DB via
  these helpers).
- **Tasks:**
  1. Add `PostgresDbGuard`; migrate `unique_postgres_url()` + its callers; add
     the regression test.
  2. Migrate `template_postgres_url()` + its direct callers; unify `TestBase`
     onto the guard; generalize teardown log strings.
- **Key risks/decisions:** The two helper signature changes are workspace-wide
  compile breaks — each task is split so it compiles green on its own (Task 1
  touches only the `unique_*` path; Task 2 only the `template_*` path +
  `TestBase`). PostgreSQL teardown assertions only truly execute under the Nix
  gate (they self-skip when `!postgres_testing_enabled()`), so
  `cargo xtask check` — not a bare `nextest` — is the authoritative per-task
  verification.

## Global Constraints

- **Commit trailer:** No `Co-Authored-By` trailer (jaunder + user preference).
- **Single teardown impl:** teardown flows only through the existing
  `drop_test_database` free function (`storage/src/test_support.rs:372`); do
  **not** add a second `DROP DATABASE`. It runs on a joined scoped thread with
  its own current-thread runtime, `DROP DATABASE … WITH (FORCE)`, 10 s timeout,
  never panics.
- **Backend parity:** the PostgreSQL-teardown regression test is inherently
  single-backend; mirror the existing `pg_teardown.rs` test exactly —
  `#[apply(postgres_only)]` + a `// reason:` comment — so it satisfies the
  `test-backend-pattern` guard the same way
  `per_test_database_is_dropped_on_teardown` already does.
- **Gate:** the pre-commit hook runs the full `cargo xtask check` (fmt +
  clippy + Nix coverage/tests incl. PostgreSQL). Run
  `devtool run -- cargo xtask check` and get it green before each commit
  (**jaunder-commit**). Per-task iteration uses `check`; AC6's
  `cargo xtask validate` (which adds e2e) is the pre-merge gate run by
  **jaunder-ship** — this change has no e2e surface, so `check` is the
  meaningful per-task signal.
- **Crate names:** `storage` (the helpers + `TestBase`) and `jaunder` (the
  `server/` integration tests).

---

### Task 1: `PostgresDbGuard` + `unique_postgres_url()` migration + regression test

Closes the `unique_postgres_url()` leak (spec Decisions 1, 2, 4, 5). Leaves
`template_postgres_url()` and `TestBase` untouched, so the workspace compiles
green at the end of this task.

**Files:**

- Modify: `storage/src/test_support.rs` — add `PostgresDbGuard` (after
  `report_drop_outcome`, ~line 416); change `unique_postgres_url()`
  (`:437-461`).
- Modify: `server/tests/helpers/mod.rs:14-20` — add `PostgresDbGuard` to the
  re-export list.
- Modify: `server/tests/misc/commands.rs` — `storage_args` (`:41-48`) + its
  import (`:36-39`) + every `storage_args(...)` call site.
- Modify: `server/tests/misc/backup_interop.rs` — `postgres_storage_args`
  (`:38-43`) + import (`:24`) + its 2 call sites (`:158`, `:180`).
- Modify: `server/tests/storage/storage.rs:862, 872, 885` — the 3 direct
  `unique_postgres_url()` callers.
- Test: `server/tests/misc/pg_teardown.rs` — add the regression test + import.

**Interfaces:**

- Consumes: existing `drop_test_database(&str)`, `unique_postgres_db_name()`,
  `postgres_url_with_db_name(&str)`, `postgres_bootstrap_url()` (all in
  `test_support.rs`); `db_name_from_url` + `database_exists` (file-local in
  `pg_teardown.rs:12,22`).
- Produces:
  - `pub struct PostgresDbGuard { db_name: String }` with `impl Drop`. Field is
    **private**.
  - `pub async fn unique_postgres_url() -> (DbConnectOptions, PostgresDbGuard)`.
  - `async fn storage_args(backend: Backend, base: &TempDir) -> (StorageArgs, Option<PostgresDbGuard>)`
    (in `commands.rs`).
  - `async fn postgres_storage_args(base: &TempDir, name: &str) -> (StorageArgs, PostgresDbGuard)`
    (in `backup_interop.rs`).

- [x] **Step 1: Add `PostgresDbGuard` with a deliberately empty `Drop`, and
      migrate `unique_postgres_url()` to return it.**

In `storage/src/test_support.rs`, after `report_drop_outcome` (~line 416):

```rust
/// RAII owner of a per-test Postgres database created by [`unique_postgres_url`]
/// (or [`template_postgres_url`]). Dropping it removes the database via
/// [`drop_test_database`], so the ephemeral cluster's data dir does not grow with
/// the suite. This is the single teardown primitive; [`TestBase`] composes it.
pub struct PostgresDbGuard {
    db_name: String,
}

impl Drop for PostgresDbGuard {
    fn drop(&mut self) {
        // STEP 1 (temporary): does NOT drop the database yet, so the regression
        // test in Step 3 observes the leak (red). The `let _ = &self.db_name` reads
        // the field, so the intermediate state has no `dead_code` "field never read"
        // warning (which `-D warnings` would turn into a clippy failure, masking the
        // assertion-level red). Replaced with the real teardown in Step 4.
        let _ = &self.db_name;
    }
}
```

Change `unique_postgres_url()` (`:437`) to build and return the guard alongside
the URL; the body up to the `CREATE DATABASE` is unchanged — only the return:

```rust
pub async fn unique_postgres_url() -> (DbConnectOptions, PostgresDbGuard) {
    let db_name = unique_postgres_db_name();
    // ... unchanged bootstrap connect + CREATE DATABASE <db_name> OWNER <owner> ...
    let options = postgres_url_with_db_name(&db_name).parse().unwrap();
    (options, PostgresDbGuard { db_name })
}
```

- [x] **Step 2: Update every `unique_postgres_url()` caller to bind the guard**
      (so the workspace compiles).

`server/tests/helpers/mod.rs:14-20` — add `PostgresDbGuard` to the
`pub use storage::test_support::{…}` list.

`server/tests/misc/commands.rs`:

```rust
async fn storage_args(backend: Backend, base: &TempDir) -> (StorageArgs, Option<PostgresDbGuard>) {
    let storage_path = base.path().join("storage");
    let (db, guard) = match backend {
        Backend::Sqlite => (sqlite_url(base), None),
        Backend::Postgres => {
            let (db, guard) = unique_postgres_url().await;
            (db, Some(guard))
        }
    };
    (StorageArgs { storage_path, db }, guard)
}
```

Add `PostgresDbGuard` to the `use crate::helpers::{…}` import (`:36-39`). Then,
at **every** call site — find them with
`rg -n 'storage_args\(' server/tests/misc/commands.rs` (the ~25 `storage_args(`
calls; exclude the `uninitialized_storage_args` calls, which are unchanged) —
apply the uniform transform:

```rust
// before:
let args = storage_args(backend, &base).await;
// after:
let (args, _pg) = storage_args(backend, &base).await;
```

`args` stays a `StorageArgs`, so downstream `args.db` / `&args` uses (incl. the
inline `StorageArgs {…}` blocks at `:187` and `:263`, and `cmd_init(&args, …)`)
are unchanged. `uninitialized_storage_args` and its call sites are **not**
touched.

`server/tests/misc/backup_interop.rs`:

```rust
async fn postgres_storage_args(base: &TempDir, name: &str) -> (StorageArgs, PostgresDbGuard) {
    let (db, guard) = unique_postgres_url().await;
    (
        StorageArgs { storage_path: base.path().join(format!("{name}-storage")), db },
        guard,
    )
}
```

Add `PostgresDbGuard` to the import (`:24`). At its 2 call sites (`:158`,
`:180`), bind the guard:
`let (target_args, _pg_target) = postgres_storage_args(&base, "postgres-target").await;`
and the analogous `_pg_source` at `:180` (keep each test's existing arg-variable
name; add the `_pg_*` binding).

`server/tests/storage/storage.rs` (`:862`, `:872`, `:885`):

```rust
// before:
let url = unique_postgres_url().await;
// after:
let (url, _pg) = unique_postgres_url().await;
```

- [x] **Step 3: Write the regression test and run it — verify it FAILS (leak
      observed).**

In `server/tests/misc/pg_teardown.rs`, add `unique_postgres_url` to the
`use crate::helpers::{…}` import (`:1-3`), then add:

```rust
#[apply(postgres_only)]
// reason: asserts unique_postgres_url()'s per-test database is dropped when its
// PostgresDbGuard is dropped — via pg_database; SQLite has no such cluster.
#[tokio::test]
async fn unique_postgres_database_is_dropped_on_guard_drop(#[case] backend: Backend) {
    let _ = backend;
    if !postgres_testing_enabled() {
        return;
    }

    let (options, guard) = unique_postgres_url().await;
    let db_name = db_name_from_url(&options.to_string());

    assert!(
        database_exists(&db_name).await,
        "unique_postgres_url() database {db_name} should exist while its guard is held"
    );

    drop(guard);

    assert!(
        !database_exists(&db_name).await,
        "unique_postgres_url() database {db_name} should be dropped once its guard is gone"
    );
}
```

Run (authoritative — carries the ephemeral PostgreSQL cluster):

Run: `devtool run -- cargo xtask check` Expected: FAIL —
`unique_postgres_database_is_dropped_on_guard_drop` fails its second assertion
("should be dropped once its guard is gone"), because the temporary `Drop` reads
but does not act on `db_name` (no `drop_test_database` call). The field-touch
keeps the intermediate compiling clean under `-D warnings`, so the failure is
the assertion — proving the test actually catches the leak — not a lint.

- [x] **Step 4: Implement `PostgresDbGuard::drop`.**

Replace the empty body with the real teardown:

```rust
impl Drop for PostgresDbGuard {
    fn drop(&mut self) {
        drop_test_database(&self.db_name);
    }
}
```

- [x] **Step 5: Run the gate — verify PASS.**

Run: `devtool run -- cargo xtask check` Expected: PASS — the new test passes,
and the whole suite (incl. the ~30 previously-leaking `unique_postgres_url()`
caller tests) is green. Confirm via the sidecar if needed:
`ctx_execute(shell, "jq '.ok' .xtask/last-result.json")`.

- [x] **Step 6: Commit.**

```bash
git add storage/src/test_support.rs server/tests/helpers/mod.rs server/tests/misc/commands.rs server/tests/misc/backup_interop.rs server/tests/storage/storage.rs server/tests/misc/pg_teardown.rs
git commit -m "fix(test-support): drop unique_postgres_url() databases via PostgresDbGuard (#43)"
```

(The pre-commit hook re-runs `cargo xtask check`; it must already be green from
Step 5. **No `Co-Authored-By` trailer.**)

---

### Task 2: `template_postgres_url()` migration + `TestBase` unification

Closes the folded-in `template_postgres_url()` direct-caller leaks and unifies
`TestBase` onto `PostgresDbGuard` (spec Decisions 2, 3, 4). Touches only the
`template_*` path + `TestBase`, so the workspace compiles green on its own. The
existing `per_test_database_is_dropped_on_teardown` (`pg_teardown.rs:43`) is the
safety net proving the `TestBase` refactor preserves teardown; Task 1's
`PostgresDbGuard` is the already-proven teardown primitive these callers now
reuse.

**Files:**

- Modify: `storage/src/test_support.rs` — `template_postgres_url()`
  (`:535-557`); `TestBase` struct + constructors + delete its `Drop`
  (`:91-148`); `Backend::setup` Postgres arm (`:193-211`); `report_drop_outcome`
  log strings (`:413-414`); the `TestBase` doc comment (`:86-90`).
- Modify: `server/tests/storage/storage.rs` — `open_pg_pool` (`:71-75`) + its
  caller `lookup_names` (`:85`);
  `authenticate_with_corrupted_hash_returns_internal_error` (`:897`).

**Interfaces:**

- Consumes: `PostgresDbGuard` (from Task 1).
- Produces:
  - `pub async fn template_postgres_url() -> (DbConnectOptions, PostgresDbGuard)`.
  - `TestBase` field `_pg: Option<PostgresDbGuard>` (private);
    `fn postgres(dir: TempDir, pg: PostgresDbGuard, pool: PgPool) -> Self`; no
    `impl Drop for TestBase`. Public surface (`Deref` to `TempDir`, `pool()`,
    `close_pool()`) unchanged.
  - `async fn open_pg_pool() -> (PgPool, PostgresDbGuard)` (in `storage.rs`).

- [x] **Step 1: Migrate `template_postgres_url()` to return the guard.**

In `storage/src/test_support.rs`, change `template_postgres_url()` (`:535`)
return type and tail (body up to the `CREATE DATABASE … TEMPLATE` clone
unchanged):

```rust
pub async fn template_postgres_url() -> (DbConnectOptions, PostgresDbGuard) {
    // ... unchanged ensure_template_db + CREATE DATABASE <db_name> ... TEMPLATE ...
    let options = postgres_url_with_db_name(&db_name).parse().unwrap();
    (options, PostgresDbGuard { db_name })
}
```

- [x] **Step 2: Unify `TestBase` onto `PostgresDbGuard` and delete its bespoke
      `Drop`.**

Replace the `postgres_db: Option<String>` field with a self-dropping guard,
declared **after** `pool` so the pool drops before the database (`WITH (FORCE)`
makes ordering non-critical, but this matches intent):

```rust
pub struct TestBase {
    dir: TempDir,
    pool: CloseablePool,
    /// `Some` on Postgres (drops the per-test database on teardown); `None` on SQLite.
    _pg: Option<PostgresDbGuard>,
}

impl TestBase {
    fn sqlite(dir: TempDir, pool: SqlitePool) -> Self {
        Self { dir, pool: CloseablePool::Sqlite(pool), _pg: None }
    }

    fn postgres(dir: TempDir, pg: PostgresDbGuard, pool: PgPool) -> Self {
        Self { dir, pool: CloseablePool::Postgres(pool), _pg: Some(pg) }
    }
}
```

Delete `impl Drop for TestBase` (`:142-148`) entirely — the `_pg` field drops
itself. Update the `TestBase` doc comment (`:86-90`) to say it holds a
`PostgresDbGuard` that drops the per-test database (drop the "cloned from the
template" specificity, since it now serves both helpers). Keep `Deref`,
`pool()`, `close_pool()` unchanged.

- [x] **Step 3: Thread the guard through `Backend::setup`.**

In the Postgres arm (`:193-211`), take the guard from `template_postgres_url()`
and hand it to `TestBase::postgres` — dropping the now-unneeded `get_database()`
name extraction:

```rust
Backend::Postgres => {
    let (url, guard) = template_postgres_url().await;
    let DbConnectOptions::Postgres { options, .. } = &url else {
        unreachable!() // cov:ignore — template_postgres_url() always yields Postgres
    };
    let (state, pool) = crate::postgres::open_postgres_database_with_pool(options)
        .await
        .unwrap();
    // Record the per-test DB URL so raw-SQL helpers reuse this exact database.
    std::fs::write(dir.path().join(PG_URL_FILE), url.to_string())
        .expect("write recorded Postgres URL");
    (state, TestBase::postgres(dir, guard, pool))
}
```

- [x] **Step 4: Update the two direct `template_postgres_url()` callers in
      `storage.rs`.**

`open_pg_pool` (`:71-75`) returns the guard alongside the pool; `lookup_names`
(`:85`) holds it for the query:

```rust
async fn open_pg_pool() -> (PgPool, PostgresDbGuard) {
    let (url, guard) = template_postgres_url().await;
    let pool = PgPool::connect(&url.to_string()).await.unwrap();
    (pool, guard)
}
```

```rust
// lookup_names, Postgres arm (:84-87) — before:
Backend::Postgres => sqlx::query_scalar(&sql)
    .fetch_all(&open_pg_pool().await)
    .await
    .unwrap(),
// after:
Backend::Postgres => {
    let (pool, _pg) = open_pg_pool().await;
    sqlx::query_scalar(&sql).fetch_all(&pool).await.unwrap()
}
```

`authenticate_with_corrupted_hash_returns_internal_error` (`:897`):

```rust
// before:
let DbConnectOptions::Postgres { options, .. } = template_postgres_url().await else {
    panic!("expected postgres options");
};
// after:
let (url, _pg) = template_postgres_url().await;
let DbConnectOptions::Postgres { options, .. } = url else {
    panic!("expected postgres options");
};
```

- [x] **Step 5: Generalize the stale teardown log strings.**

In `report_drop_outcome` (`:413-414`), the messages hardcode `issue #28`; make
them helper-agnostic now that the primitive is shared:

```rust
Ok(Err(error)) => eprintln!("test database drop {db_name} failed: {error}"), // cov:ignore
Err(_elapsed) => eprintln!("test database drop {db_name} timed out"),        // cov:ignore
```

- [x] **Step 6: Run the gate — verify PASS.**

Run: `devtool run -- cargo xtask check` Expected: PASS — workspace compiles;
`per_test_database_is_dropped_on_teardown` still passes (proving the `TestBase`
refactor preserves teardown);
`unique_postgres_database_is_dropped_on_guard_drop` (Task 1) still passes; full
suite green.

- [x] **Step 7: Commit.**

```bash
git add storage/src/test_support.rs server/tests/storage/storage.rs
git commit -m "fix(test-support): drop template_postgres_url() clones; unify TestBase teardown (#43)"
```

(**No `Co-Authored-By` trailer.**)

---

## Self-review

- **Spec coverage:** Decision 1 (guard) → Task 1 Step 1/4. Decision 2 (both
  helpers return guard) → Task 1 Step 1 + Task 2 Step 1. Decision 3 (`TestBase`
  composes guard, `Drop` deleted) → Task 2 Steps 2-3. Decision 4 (all direct
  callers hold guards) → Task 1 Step 2 + Task 2 Steps 3-4. Decision 5
  (regression test) → Task 1 Step 3. Stale-`#28`-string cleanup → Task 2 Step 5.
  Out-of-scope #277 → filed. AC1-5 all map; AC6 (gate green) → each task's gate
  step.
- **Placeholder scan:** none — every step carries concrete Rust and exact
  commands.
- **Type consistency:** `PostgresDbGuard` / `unique_postgres_url` /
  `template_postgres_url` / `storage_args` / `postgres_storage_args` /
  `open_pg_pool` / `TestBase::postgres` signatures match across the tasks that
  produce and consume them.
