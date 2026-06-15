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

### Bound propagation note

Rust does not propagate `where`-clause bounds from a supertrait into an `impl` block that
uses that supertrait as a bound. Concretely: `impl<DB: SessionDialect> SessionStorage for
SessionStore<DB>` cannot rely on the `Backend` where-clauses being satisfied — they must
be restated on the `impl` header. The required explicit bounds are:

- `for<'q> i64: Encode<'q, DB> + Type<DB>`
- `for<'q> String: Encode<'q, DB> + Type<DB>`
- `for<'q> &'q str: Encode<'q, DB> + Type<DB>`
- `for<'q> DateTime<Utc>: Encode<'q, DB> + Type<DB>`
- `for<'c> &'c Pool<DB>: Executor<'c, Database = DB>`
- `for<'q> DB::Arguments<'q>: IntoArguments<'q, DB>`
- `SessionRow: for<'r> FromRow<'r, DB::Row>`

Likewise, the `SessionDialect` trait declaration must repeat the `Backend` where-clause
bounds on itself, because Rust does not carry them through the supertrait reference.

## Consequences

- One place to change shared SQL; the bound-bundle is written once and reused, so each
  subsequent trait costs only its divergences.
- Public traits stay object-safe; `Arc<dyn …>` and Leptos type-keyed context are unaffected.
- Span names become backend-agnostic (`storage.session.*`) with a `db.system` field
  carrying `"sqlite"` or `"postgres"` for observability.
- Not `sqlx::Any`: `DB` stays concrete at the leaves, retaining full per-backend SQL power.
- Rejected: generic free fns + forwarding wrappers (boilerplate remains); whole-impl
  `macro_rules!` (fights `#[async_trait]`, poor tooling); trait default methods (need an
  associated `Db` type → breaks object safety).

Subsequent storage traits follow this pattern in later beads.
