# Issue #748 — Storage-owned instants use `UtcInstant`

## Outcome

Make `common::time::UtcInstant` the common type for every storage-owned absolute
instant, carrying its semantic type through the storage boundary without
changing stored values, schemas, protocol representations, or time semantics.

This records the implementation contract for
`docs/adr/drafts/storage-owned-instants.md`. That proposed decision supersedes
only ADR-0072's raw-`DateTime<Utc>` storage exception and updates ADR-0027's
type wording while retaining its explicit-`now` rule.

## Load-bearing decisions

- An **absolute instant** denotes a point on the UTC timeline. Storage owns one
  wherever it declares, accepts, returns, decodes, encodes, or fixtures that
  point through its traits, records, inputs, rows, cursors, dialects, and backup
  representation.
- Every storage-owned absolute instant becomes `common::time::UtcInstant`. This
  is a clean cutover: production storage declarations contain no raw
  `chrono::DateTime<Utc>` for an absolute instant.
- The scope is deliberately exhaustive: exported storage records and traits,
  private query-row/tuple shapes, cursors, write inputs, generic bounds,
  SQLite/PostgreSQL dialect signatures, `BackupManifest`, and storage fixtures
  all use the common instant type when they represent an absolute instant.
- Existing semantic, role-specific wrappers over `UtcInstant` remain. This work
  does not flatten distinct roles into bare instants or replace a role wrapper
  with a generic timestamp merely because its backing type changes.
- `UtcInstant` remains a minimal chrono-backed wrapper around
  `chrono::DateTime<Utc>`. It retains `value()`, the established `From`
  conversions, transparent serde, parsing, and display behavior.
- Its only new general capabilities are `SqlxBridge` and `PartialOrd`/`Ord`. No
  broad newtype trailer, validation rule, parsing format, serde form, or
  storage-specific sibling type is introduced.
- `SqlxBridge` is the ordinary transparent SQLx bridge: binds and decodes retain
  `UtcInstant` directly on both SQLite and PostgreSQL rather than stripping to a
  raw chrono value at the storage edge.
- SQL physical representation is unchanged. Existing column types, SQL syntax,
  stored values, precision, timezone normalization, migrations, and cross-
  backend restore guarantees remain as they are.
- The migration concerns instants only. Durations, local-wall-clock values,
  `SystemTime` suffixes, and non-storage protocol representations stay in their
  current types and forms.
- External wire behavior remains unchanged. Existing RFC 3339 serde and protocol
  payloads retain their current bytes/semantics; this is not a wire migration.
- Scheduled-publication behavior remains ADR-0027's behavior: public reads take
  an explicit `now`, apply the existing time gate, and never read the clock
  inside a query. Only the type of storage-owned absolute instant positions
  changes.
- Expiry, retention, feed, backup, pagination, scheduling, and publication logic
  preserve their existing instant comparisons and nullable meaning; a type
  migration must not alter a boundary condition or introduce clock reads.
- Chrono's soft deprecation motivates naming the public common type now: it
  reduces a future public-interface blast radius. It neither migrates Jaunder to
  Jiff nor claims full chrono implementation isolation; reviewed current Jiff
  documentation has no native SQLx integration.
- The design follows ADR-0063's domain-value intent and ADR-0071's first-class
  SQLx-boundary rule, while revising the storage limitation recorded in
  ADR-0072. It is architecture vocabulary, not a new domain term, so
  `CONTEXT.md` remains unchanged.

## Acceptance

- A repository search over production storage-owned absolute-instant
  declarations finds no raw `DateTime<Utc>` declaration in exported records or
  traits, private rows/tuples, cursors, inputs, generic SQLx bounds,
  SQLite/PostgreSQL dialects, `BackupManifest`, or their storage fixtures; each
  such position names `UtcInstant` or its existing semantic wrapper.
- Storage code can bind and decode non-null `UtcInstant` values directly through
  SQLx on SQLite and PostgreSQL, with no raw-chrono conversion at the storage
  boundary.
- Every migrated shape has dual-backend coverage using the repository backend
  harness, including direct records, query rows/tuples, cursor and input paths,
  dialect-specific paths, backup manifest serialization, and each nullable
  `Option<UtcInstant>` shape.
- Nullable instants preserve `NULL`/`None` meaning on both backends; absent
  publication, expiry, cursor, or optional timestamp values do not become a
  default instant or a different query predicate.
- Existing role-specific instant wrappers retain their distinct types and
  round-trip through their storage positions without callers unwrapping them to
  raw chrono values.
- Scheduling and publication tests still prove draft (`NULL`), scheduled
  (`published_at > now`), and live (`published_at <= now`) behavior, including
  explicit `now` injection, cached-feed go-live, restart catch-up, immediate
  publication, and backdating.
- Expiry, retention, feed-window, pagination/cursor, and backup/restore behavior
  retain their existing comparison and ordering semantics after the type change.
- Dual-backend backup coverage proves values remain interoperable at the
  existing PostgreSQL microsecond timestamp resolution; no claim of
  byte-identical dumps is introduced.
- Protocol and web-boundary coverage proves existing RFC 3339 timestamp fields,
  offset normalization, local-browser wall-clock conversion, and external
  AtomPub/feed representations remain wire-compatible.
- Storage declarations for durations, local-wall-clock values, `SystemTime`
  suffixes, and non-storage protocol timestamp representations remain unchanged.
- The repository validation gate, including its SQLite and PostgreSQL coverage,
  passes with the new architecture citation
  `docs/adr/drafts/storage-owned-instants.md` and without changing
  `docs/README.md` or `CONTEXT.md`.

## Boundaries

- No Jiff migration, Jiff abstraction layer, or assertion that chrono has been
  fully hidden from implementation code.
- No schema migration, timestamp precision change, timezone policy change, data
  rewrite, backup byte canonicalization, or SQL representation change.
- No changes to duration types, local wall-clock modeling, `SystemTime` suffix
  conventions, browser conversion, or non-storage protocol representations.
- No new clock reads, scheduling policy, visibility predicate, expiry policy,
  pagination ordering, or semantic-wrapper collapse.
- No raw-chrono compatibility shim at the storage boundary: every affected
  caller and internal storage shape moves to the common type.
