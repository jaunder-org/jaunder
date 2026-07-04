# Plan — #170: backend-generic fault-injection harness

- Issue: jaunder-org/jaunder#170
- Spec:
  [`docs/superpowers/specs/2026-07-04-issue-170-fault-injection-harness.md`](../specs/2026-07-04-issue-170-fault-injection-harness.md)
- Date: 2026-07-04
- For agentic workers: drive with **`jaunder-iterate`**; delegate individual
  tasks via **`jaunder-dispatch`** when useful. Tick checkboxes in real time.

## Review header

**Goal.** Make storage-error fault injection backend-agnostic: retain a
closeable pool in `TestBase`, so the two `web_backup.rs` auth-error tests run on
both backends instead of SQLite-only. Removes the SQLite-only
`test_sqlite_state_with_pool` and unifies the asymmetric raw-SQL pool access.
Prerequisite for #135.

**Scope.**

- _In:_ `CloseablePool` type; pool retained by `Backend::setup()` in `TestBase`;
  convert 2 web tests to `#[apply(backends)]`; remove
  `test_sqlite_state_with_pool` + migrate all 6 callers; rename
  `make_app_state`→`make_sqlite_app_state`; migrate pool-only raw-SQL PG sites
  to the shared pool.
- _Out (deferred, Task 1 files them):_ generic `make_app_state<DB>` unification;
  #135's storage-layer closed-pool conversions (consume this harness later).
  `AppState` is **not** removed (see spec).

**Tasks.**

1. ✅ File the deferred `make_*_app_state` unification as a separate issue →
   **#238**.
2. Add `CloseablePool` + `open_*_with_pool` seams; store the pool in `TestBase`
   with `close_pool()`/`pool()`.
3. Convert the two `web_backup.rs` storage-error tests to `#[apply(backends)]`
   via `base.close_pool()`.
4. Remove `test_sqlite_state_with_pool`; migrate the 4 `test-support` smoke
   tests to `Backend::Sqlite.setup()`.
5. Rename `make_app_state` → `make_sqlite_app_state`.
6. Migrate pool-only raw-SQL PG sites from `recorded_postgres_url` reconnect to
   `base.pool()`.
7. Full gate: `cargo xtask validate --no-e2e` green; reanchor coverage baseline
   if needed (with approval).

**Key risks/decisions.**

- Pool lives in `TestBase` (private field) → the ~330 `TestEnv { state, base }`
  destructures are untouched.
- PG teardown stays safe only because `drop_test_database` uses
  `DROP DATABASE … WITH (FORCE)` (spec Risks) — do not regress it.
- `CloseablePool` is test-only; production `db.rs` must not depend on it — the
  `_with_pool` seams return concrete `Pool<DB>`, and `Backend::setup` (test
  code) wraps them.
- Postgres close→500 is verified by AC3, not assumed.

## Global constraints

- **Language/layout:** Rust; follow `CONTRIBUTING.md` (backend parity, coverage
  ratchet, ADR-0019 dialect-file rules) and the repo layout. `CloseablePool` and
  harness live in `storage/src/test_support.rs`.
- **Gate per task:** run `devtool run -- cargo xtask check` (fmt + clippy
  `-D warnings` + Nix coverage/tests) before each commit (**`jaunder-commit`**).
  PG-backed tests need a cluster:
  `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- <cmd>`.
- **Commits:** one per task after its check is green; **no `Co-Authored-By`
  trailer**; do not commit without explicit approval per repo policy.
- **Verify base:** three-dot `git diff main...HEAD` (fork tag
  `wt-base-issue-170`).

---

## Task 1 — File the deferred builder-unification issue

**Why first:** separable concern surfaced in the design interview (spec
"Non-goals"); capture up front so it can be picked up concurrently.

