# SessionStorage dedup via generic store + Dialect (§1.1 POC) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deduplicate `SessionStorage` across the SQLite and Postgres backends into a single generic `SessionStore<DB>` implementation, proving the §1.1 `Backend` + `Dialect` pattern for the rest of the storage layer.

**Architecture:** A new object-*un*safe `Backend` trait bundles the sqlx scalar-bind/pool bounds once and carries a `DB_SYSTEM` const. A per-trait `SessionDialect: Backend` holds only the one diverging operation (`touch_and_load`, the SQLite-tx vs Postgres-CTE part of `authenticate`) plus the `SessionRow` row bound. A generic `SessionStore<DB>` struct implements the unchanged, object-safe public `SessionStorage` trait once; `type` aliases (`SqliteSessionStorage`/`PostgresSessionStorage`) keep every existing construction site compiling. Span names go backend-agnostic with a `db.system` field. Spec: `docs/superpowers/specs/2026-06-15-storage-backend-dedup-dialect-design.md`.

**Tech Stack:** Rust, `sqlx` (runtime `query`/`query_as`, not macros), `async_trait`, `tracing`.

**Testing note (refactor):** This is a behavior-preserving refactor. The safety net is the **existing** suite: the SQLite parity tests in `server/tests/storage.rs`, the per-backend closed-pool tests in each `sessions.rs`, and the web session integration tests. The rule each task: keep them green. The only *new* behavior is the `db.system` span field, verified by code review + a grep assertion (no cheap span-capture harness exists in-repo; a full span-field test is out of scope).

**Inner-loop commands:** iterate with `cargo nextest run` (full workspace — never `-p storage` alone); the commit gate is `scripts/verify` (run it, do not hand-run its stages).

---

### Task 1: Add the `Backend` trait (bound-bundle + `DB_SYSTEM`)

**Files:**
- Create: `storage/src/backend.rs`
- Modify: `storage/src/lib.rs` (add `mod backend;` near line 6, and `pub use backend::*;` in the re-export block)

- [ ] **Step 1: Create `storage/src/backend.rs`**

```rust
//! Backend abstraction for the deduplicated storage layer.
//!
//! `Backend` bundles, once, the sqlx scalar-bind and pool-executor bounds that
//! every generic storage helper needs, and carries the `db.system` value. It is
//! used only as a bound on concrete generic stores (e.g. `SessionStore<DB>`) —
//! never as a trait object — so its associated const does not affect the
//! object-safety of the public storage traits.

use chrono::{DateTime, Utc};

/// A sqlx database jaunder supports, with the common bind/executor bounds and
/// its OpenTelemetry `db.system` identity.
pub trait Backend: sqlx::Database
where
    for<'q> i64: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> &'q str: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'c> &'c sqlx::Pool<Self>: sqlx::Executor<'c, Database = Self>,
{
    /// Value of the `db.system` span field (`"sqlite"` | `"postgres"`).
    const DB_SYSTEM: &'static str;
}

impl Backend for sqlx::Sqlite {
    const DB_SYSTEM: &'static str = "sqlite";
}

impl Backend for sqlx::Postgres {
    const DB_SYSTEM: &'static str = "postgres";
}
```

- [ ] **Step 2: Declare and export the module in `storage/src/lib.rs`**

Add `mod backend;` to the `mod` block (alphabetical: just after `mod auth;`) and `pub use backend::*;` to the `pub use` block (just after `pub use auth::*;`).

- [ ] **Step 3: Verify it compiles**

Run: `cargo nextest run -p storage backend 2>&1 | tail -5` (or `cargo build -p storage`)
Expected: compiles clean. If the `for<'q>` bounds are rejected as a trait `where`-clause on your toolchain, move them onto the eventual `impl` headers instead (noted in Task 2) — but prefer them here.

- [ ] **Step 4: Commit** *(skip if running the whole plan as one bead commit — see Task 7)*

---

### Task 2: Add `SessionDialect`, `SessionStore<DB>`, and the generic impl

**Files:**
- Modify: `storage/src/sessions.rs` (append after the existing `SessionStorage` trait; keep `SessionStorage`, `SessionAuthError`, `SessionRecord` untouched)

- [ ] **Step 1: Append the dialect trait, the store, and the generic impl**

