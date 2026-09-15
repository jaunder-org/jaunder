# SeaQuery evaluation for Jaunder dynamic SQL (#1491)

## Conclusion

Do not adopt SeaQuery for Jaunder's current dynamic SQL patterns.

SeaQuery 1.0.2 can express the structural subset Jaunder needs and
`sea-query-sqlx` 0.9.1 matches SQLx 0.9 at the Cargo-version level. In Jaunder,
however, the adapter bypasses the `StorageBind` admission boundary, cannot carry
Jaunder's domain newtypes without a second conversion layer, and cannot support
Jaunder's Jiff values with PostgreSQL. Complex shared queries still need raw
expressions whose placeholder grammar differs by backend. SQLx's native
`QueryBuilder` already handles the favorable variable-cardinality cases while
preserving Jaunder's types.

The result is not a repository-wide objection to query builders. It is a
per-pattern finding that SeaQuery does not make these particular constructions
genuinely better under Jaunder's current invariants. No production dependency,
prototype code, or follow-up issue is warranted.

## Method

The evaluation was performed on 2026-09-15 against:

- Jaunder's pinned Rust 1.97.1 and SQLx `^0.9.0`, with SQLite, PostgreSQL,
  migrations, and Tokio runtime features (`rust-toolchain.toml`, `Cargo.toml`);
- the newest stable releases then published: `sea-query` 1.0.2 and
  `sea-query-sqlx` 0.9.1;
- production dynamic-SQL owners under `storage/src`, classified by why their SQL
  varies rather than by a raw count of `format!` calls; and
- a disposable Rust 2024 prototype, removed before delivery, that generated SQL
  for both backends and executed the favorable bulk-ID case against a temporary
  file-backed SQLite database.

The prototype covered:

1. a variable-cardinality `UPDATE ... WHERE id IN (...)` modeled on SQLite
   feed-event transitions;
2. a reduced site/user-tag Syndication Feed window query with a ranked CTE,
   dynamic joins, a visibility fragment, ordering, and bound values; and
3. SQLx execution success and a missing-table database error through
   `query_with` and SeaQuery's generated arguments.

It also ran two intentional compile-fail probes: Jaunder-like ID newtype input
and Jiff-enabled SQLite/PostgreSQL integration.

## Candidate compatibility

| Concern          | Evidence                                                                                                                                                                                                                  | Result                                                 |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Stable versions  | crates.io metadata reported `sea-query` 1.0.2 (2026-08-12) and `sea-query-sqlx` 0.9.1 (2026-05-30).                                                                                                                       | Current stable pair.                                   |
| Rust             | Declared minimums are Rust 1.88.0 and 1.94.0 respectively; Jaunder pins 1.97.1.                                                                                                                                           | Compatible.                                            |
| SQLx             | Adapter 0.9.1 declares `sqlx = "^0.9"` and `sea-query = "^1.0.0"`; the prototype resolved exactly SQLx 0.9.0 and SeaQuery 1.0.2.                                                                                          | Compatible at dependency resolution.                   |
| Backends/runtime | Adapter features include `sqlx-sqlite`, `sqlx-postgres`, `runtime-tokio`, and `tls-none`; both query builders compiled together.                                                                                          | Compatible for primitive values.                       |
| License          | Both crates declare `MIT OR Apache-2.0`.                                                                                                                                                                                  | Compatible with existing policy.                       |
| Jiff             | Enabling `with-jiff` with `sqlx-postgres` fails at compile time unless `unimplemented-jiff-sqlx` acknowledges retained runtime panics. The adapter source states that SQLx 0.9 PostgreSQL Jiff arguments are unsupported. | Incompatible with Jaunder's timestamp-bearing queries. |

Primary metadata and API sources:

- [crates.io `sea-query` metadata](https://crates.io/api/v1/crates/sea-query)
  and
  [1.0.2 dependencies](https://crates.io/api/v1/crates/sea-query/1.0.2/dependencies)
- [crates.io `sea-query-sqlx` metadata](https://crates.io/api/v1/crates/sea-query-sqlx)
  and
  [0.9.1 dependencies](https://crates.io/api/v1/crates/sea-query-sqlx/0.9.1/dependencies)
- [`SqlxBinder` 0.9.1 source](https://docs.rs/crate/sea-query-sqlx/0.9.1/source/src/sqlx.rs)
- [SQLite argument adapter](https://docs.rs/crate/sea-query-sqlx/0.9.1/source/src/sqlx_sqlite.rs)
  and
  [PostgreSQL argument adapter](https://docs.rs/crate/sea-query-sqlx/0.9.1/source/src/sqlx_postgres.rs)
- [SeaQuery 1.0.2 query API](https://docs.rs/sea-query/1.0.2/sea_query/)

No older stable adapter needed evaluation: the newest stable pair already
resolves against Jaunder's SQLx and Rust versions. Its Jiff limitation is an API
capability failure, not a semver-resolution failure that an older adapter could
repair.

## Dynamic construction inventory

### Formatted shared projections and predicates

`storage/src/posts/store.rs:856-2070`,
`storage/src/posts/syndication.rs:62-251`, and
`storage/src/posts/visibility.rs:103-208` assemble Post projections, backend tag
aggregation, cursor/order variants, four Syndication Feed surfaces, and
viewer-dependent visibility. Dynamic values remain SQLx binds; formatted inputs
are closed backend constants or owner-generated opaque fragments.

This is the highest-friction string construction and the strongest apparent
SeaQuery candidate. It is also where the prototype found the largest mismatch:
timestamp values need Jiff, visibility is a substantial correlated subquery,
backend tag aggregation remains custom SQL, and existing callers deliberately
coordinate fragment placeholder positions with typed bind order.

### Native SQLx builder composition

`storage/src/posts/media.rs:256-430` builds variable-cardinality evidence CTEs
and reusable predicates. `storage/src/media.rs:289-445` composes those helpers
for guarded deletion, retained-ID, safety, and reclaimability queries.
`storage/src/sqlite/posts.rs:433-475` and
`storage/src/postgres/posts.rs:536-580` use `QueryBuilder::push_values` for
variable-row media-reference writes.

These sites already receive automatic placeholder sequencing from SQLx while
`QueryBuilderStorageExt` admits only `StorageBind` values. SeaQuery can
represent CTEs and row lists, but its value path is less constrained and would
add a parallel builder vocabulary without removing backend policy.

### Backend-specific bulk membership

`storage/src/sqlite/feed_events.rs:22-350` generates `IN (?,...)` lists for bulk
transitions. The matching PostgreSQL dialect intentionally uses one array bind
with `id = ANY($n)` (`storage/src/feed_events.rs:447-460`). This is the smallest
favorable SeaQuery case: it generated valid `IN` lists and correct primitive
bind counts for both builders, and the SQLite statement updated three rows.

That success does not improve Jaunder's implementation. It changes PostgreSQL's
one-array-bind shape to one bind per ID, cannot accept `FeedEventId` directly,
and bypasses `StorageBind`. Keeping the PostgreSQL `ANY` expression as custom
SQL and converting IDs to primitives would retain more machinery than the
current short SQLite placeholder helper.

### Runtime catalog identifiers and backend administration

`storage/src/sqlite/open.rs:55-90`, `storage/src/sqlite/backup.rs:380-490`,
`storage/src/postgres/backup.rs:409-535`, and
`storage/src/postgres/bootstrap.rs:89-110` build SQL from validated catalog or
administrative identifiers. Jaunder already centralizes identifier/literal
quoting in `storage/src/sql.rs:440-460`.

SeaQuery can quote identifiers, but these statements are dominated by
backend-specific `PRAGMA`, JSON construction, sequence/`setval`, role, database,
and export semantics. Expressing them requires custom SQL or a second schema
builder vocabulary. Keeping the backup implementations separate is also an
explicit ADR-0019 decision; SeaQuery offers no useful shared body here.

### Fixed-fragment assembly

Small sites such as `storage/src/feed_events.rs:610-627` append one fixed
`RETURNING` clause to a backend-owned constant. They technically create a
runtime `String`, but have no runtime structural choice for an AST to clarify.
SeaQuery would be more code than the construction it replaced.

Static SQL and migrations were not evaluated, per scope.

## Prototype evidence

### Variable-cardinality generation and execution

For IDs `11, 12, 13`, SeaQuery generated:

```text
SQLite:    UPDATE "feed_events" SET "status" = ? WHERE "id" IN (?, ?, ?)
values:    ["pending", 11, 12, 13]
Postgres:  UPDATE "feed_events" SET "status" = $1 WHERE "id" IN ($2, $3, $4)
values:    ["pending", 11, 12, 13]
SQLite rows affected: 3
```

This proves backend placeholder rendering and SQLx execution for primitive
values. It does not prove Jaunder-domain bind compatibility; that probe failed
as described below.

### Complex composed query

SeaQuery represented the reduced ranked CTE, joins, ordinary predicates, and
ordering. `ROW_NUMBER() OVER (...)` and the correlated visibility predicate
remained custom expressions. The important generated shape was:

```text
WITH "ranked" AS (
  SELECT "p"."post_id", "p"."published_at",
         ROW_NUMBER() OVER (...) AS "rn"
  FROM "posts" AS "p" ...
  WHERE ... AND (<custom visibility expression>)
)
SELECT ... FROM "ranked"
WHERE "ranked"."rn" <= <bind>
ORDER BY "ranked"."published_at" DESC
```

Custom-expression values are not backend-neutral. With local `$1`/`$2`
placeholders, PostgreSQL collected the values but SQLite retained the tokens and
omitted those values from its argument list. With local `?` placeholders, the
reverse occurred. A shared Jaunder fragment would therefore need separate
backend text or complete AST expansion. This is worse than Jaunder's current
opaque fragment, which deliberately uses SQLx's accepted `$N` form on both
backends and tests exact bind arithmetic.

### Typed-bind compile failure

Replacing primitive prototype IDs with a Jaunder-shaped
`struct FeedEventId(i64)` failed:

```text
error[E0277]: the trait bound `sea_query::Value: From<FeedEventId>` is not satisfied
... required for `FeedEventId` to implement `Into<sea_query::Expr>`
... required by `ExprTrait::is_in`
```

The adapter converts SeaQuery's closed `Value` enum into SQLx arguments and
calls `Arguments::add` on primitive/foreign values directly. It never passes
through `storage::sql::StorageBind` or `QueryStorageExt::bind_storage`.
Supporting Jaunder's newtypes would require new SeaQuery conversions for those
types and a second admission policy. Unwrapping them to primitives would make
the existing type-only bind gate ineffective at precisely the dynamic-query
sites under evaluation.

### Jiff compile failure

Enabling the adapter's `with-jiff` feature alongside SQLite and PostgreSQL
failed deterministically:

```text
error[E0080]: evaluation panicked: sea-query-sqlx does not support with-jiff
 together with sqlx-mysql/sqlx-postgres/sqlx-any yet; enable the
 `unimplemented-jiff-sqlx` feature to acknowledge the limitation and keep the
 current runtime panic behavior
```

Jaunder uses `UtcInstant` throughout post listing, Syndication Feed windows, and
feed-event transitions. A feature whose documented path preserves runtime panic
behavior cannot satisfy Jaunder's error policy.

### SQL safety, errors, and observability

SQLx 0.9 rejected SeaQuery's returned `String` at `query_with` until the
prototype wrapped it in `AssertSqlSafe`:

```text
error[E0277]: dynamic SQL strings should be audited for possible injections
help: ... wrap with `AssertSqlSafe()`
```

SeaQuery therefore does not remove Jaunder's explicit dynamic-SQL audit
boundary. After that wrapper, executing against a missing table returned the
ordinary SQLx value:

```text
Database(SqliteError { code: 1, message: "no such table: missing_table" })
```

So SQLx database errors still propagate normally. Query construction itself is
infallible, while unsupported adapter values may panic; the Jiff probe shows
that this is weaker than Jaunder's typed error policy.

Storage spans are owned by the store methods, outside SQL construction—for
example `storage.feed_events.mark_regenerated` and the `storage.posts.*` list
spans—with the existing bounded `db.system` field. Replacing `query` with
`query_with` would not rename those spans or add fields. Conversely, SeaQuery
adds no useful observability: generated SQL and bind conversion happen inside
the same owning span, and its adapter exposes no typed diagnostic boundary. The
observability result is therefore neutral for supported primitive values and
unacceptable for panic-only unsupported values.

## Decision matrix

The labels use the thresholds defined by the approved issue spec.

| Pattern                                                | Readability/reuse                                                                                                                     | Safety and backend clarity                                                                                     | Escape hatches / cost                                                                    | Decision         |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ---------------- |
| Post/visibility/Syndication Feed formatted composition | Structured joins and predicates are readable, but the full query becomes a long builder chain and existing reusable fragments remain. | Fails Jiff support and `StorageBind`; shared custom-value fragments require backend-specific placeholder text. | Raw window, visibility, tag, date, and backend expressions remain; two new dependencies. | **do not adopt** |
| Media evidence CTEs and guarded operations             | SeaQuery can model CTEs, but SQLx `QueryBuilder` already provides focused reusable helpers.                                           | Native builder preserves Jaunder newtypes and backend-specific casts. SeaQuery does not.                       | Replaces one builder vocabulary with a broader one without removing policy.              | **do not adopt** |
| Variable-row `post_media` writes                       | Both builders handle row cardinality.                                                                                                 | Native `push_storage_bind` retains type admission on both backends.                                            | No meaningful reduction over `push_values`.                                              | **do not adopt** |
| Feed-event bulk membership                             | Removes the SQLite placeholder helper.                                                                                                | Loses `FeedEventId`, changes PostgreSQL `ANY` shape, and Jiff-bearing transitions are unsupported.             | Preserving `ANY` requires custom SQL; benefit is smaller than integration cost.          | **do not adopt** |
| Backup/catalog/open SQL                                | Identifier API could replace local quoting.                                                                                           | Backend differences remain explicit and intentionally separate.                                                | Most useful statements still require custom backend SQL.                                 | **do not adopt** |
| PostgreSQL bootstrap/admin SQL                         | Identifier construction is available.                                                                                                 | Role/database/password semantics remain PostgreSQL-only and carefully audited.                                 | Adds a schema vocabulary for a few statements with no reusable cross-backend form.       | **do not adopt** |
| Fixed-fragment assembly                                | No benefit over one fixed interpolation.                                                                                              | Current provenance is closed and values remain bound.                                                          | AST is more code than the query choice.                                                  | **do not adopt** |

No pattern reaches `consider`: the uncertainties are not missing measurements
that a bounded follow-up could resolve. They are observed integration and
invariant failures in the current stable releases. Revisit only in response to a
concrete future need and materially changed upstream capabilities—especially a
pluggable typed bind sink and non-panicking Jiff/PostgreSQL support—not as an
open-ended follow-up from #1491.

## Architectural consistency

- [ADR-0019](../../adr/0019-generic-storage-backend-via-dialect.md) keeps
  concrete database types and focused dialect differences; SeaQuery does not
  remove those differences.
- [ADR-0021](../../adr/0021-sqlite-transaction-discipline.md) makes statement
  and transaction shape correctness. The evaluation therefore rejects replacing
  PostgreSQL `ANY` or SQLite single-statement updates merely because returned
  rows might match.
- [ADR-0163](../../adr/0163-sqlx-decode-approval-is-type-only.md) governs decode
  rather than SQL text, but it exemplifies Jaunder's type-only database-boundary
  policy. `storage/src/sql.rs` applies the corresponding closed admission rule
  to binds; SeaQuery's primitive `Value` adapter bypasses it.

The conclusion changes no accepted decision and introduces no new architectural
direction, so no ADR is required.