**Do:** Via **`jaunder-issues`**, open a `test-infra`/`refactor` issue: "Unify
`make_sqlite_app_state`/`make_postgres_app_state` into a generic
`make_app_state<DB>(pool)`." Body: today the two builders
(`storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs`) are identical
except the backend type prefix; per ADR-0019 all fields but `atomic` are
`XStore<DB>` aliases, so a generic builder with an `AtomicOps` hook collapses
them. Note it touches ADR-0019's bound-propagation surface. Milestone 5.
Reference #170.

**Verify:** issue exists, labeled, referenced from this plan. No code change.

---

## Task 2 — `CloseablePool` + pool-returning seams + pool in `TestBase`

**Files:**

- `storage/src/test_support.rs` — add `CloseablePool`, add pool field to
  `TestBase`, thread pool through `Backend::setup`.
- `storage/src/db.rs` — add `open_database_with_pool` /
  `open_existing_database_with_pool`.
- `storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs` — add
  `open_*_database_with_pool` returning `(Arc<AppState>, Pool<DB>)`; existing
  fns delegate + drop the pool.
- `server/tests/helpers/mod.rs` — add `CloseablePool` to the
  `pub use storage::test_support::{…}` re-export (satisfies AC1's re-export
  clause; lets #135's server-side callers name the type).

**Interfaces (new):**

```rust
// storage/src/test_support.rs
/// A backend-tagged handle to the pool behind a test `AppState`, so tests can
/// inject a storage fault by closing it (subsequent queries error) or run raw SQL.
pub enum CloseablePool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl CloseablePool {
    /// Close the pool; the next query through any storage handle backed by it
    /// returns `sqlx::Error::PoolClosed`, which the storage layer maps to its
    /// `Internal` error. Backend-generic (`sqlx::Pool::close`).
    pub async fn close(&self) {
        match self {
            CloseablePool::Sqlite(p) => p.close().await,
            CloseablePool::Postgres(p) => p.close().await,
        }
    }
    /// The SQLite pool for raw-SQL access. Panics on a Postgres env.
    #[must_use]
    pub fn sqlite(&self) -> &sqlx::SqlitePool {
        match self { CloseablePool::Sqlite(p) => p, CloseablePool::Postgres(_) => panic!("sqlite() on a Postgres CloseablePool") }
    }
    /// The Postgres pool for raw-SQL access. Panics on a SQLite env.
    #[must_use]
    pub fn postgres(&self) -> &sqlx::PgPool {
        match self { CloseablePool::Postgres(p) => p, CloseablePool::Sqlite(_) => panic!("postgres() on a SQLite CloseablePool") }
    }
}
```

`TestBase` gains a private field and two methods (no destructure impact — fields
are private):

```rust
pub struct TestBase {
    dir: TempDir,
    postgres_db: Option<String>,
    pool: CloseablePool,      // NEW
}
impl TestBase {
    fn sqlite(dir: TempDir, pool: sqlx::SqlitePool) -> Self { Self { dir, postgres_db: None, pool: CloseablePool::Sqlite(pool) } }
    fn postgres(dir: TempDir, db_name: String, pool: sqlx::PgPool) -> Self { Self { dir, postgres_db: Some(db_name), pool: CloseablePool::Postgres(pool) } }
    /// Inject a storage fault: close the pool behind this env's `AppState`.
    pub async fn close_pool(&self) { self.pool.close().await; }
    /// The pool behind this env's `AppState`, for raw-SQL seed/inspect.
    #[must_use] pub fn pool(&self) -> &CloseablePool { &self.pool }
}
```

Pool-returning seams (production code, concrete pool — **no `CloseablePool`
dependency in `db.rs`**):

```rust
// storage/src/sqlite/mod.rs — new; existing open_sqlite_database delegates and drops the pool
pub(super) async fn open_sqlite_database_with_pool(
    options: &SqliteConnectOptions, create_if_missing: bool,
) -> sqlx::Result<(Arc<AppState>, SqlitePool)> { /* body of today's open_sqlite_database (keep the PRAGMA cache_size + migrate! calls), returning (make_app_state(pool.clone()), pool) — the builder is renamed to make_sqlite_app_state in Task 5, which runs after this */ }

pub(super) async fn open_sqlite_database(options: &SqliteConnectOptions, create_if_missing: bool) -> sqlx::Result<Arc<AppState>> {
    Ok(open_sqlite_database_with_pool(options, create_if_missing).await?.0)
}
// storage/src/postgres/mod.rs — symmetric open_postgres_database_with_pool -> (Arc<AppState>, PgPool)
// storage/src/db.rs — open_database_with_pool / open_existing_database_with_pool dispatch on DbConnectOptions and return the concrete pool for each backend arm.
```

`Backend::setup` uses the `_with_pool` path and wraps into `CloseablePool`
inside the `TestBase::{sqlite,postgres}` constructors (SQLite arm passes the
`SqlitePool`, Postgres arm the `PgPool`).

**Tests:** no new test; this task is exercised by every existing
`Backend::setup()` caller.

**Run (expect PASS — nothing behaviorally changed yet):**

- `devtool run -- cargo xtask check --no-test` (compiles, clippy clean).
- `cargo nextest run -p storage` (SQLite pass).
- `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder`
  (both backends still green).

**Commit:**
`test-infra(#170): add CloseablePool + pool-returning open seams; retain pool in TestBase`.

---

## Task 3 — Convert the two `web_backup.rs` storage-error tests to dual-backend

**Files:** `server/tests/web/web_backup.rs`.

**Do:** Replace each `#[apply(sqlite_only)]` test (lines ~469–490, ~495–516)
with:

```rust
#[apply(backends)]
#[tokio::test]
async fn backup_warning_visible_propagates_storage_error_during_auth(#[case] backend: Backend) {
    // Err(non-Unauthorized) branch: close the pool after session creation so
    // authenticate() returns Internal (not Unauthorized) → handler returns 500.
    let TestEnv { state, base } = backend.setup().await;
    let cookie = create_session_cookie(&state, "operator", true).await;

    base.close_pool().await;

    let (status, _body) = post_form(Arc::clone(&state), "/api/backup_warning_visible", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}
```

Same shape for `current_user_is_operator_propagates_storage_error_during_auth`
(`/api/current_user_is_operator`). Remove the `// reason:` markers, the
`let _ = backend;`, the manual `TempDir`, and the `test_sqlite_state_with_pool`
call. Drop the now-unused `sqlite_only` from the file's `use` list only if no
other test in the file needs it.

**Run:**

- FAIL-gate check that Postgres is actually exercised:
  `cargo nextest list -p jaunder -E 'test(propagates_storage_error_during_auth)'`
  shows `::case_1_sqlite` **and** `::case_2_postgres` for each.
- `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder -E 'test(propagates_storage_error_during_auth)'`
  → all 4 cases PASS (**AC3**). If a Postgres case flakes on pool drain, ensure
  `close_pool().await` fully awaits before `post_form` (already the shape).

**Commit:**
`test(#170): run backup auth storage-error propagation on both backends`.

---

## Task 4 — Remove `test_sqlite_state_with_pool`; migrate the 4 `test-support` smoke tests

**Files:** `storage/src/test_support.rs` (delete helper),
`test-support/src/lib.rs` (4 call sites: `:184`, `:213`, `:238`, `:272`),
`server/tests/helpers/mod.rs` (drop the re-export).

**Do:** In each `test-support` smoke test, replace:

```rust
let base = tempfile::TempDir::new().unwrap();
let (state, _pool) = test_support::test_sqlite_state_with_pool(&base).await;
```

with:

```rust
let state = test_support::Backend::Sqlite.setup().await.state;
```

(These stay SQLite-only by design — see their module docs; not under the
`server/tests` guard.) Then delete `pub async fn test_sqlite_state_with_pool`
and remove its `server/tests/helpers/mod.rs` re-export.

**Note (benign config change):** `test_sqlite_state_with_pool` builds a bare
in-file pool; `Backend::Sqlite.setup()` uses the production
`open_sqlite_database` config (WAL, busy_timeout, cache PRAGMA). The smoke tests
gain the real pool config — an improvement, but flagged so a behavior shift
isn't a surprise.

**Run:**

- `rg -n test_sqlite_state_with_pool` → only generated/archive hits remain
  (**AC4**).
- `cargo nextest run -p test-support` → 4 smoke tests PASS.
- `devtool run -- cargo xtask check --no-test` (clippy: no
  dead-code/unused-import fallout).

**Commit:**
`test-infra(#170): drop test_sqlite_state_with_pool; smoke tests use Backend::setup`.

---

## Task 5 — Rename `make_app_state` → `make_sqlite_app_state`

**Files:** `storage/src/sqlite/mod.rs` (definition `:62` + caller in
`open_sqlite_database_with_pool`).

**Do:** Rename the private fn and its call site for symmetry with
`make_postgres_app_state`. Pure rename; no behavior change.

**Run:** `devtool run -- cargo xtask check --no-test` (compiles, clippy clean).
`rg -n 'fn make_app_state\b'` → no hits.

**Commit:**
`refactor(#170): name the SQLite AppState builder make_sqlite_app_state`.

---

## Task 6 — Migrate pool-only raw-SQL PG sites to the shared `TestBase` pool

**Files:** `server/tests/storage/storage.rs` (Postgres arms at `:193`, `:2577`,
`:2607`, `:7171`).

**Do:** Replace
`let pool = PgPool::connect(&recorded_postgres_url(&env.base)).await.unwrap();`
(then `fetch_*(&pool)`) with borrowing the shared pool:
`let pool = env.base.pool().postgres();` and `fetch_*(pool)`. Each of the 4 arms
uses the pool for read/inspect only (none closes it), so the shared borrow is
semantically equivalent. Leave `recorded_postgres_url`/`PG_URL_FILE` for the
URL-string consumers (`pg_teardown.rs:50`, `storage.rs:620`).

**Out of scope (do NOT migrate):** the SQLite sibling arms that use
`open_pool(&env.base)` (e.g. `:189`). AC6 names only the Postgres sites, and
`open_pool` is annotated FK-enforcing (`storage.rs:167`) — swapping it for the
shared pool would need FK-semantics equivalence _verified_, not assumed (repo
sqlx-SQLite `foreign_keys` gotcha). Keep this task tight to the 4 PG arms; a
symmetric SQLite cleanup is a separate follow-up if wanted.

**Run:**

- `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder --test storage`
  → PASS on both backends.
- `rg -n recorded_postgres_url` → every remaining caller parses a URL string,
  not a pool (**AC6**).

**Commit:** `test-infra(#170): raw-SQL tests use the shared TestBase pool`.

---

## Task 7 — Full gate + coverage baseline

**Do:** Run the full local gate; resolve any coverage-baseline/CRAP drift from
the removed helper + added PG case.

**Run:**

- `devtool run -- cargo xtask validate --no-e2e` → green (static + clippy + Nix
  coverage). (**AC7**)
- If the coverage gate flags moved lines: reanchor `coverage-baseline.json` (and
  `crap-manifest.json` if needed) per the coverage-baseline approval policy —
  **halt for approval before committing the reanchor** (feedback:
  coverage-baseline approval).

**Commit (baseline only, if reanchored, after approval):**
`chore(coverage)(#170): reanchor baseline after fault-injection harness`.

---

## Self-review checklist

- [ ] Every spec AC (AC1–AC7) maps to a task: AC1/AC2→T2, AC3→T3, AC4→T4,
      AC5→T5, AC6→T6, AC7→T7.
- [ ] No task smuggles deferred work (generic builder, #135 conversions) — T1
      files them instead.
- [ ] Each task independently verifiable with a named command + expected result.
- [ ] `WITH (FORCE)` teardown invariant preserved (no change to
      `drop_test_database`).
- [ ] No `CloseablePool` reference in production `db.rs`.