```rust
use crate::backend::Backend;
use crate::helpers::{generate_hashed_token, session_record_from_row, SessionRow};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::{Database, Pool};

/// Per-backend divergences of `SessionStorage`. The only operation that differs
/// between SQLite and Postgres is the atomic touch-and-load used by
/// `authenticate` (SQLite: explicit tx; Postgres: data-modifying CTE).
#[async_trait]
pub trait SessionDialect: Backend
where
    SessionRow: for<'r> sqlx::FromRow<'r, Self::Row>,
{
    /// Update `last_used_at` for `token_hash` to `now` and return the joined
    /// session row (with username), atomically. `None` if no such session.
    async fn touch_and_load(
        pool: &Pool<Self>,
        token_hash: &str,
        now: chrono::DateTime<Utc>,
    ) -> sqlx::Result<Option<SessionRow>>;
}

/// Generic `SessionStorage` backed by any `SessionDialect` database.
pub struct SessionStore<DB: Database> {
    pool: Pool<DB>,
}

impl<DB: Database> SessionStore<DB> {
    #[must_use]
    pub fn new(pool: Pool<DB>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl<DB: SessionDialect> SessionStorage for SessionStore<DB> {
    #[tracing::instrument(
        name = "storage.session.create",
        skip(self, label),
        fields(user_id, db.system = DB::DB_SYSTEM)
    )]
    async fn create_session(&self, user_id: i64, label: &str) -> sqlx::Result<String> {
        let (raw_token, token_hash) = generate_hashed_token()?;
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (token_hash, user_id, label, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&token_hash)
        .bind(user_id)
        .bind(label)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(raw_token)
    }

    #[tracing::instrument(
        name = "storage.session.authenticate",
        skip(self, raw_token),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn authenticate(&self, raw_token: &str) -> Result<SessionRecord, SessionAuthError> {
        let token_hash =
            crate::auth::hash_token(raw_token).map_err(|_| SessionAuthError::InvalidToken)?;
        let now = Utc::now();

        let row = DB::touch_and_load(&self.pool, &token_hash, now)
            .await?
            .ok_or(SessionAuthError::SessionNotFound)?;

        Ok(session_record_from_row(row)?)
    }

    #[tracing::instrument(
        name = "storage.session.revoke",
        skip(self, token_hash),
        fields(db.system = DB::DB_SYSTEM)
    )]
    async fn revoke_session(&self, token_hash: &str) -> sqlx::Result<()> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn list_sessions(&self, user_id: i64) -> sqlx::Result<Vec<SessionRecord>> {
        let rows = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s JOIN users u ON s.user_id = u.user_id
             WHERE s.user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(session_record_from_row).collect()
    }
}
```

- [ ] **Step 2: Verify the generic impl compiles**

