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
3. SQLx execution success and current-SQLx/SeaQuery missing-table database
   errors on both SQLite and PostgreSQL.

It also ran two intentional compile-fail probes—Jaunder-like ID newtype input
and Jiff-enabled SQLite/PostgreSQL integration—and applied the repository's
`cargo deny` policy to the disposable dependency graph.

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

### Dependency policy

The exact candidate pair was temporarily added to the root workspace dependency
catalog and to `storage` so it participated in the complete product graph. The
repository's authoritative host command then reported:

```text
cargo deny check
exit code: 0
advisories ok, bans ok, licenses ok, sources ok
```

The temporary manifest and root-lock changes were restored, removing SeaQuery
from the product graph. Running the same command against the final tree produced
the same exit-zero four-policy result. No exception, allowlist, source override,
or repository policy change was needed for SeaQuery or its adapter, and no
candidate dependency remains in a manifest or lockfile.

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
remained custom expressions. With `?` as the custom expression's two local
placeholders, the exact user-plus-tag output was:

```text
SQLite SQL: WITH "ranked" AS (SELECT "p"."post_id", "p"."published_at", ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS "rn" FROM "posts" AS "p" INNER JOIN "users" ON "p"."user_id" = "users"."user_id" INNER JOIN "post_tags" ON "p"."post_id" = "post_tags"."post_id" WHERE "p"."published_at" IS NOT NULL AND "p"."deleted_at" IS NULL AND ((p.user_id = ? OR EXISTS (SELECT 1 FROM post_audiences pa WHERE pa.post_id = p.post_id AND pa.target_kind_id = ?))) AND "users"."username" = ? AND "post_tags"."tag_id" = ?) SELECT "ranked"."post_id", "ranked"."published_at" FROM "ranked" WHERE "ranked"."rn" <= ? ORDER BY "ranked"."published_at" DESC
SQLite values: Values([BigInt(Some(7)), BigInt(Some(1)), String(Some("alice")), BigInt(Some(9)), BigInt(Some(20))])

PostgreSQL SQL: WITH "ranked" AS (SELECT "p"."post_id", "p"."published_at", ROW_NUMBER() OVER (ORDER BY p.published_at DESC, p.post_id DESC) AS "rn" FROM "posts" AS "p" INNER JOIN "users" ON "p"."user_id" = "users"."user_id" INNER JOIN "post_tags" ON "p"."post_id" = "post_tags"."post_id" WHERE "p"."published_at" IS NOT NULL AND "p"."deleted_at" IS NULL AND ((p.user_id = ? OR EXISTS (SELECT 1 FROM post_audiences pa WHERE pa.post_id = p.post_id AND pa.target_kind_id = ?))) AND "users"."username" = $1 AND "post_tags"."tag_id" = $2) SELECT "ranked"."post_id", "ranked"."published_at" FROM "ranked" WHERE "ranked"."rn" <= $3 ORDER BY "ranked"."published_at" DESC
PostgreSQL values: Values([String(Some("alice")), BigInt(Some(9)), BigInt(Some(20))])
```

The PostgreSQL builder left the custom `?` tokens unbound. Repeating the run
with local `$1`/`$2` placeholders reversed the failure: PostgreSQL generated
`$1`, `$2`, then `$3` with values `[7, 1, 20]`, while SQLite retained `$1` and
`$2` but recorded only the trailing value `[20]`. Custom-expression values are
therefore not backend-neutral. A shared Jaunder fragment would need separate
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
boundary. A second disposable run executed equivalent missing-table statements
through the current `sqlx::query(AssertSqlSafe(...))` path and SeaQuery's
`sqlx::query_with(AssertSqlSafe(...), generated_arguments)` path. The exact
results were:

```text
current sqlite error: Database(SqliteError { code: 1, message: "no such table: missing_current" })
sea-query sqlite error: Database(SqliteError { code: 1, message: "no such table: missing_sea_query" })
current postgres error: Database(PgDatabaseError { severity: Error, code: "42P01", message: "relation \"missing_current\" does not exist", detail: None, hint: None, position: Some(Original(16)), where: None, schema: None, table: None, column: None, data_type: None, constraint: None, file: Some("parse_relation.c"), line: Some(1501), routine: Some("parserOpenTable") })
sea-query postgres error: Database(PgDatabaseError { severity: Error, code: "42P01", message: "relation \"missing_sea_query\" does not exist", detail: None, hint: None, position: Some(Original(18)), where: None, schema: None, table: None, column: None, data_type: None, constraint: None, file: Some("parse_relation.c"), line: Some(1501), routine: Some("parserOpenTable") })
```

