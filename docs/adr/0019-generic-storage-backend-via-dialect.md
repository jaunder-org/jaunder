# 0019. Generic storage backends via a `Backend` marker and per-trait `Dialect`

- Status: accepted
- Date: 2026-06-15
- Deciders: Michael Alan Dorman

## Context

`storage/src/sqlite/*` and `storage/src/postgres/*` were near-duplicate
implementations of the same object-safe storage traits, differing only in pool
type and span-name prefix (see code-analysis §1.1). The traits are consumed as
`Arc<dyn …>` (DI per ADR-0016), so they MUST stay object-safe.

## Decision

Share the bodies in one generic store per trait, keeping the public traits
unchanged:

- A non-object-safe `Backend: sqlx::Database` marker trait carries
  `const DB_SYSTEM` (the `db.system` span field) and nothing else — it
  deliberately holds no bind/executor `where`-bounds (see the bound-propagation
  note).
- A per-trait `XDialect: Backend` trait holds only that trait's diverging
  operations and its row bound. Traits with NO divergence (e.g.
  `UserConfigStorage`, whose backends are byte-identical) need no dialect at all
  — the generic `XStore<DB>` impl is written once, bounded directly by
  `Backend`.
- A generic `XStore<DB>` struct implements the public `XStorage` trait once;
  backend- specific SQL lives in the `XDialect` impls for `Sqlite`/`Postgres`.
- `type SqliteXStorage = XStore<Sqlite>` aliases preserve construction sites.

`SessionStorage` is the proof-of-concept (its `authenticate` genuinely diverges:
SQLite explicit tx vs Postgres data-modifying CTE →
`SessionDialect::touch_and_load`).

### Bound propagation note

Rust does not propagate a trait's `where`-clause bounds to subtraits or to
`impl` blocks that use the trait as a bound. An earlier design put the common
sqlx bind/executor bounds on `Backend`'s `where`-clause to "bundle them once";
that bought nothing — every store `impl` had to restate them anyway, and worse,
`DB: Backend` then _required_ all of them even for a trait that doesn't use them
(`UserConfigStorage` was forced to carry the `DateTime` bound). So `Backend`
carries no bounds, and each store `impl` restates exactly what it binds. The
full menu (each store uses the subset it needs):

- `for<'q> i64: Encode<'q, DB> + Type<DB>`
- `for<'q> &'q str: Encode<'q, DB> + Type<DB>`
- `for<'q> DateTime<Utc>: Encode<'q, DB> + Type<DB>` (only if it binds
  timestamps)
- `for<'c> &'c Pool<DB>: Executor<'c, Database = DB>`
- `for<'q> DB::Arguments<'q>: IntoArguments<'q, DB>`
- `<RowTuple>: for<'r> FromRow<'r, DB::Row>` (for `query_as`)

A divergent trait's `XDialect` declaration likewise restates the subset its own
method signatures need, since those bounds don't carry through the `Backend`
supertrait reference.

## Consequences

- One place to change shared SQL; each subsequent trait costs only its
  divergences plus a fixed, copy-pasteable bounds block on its `impl`.
- Public traits stay object-safe; `Arc<dyn …>` and Leptos type-keyed context are
  unaffected.
- Span names become backend-agnostic (`storage.session.*`) with a `db.system`
  field carrying `"sqlite"` or `"postgres"` for observability.
- Not `sqlx::Any`: `DB` stays concrete at the leaves, retaining full per-backend
  SQL power.
- Rejected: generic free fns + forwarding wrappers (boilerplate remains);
  whole-impl `macro_rules!` (fights `#[async_trait]`, poor tooling); trait
  default methods (need an associated `Db` type → breaks object safety).

Subsequent storage traits follow this pattern in later beads.

## Scope and exclusions

This pattern pays off where the backends are near-identical and the divergence
is a small, enumerable set of operations (a dialect with a handful of methods or
SQL-fragment consts). It is deliberately NOT applied to **backup storage**
(`storage/src/sqlite/backup.rs` vs `storage/src/postgres/backup.rs`), which
differ in 361 of 469 lines: dump/restore is fundamentally backend-specific
(catalog introspection, `COPY` streaming vs SQLite row-dump, sequence/`setval`
handling, type formatting). A `BackupStore<DB>` + `BackupDialect` would push
almost the entire body into the dialect — adding an indirection layer while
removing essentially no duplication. The two implementations are kept separate
on purpose; the shared concept lives in the public `BackupStorage` trait, not in
a generic store.

The rule of thumb: dedup when the dialect would be small relative to the shared
body; leave separate when the "shared store" would be a thin shell over a
near-total dialect. (Decided in jaunder-p8ea after the §1.1 rollout deduped the
other ten traits.)