Run: `cargo build -p storage 2>&1 | tail -20`
Expected: compiles (the old `SqliteSessionStorage`/`PostgresSessionStorage` structs still exist and are untouched). Likely failure modes and fixes:
- *"the trait bound `i64: Encode<…>` is not satisfied"* in `create_session`/`revoke_session` → the `Backend` `where`-clause isn't propagating as an implied bound; restate the four scalar/pool bounds on the `impl<DB: SessionDialect>` header.
- *"the trait bound `SessionRow: FromRow<…>` is not satisfied"* in `list_sessions` → likewise restate `where SessionRow: for<'r> sqlx::FromRow<'r, DB::Row>` on the impl header (it's declared on `SessionDialect` but may not propagate as implied).
- *`db.system` field rejected* → tracing accepts dotted field names; ensure the `fields(...)` form is `fields(user_id, db.system = DB::DB_SYSTEM)`.

- [ ] **Step 3: Commit** *(or defer to Task 7)*

---

### Task 3: Implement `SessionDialect` for SQLite + alias (rewrite `sqlite/sessions.rs`)

**Files:**
- Modify (rewrite top half): `storage/src/sqlite/sessions.rs`
- Test: existing `#[cfg(test)] mod tests` in the same file (keep verbatim — they use `SqliteSessionStorage::new`, now the alias)

- [ ] **Step 1: Replace the struct + `impl SessionStorage` block with the dialect impl + alias**

Replace lines 1–105 (the imports, `struct SqliteSessionStorage`, its `new`, and `impl SessionStorage for SqliteSessionStorage`) with:

```rust
use async_trait::async_trait;
use sqlx::{Pool, Sqlite};

use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// SQLite-backed session storage.
pub type SqliteSessionStorage = SessionStore<Sqlite>;

#[async_trait]
impl SessionDialect for Sqlite {
    async fn touch_and_load(
        pool: &Pool<Sqlite>,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> sqlx::Result<Option<SessionRow>> {
        // Two statements in one tx: SQLite's RETURNING with a correlated
        // subquery causes SQLITE_BUSY under concurrency, so update then select.
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE sessions SET last_used_at = $1 WHERE token_hash = $2")
            .bind(now)
            .bind(token_hash)
            .execute(&mut *tx)
            .await?;

        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT s.token_hash, s.user_id, u.username, s.label, s.created_at, s.last_used_at
             FROM sessions s
             JOIN users u ON u.user_id = s.user_id
             WHERE s.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(row)
    }
}
```

Leave the existing `#[cfg(test)] mod tests { ... }` block exactly as it is.

- [ ] **Step 2: Run the SQLite session tests**

Run: `cargo nextest run -p storage sqlite::sessions 2>&1 | tail -15`
Expected: PASS (the 4 closed-pool tests). The `authenticate_with_closed_pool` test exercises `touch_and_load` failing on a closed pool → `SessionAuthError::Internal`.

- [ ] **Step 3: Commit** *(or defer to Task 7)*

---

### Task 4: Implement `SessionDialect` for Postgres + alias (rewrite `postgres/sessions.rs`)

**Files:**
- Modify (rewrite top half): `storage/src/postgres/sessions.rs`
- Test: existing `#[cfg(test)] mod tests` (keep verbatim — `#[ignore]` VM tests)

- [ ] **Step 1: Replace the struct + impl block with the dialect impl + alias**

Replace lines 1–97 (imports, `struct PostgresSessionStorage`, `new`, `impl SessionStorage for PostgresSessionStorage`) with:

```rust
use async_trait::async_trait;
use sqlx::{Pool, Postgres};

use crate::helpers::SessionRow;
use crate::sessions::{SessionDialect, SessionStore};

/// Postgres-backed session storage.
pub type PostgresSessionStorage = SessionStore<Postgres>;

#[async_trait]
impl SessionDialect for Postgres {
    async fn touch_and_load(
        pool: &Pool<Postgres>,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> sqlx::Result<Option<SessionRow>> {
        // Postgres can update-and-join atomically with a data-modifying CTE.
        sqlx::query_as::<_, SessionRow>(
            "WITH updated AS (
                UPDATE sessions
                SET last_used_at = $1
                WHERE token_hash = $2
                RETURNING token_hash, user_id, label, created_at, last_used_at
             )
             SELECT updated.token_hash, updated.user_id, u.username, updated.label, updated.created_at, updated.last_used_at
             FROM updated
             JOIN users u ON updated.user_id = u.user_id",
        )
        .bind(now)
        .bind(token_hash)
        .fetch_optional(pool)
        .await
    }
}
```

Leave the existing `#[cfg(test)] mod tests { ... }` block exactly as it is.

- [ ] **Step 2: Verify the workspace compiles (Postgres tests are VM-gated)**

Run: `cargo build --workspace 2>&1 | tail -20`
Expected: clean. The Postgres `touch_and_load` body is only compiled, not run here (its tests are `#[ignore = "requires PostgreSQL test VM"]`); the host-PG coverage pass under `scripts/verify` exercises it.

- [ ] **Step 3: Commit** *(or defer to Task 7)*

---

### Task 5: Confirm exports, span-name fallout, and full parity suite

**Files:**
- Inspect/Modify if needed: `storage/src/lib.rs`, `storage/src/sqlite/mod.rs`, `storage/src/postgres/mod.rs`

- [ ] **Step 1: Confirm the alias exports still resolve**

The aliases are re-exported by `pub use sessions::SqliteSessionStorage;` (sqlite/mod.rs:19) and `pub use sessions::PostgresSessionStorage;` (postgres/mod.rs:355), and surfaced from the crate via the `pub use sqlite::{…}` / `pub use postgres::{…}` lists in `lib.rs`. No change expected — `SqliteSessionStorage` is now a `type` alias rather than a `struct`, but the export paths are identical.

Run: `cargo build --workspace 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 2: Check nothing asserts the old backend-prefixed span names**

Run: `grep -rn 'storage\.sqlite\.session\|storage\.postgres\.session' --include='*.rs' .`
Expected: no matches. If any test/string references them, update it to `storage.session.*` + `db.system` (or note why). Acceptance change is span-name → `storage.session.*` with `db.system` field.

- [ ] **Step 3: Run the full parity + session suite**

Run: `cargo nextest run 2>&1 | tail -25`
Expected: all SQLite tests pass, including `server/tests/storage.rs` parity suite and `web` session integration tests; Postgres tests skipped (VM). 0 failures.

- [ ] **Step 4: Commit** *(or defer to Task 7)*

---

### Task 6: Document the pattern as ADR 0019

**Files:**
- Create: `docs/decisions/0019-generic-storage-backend-via-dialect.md`

- [ ] **Step 1: Write the ADR (MADR style, matching 0016–0018)**

```markdown
# 0019. Generic storage backends via a `Backend` bound-bundle and per-trait `Dialect`

- Status: accepted
- Date: 2026-06-15
- Deciders: Michael Alan Dorman

## Context

`storage/src/sqlite/*` and `storage/src/postgres/*` were near-duplicate implementations
of the same object-safe storage traits, differing only in pool type and span-name prefix
(see code-analysis §1.1). The traits are consumed as `Arc<dyn …>` (DI per ADR-0016), so
they MUST stay object-safe.

## Decision

Share the bodies in one generic store per trait, keeping the public traits unchanged:

- A non-object-safe `Backend: sqlx::Database` trait bundles the common scalar-bind and
  pool-executor bounds once and carries `const DB_SYSTEM` (the `db.system` span field).
- A per-trait `XDialect: Backend` trait holds only that trait's diverging operations and
  its row bound.
- A generic `XStore<DB>` struct implements the public `XStorage` trait once; backend-
  specific SQL lives in the `XDialect` impls for `Sqlite`/`Postgres`.
- `type SqliteXStorage = XStore<Sqlite>` aliases preserve construction sites.

`SessionStorage` is the proof-of-concept (its `authenticate` genuinely diverges:
SQLite explicit tx vs Postgres data-modifying CTE → `SessionDialect::touch_and_load`).

## Consequences

- One place to change shared SQL; the bound-bundle is written once and reused, so each
  subsequent trait costs only its divergences.
- Public traits stay object-safe; `Arc<dyn …>` and Leptos type-keyed context are unaffected.
- Not `sqlx::Any`: `DB` stays concrete at the leaves, retaining full per-backend SQL power.
- Rejected: generic free fns + forwarding wrappers (boilerplate remains); whole-impl
  `macro_rules!` (fights `#[async_trait]`, poor tooling); trait default methods (need an
  associated `Db` type → breaks object safety).

Subsequent storage traits follow this pattern in later beads.
```

- [ ] **Step 2: Commit** *(or defer to Task 7)*

---

### Task 7: Certify and commit the bead

- [ ] **Step 1: Run the commit gate**

Run: `scripts/verify`
Expected: `commit gate passed (tests + coverage + e2e; skipped nix VM)`. Do NOT hand-run the individual stages; if it fails on a stale instrumented build (`mismatched data`), clear it and re-run `scripts/verify`.

- [ ] **Step 2: Sonnet subagent diff review**

Dispatch a Sonnet subagent to review the full diff against the spec + acceptance criteria; independently re-run `scripts/verify` rather than trusting the subagent's self-report.

- [ ] **Step 3: Single clean commit**

```bash
git add storage/ docs/decisions/0019-generic-storage-backend-via-dialect.md \
        docs/superpowers/specs/2026-06-15-storage-backend-dedup-dialect-design.md \
        docs/superpowers/plans/2026-06-15-session-storage-dedup-dialect.md
git commit  # message: "refactor(storage): dedup SessionStorage via generic store + Dialect (kq8w.3, §1.1)"
```

- [ ] **Step 4: Close the bead**

Run: `bd close jaunder-kq8w.3` (note in the close that the `Backend` bound-bundle composing for all bind types is the open item validated/deferred for the rollout).