Both paths preserve SQLx's backend-specific `Database` error variant and
PostgreSQL SQLSTATE. Query construction itself is infallible, while unsupported
adapter values may panic; the Jiff probe shows that this is weaker than
Jaunder's typed error policy.

The observability comparison is source-auditable because the store-method span
owns the dialect call, not the SQL constructor. For example,
`storage/src/feed_events.rs:724-738` has this shape:

```rust
#[tracing::instrument(
    name = "storage.feed_events.mark_regenerated",
    skip(self, transaction, ids),
    fields(db.system = DB::DB_SYSTEM)
)]
async fn mark_regenerated(...) -> Result<(), FeedEventError> {
    // validation and connection selection
    DB::mark_regenerated(connection, ids).await
}
```

The complex post case is the same: the `storage.posts.list_published_by_user`
span and bounded `db.system` field begin at
`storage/src/posts/store.rs:1558-1565`; SQL construction and execution occur
inside that method at `:1573-1639`. Replacing an inner `query` with `query_with`
would leave the owning span name, field, skip policy, and propagated SQLx error
unchanged. SeaQuery adds no child span or bounded decision field of its own, so
observability is neutral for supported values. Its panic-only unsupported-value
path remains unacceptable even though the surrounding span still exists.

## Decision matrix

The labels use the thresholds defined by the approved issue spec.

| Pattern                                                | Readability/reuse                                                                                                                     | Safety and backend clarity                                                                                                                              | Escape hatches / cost                                                                    | Decision         |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- | ---------------- |
| Post/visibility/Syndication Feed formatted composition | Structured joins and predicates are readable, but the full query becomes a long builder chain and existing reusable fragments remain. | The adapter cannot bind Jiff values for PostgreSQL and bypasses `StorageBind`; shared custom-value fragments require backend-specific placeholder text. | Raw window, visibility, tag, date, and backend expressions remain; two new dependencies. | **do not adopt** |
| Media evidence CTEs and guarded operations             | SeaQuery can model CTEs, but SQLx `QueryBuilder` already provides focused reusable helpers.                                           | Native builder preserves Jaunder newtypes and backend-specific casts. SeaQuery does not.                                                                | Replaces one builder vocabulary with a broader one without removing policy.              | **do not adopt** |
| Variable-row `post_media` writes                       | Both builders handle row cardinality.                                                                                                 | Native `push_storage_bind` retains type admission on both backends.                                                                                     | No meaningful reduction over `push_values`.                                              | **do not adopt** |
| Feed-event bulk membership                             | Removes the SQLite placeholder helper.                                                                                                | Loses `FeedEventId`, changes PostgreSQL `ANY` shape, and Jiff-bearing transitions are unsupported.                                                      | Preserving `ANY` requires custom SQL; benefit is smaller than integration cost.          | **do not adopt** |
| Backup/catalog/open SQL                                | Identifier API could replace local quoting.                                                                                           | Backend differences remain explicit and intentionally separate.                                                                                         | Most useful statements still require custom backend SQL.                                 | **do not adopt** |
| PostgreSQL bootstrap/admin SQL                         | Identifier construction is available.                                                                                                 | Role/database/password semantics remain PostgreSQL-only and carefully audited.                                                                          | Adds a schema vocabulary for a few statements with no reusable cross-backend form.       | **do not adopt** |
| Fixed-fragment assembly                                | No benefit over one fixed interpolation.                                                                                              | Current provenance is closed and values remain bound.                                                                                                   | AST is more code than the query choice.                                                  | **do not adopt** |

No pattern reaches `consider`: the uncertainties are not missing measurements
that a bounded follow-up could resolve. They are observed integration and
invariant failures in the current stable releases. Revisit only in response to a
concrete future need and materially changed upstream capabilities—especially a
pluggable typed bind sink and non-panicking Jiff/PostgreSQL support—not as an
open-ended follow-up from #1491.

## Incidental generated-lock correction

The first hook-backed commit attempt found a pre-existing tools-workspace lock
inconsistency: `host/Cargo.toml` declares the existing workspace `rustix`
dependency, but the `host` package entry in `tools/Cargo.lock` omitted it. The
hook regenerated that one dependency-list entry and correctly refused to stage
work outside the intended Markdown set. The correction is isolated in commit
`d7744b7f7`; it adds no package or version, and
`cargo metadata --manifest-path tools/Cargo.toml --locked --format-version 1 --no-deps`
succeeds afterward. Keeping the generated correction is necessary for the
repository gate to evaluate the evidence commit; it is neither a SeaQuery
dependency nor retained prototype machinery.

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
