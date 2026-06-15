# Storage backend dedup via a generic store + `Dialect` trait (§1.1)

- **Bead:** `jaunder-kq8w.3` (§1.1 of `docs/code-analysis-2026-06-12.md`)
- **Date:** 2026-06-15
- **Status:** design approved; POC scoped to `SessionStorage`

## Problem

`storage/src/sqlite/*` and `storage/src/postgres/*` are paired implementations of the
same traits (`UserStorage`, `SessionStorage`, `MediaStorage`, `FeedEventStorage`,
`FeedCacheStorage`, `UserConfigStorage`, `SiteConfigStorage`, …). For most methods the
two files are byte-for-byte identical except for the pool type (`SqlitePool` vs `PgPool`)
and the span-name prefix (`storage.sqlite.*` vs `storage.postgres.*`). Every schema or
query change must be made twice and kept in sync by hand. This is the single largest
source of accidental complexity in `storage/`.

The traits are consumed **exclusively as trait objects** — `Arc<dyn SessionStorage>` etc.
(`web/src/**` via `expect_context::<Arc<dyn …>>()`; constructed as
`Arc::new(SqliteSessionStorage::new(pool))`). DI on these `dyn` objects was settled by
`jaunder-kq8w.1` / ADR-0016. **Object safety of the public traits is therefore a hard,
non-negotiable constraint.**

## Decision

Adopt a **single generic store + per-trait `Dialect` trait** (call it Option D). The
public `dyn`-safe traits are left unchanged. For each storage trait we introduce one
generic concrete struct and write the trait impl **once**, with the per-backend
divergences isolated behind a small dialect trait.

This POC implements the pattern for **`SessionStorage`** only (chosen because its
`authenticate` method genuinely diverges — SQLite uses an explicit
`begin`/UPDATE/SELECT transaction to avoid `SQLITE_BUSY` on `RETURNING`+correlated
subquery, Postgres uses a single data-modifying CTE — so the POC exercises the dialect
seam, not just trivial dedup). The other three methods (`create_session`,
`revoke_session`, `list_sessions`) are identical across backends today.

### Shape

```rust
// The verbose sqlx *scalar bind* bounds + pool-executor bound, bundled ONCE for the
// whole storage layer. Row (FromRow) bounds are per-trait and live on each Dialect,
// since each trait has its own row tuple.
pub trait Backend: sqlx::Database
where
    for<'q> i64: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> &'q str: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, Self> + sqlx::Type<Self>,
    for<'c> &'c sqlx::Pool<Self>: sqlx::Executor<'c, Database = Self>,
{
    /// Recorded as the `db.system` span field (§4.5).
    const DB_SYSTEM: &'static str;
}

impl Backend for sqlx::Sqlite   { const DB_SYSTEM: &'static str = "sqlite"; }
impl Backend for sqlx::Postgres { const DB_SYSTEM: &'static str = "postgres"; }

// Per-trait divergences + that trait's own row bound.
#[async_trait]
pub trait SessionDialect: Backend
where
    SessionRow: for<'r> sqlx::FromRow<'r, Self::Row>,
{
    async fn touch_and_load(
        pool: &sqlx::Pool<Self>,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> sqlx::Result<Option<SessionRow>>;
}

pub struct SessionStore<DB: sqlx::Database> { pool: sqlx::Pool<DB> }

#[async_trait]
impl<DB: SessionDialect> SessionStorage for SessionStore<DB> {
    // create_session / revoke_session / list_sessions: shared SQL written ONCE,
    // each #[tracing::instrument(name = "storage.session.*", fields(db.system = DB::DB_SYSTEM))]
    // authenticate: shared orchestration; the diverging step delegates to DB::touch_and_load.
}

// Drop-in source compat — keeps the ~20 construction sites and tests compiling unchanged.
pub type SqliteSessionStorage   = SessionStore<sqlx::Sqlite>;
pub type PostgresSessionStorage = SessionStore<sqlx::Postgres>;
```

### Observability (acceptance criterion)

Span names become backend-agnostic (`storage.session.create`, not
`storage.sqlite.session.create`) and carry a `db.system = "sqlite" | "postgres"` field
sourced from `DB::DB_SYSTEM`. The `#[tracing::instrument]` stays on the store methods (the
concrete site that knows the backend). **Verify no existing test asserts the old
backend-prefixed span names** before renaming.

## Rejected alternatives

- **A — generic free helper fns + thin forwarding methods.** Still leaves a per-backend
  forwarding method (signature + delegation) in each impl: trades duplicated body for
  duplicated signature. Verbose bounds repeat at each call site. (§1.1 Option 1.)
- **B — whole-impl `macro_rules!`.** A per-method bang macro inside the impl fights
  `#[async_trait]` (the macro expands after the proc-macro runs, so generated async fns
  are never desugared), forcing a whole-impl macro that emits `#[async_trait]` itself and
  takes the diverging body as a passed-in item. Works, but macro-generated code is not
  greppable / go-to-def-able and scales poorly when a trait has several diverging methods.
  (§1.1 Option 2.)
- **C — trait default methods.** Requires an associated `type Db: Database` +
  `fn pool(&self) -> &Pool<Self::Db>` to name the differing pool in default bodies, which
  **breaks object safety** and cascades through every `Arc<dyn …>`. Dead end.
- **`sqlx::Any`.** Previously rejected: erasing the database type restricts access to
  useful backend-specific features. Option D keeps `DB` a concrete `Sqlite`/`Postgres` at
  the leaves, so dialect methods retain full per-backend power (CTEs, `RETURNING`,
  upserts) — the Any limitation, avoided.

## Why D for the rollout

The verbose sqlx bounds live once in `Backend`; every future trait's dialect is just
`: Backend` plus its own divergent methods, so the §1.1 rollout gets *cheaper per trait*
rather than constant-cost. No macros, no wrappers, ordinary generics; public traits stay
object-safe.

## Scope

- **In scope (this bead):** `SessionStorage` deduplicated via `Backend` + `SessionDialect`
  + `SessionStore<DB>`; `db.system` span field; type aliases for source compat; the
  pattern documented as a short ADR in `docs/decisions/` for subsequent traits to follow.
- **Out of scope:** the other storage traits (follow-on beads), `sqlx::test` boilerplate
  cleanup (`jaunder-5u0w`), and the broader observability items.

## Acceptance criteria

- `SessionStorage` deduplicated as described and reviewed; both-backend test matrix
  (SQLite host + Postgres VM/host-PG) still green.
- `db.system` carried as a span field, not a name prefix.
- Pattern documented so subsequent traits can follow.
- `scripts/verify` clean.

## Open item to confirm during implementation

Whether the `Backend` bound-bundle composes cleanly for **all** bind types the rest of the
rollout needs — `Tag`, `Username`, `Option<String>`, `i64`, byte slices/`Vec<u8>`, and the
various row tuples — since that bundle is the foundation every later trait leans on. If a
type doesn't bundle cleanly under one `Backend` supertrait, the fallback is per-trait bound
blocks on each `Dialect`, which is less elegant but still works.
