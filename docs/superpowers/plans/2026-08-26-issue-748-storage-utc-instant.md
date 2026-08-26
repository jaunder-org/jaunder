# Issue #748 — Storage-owned instants use `UtcInstant` implementation outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> changes a durable architecture seam and storage interfaces shared by both
> database backends and several caller crates.

Authoritative contract:
`docs/superpowers/specs/2026-08-25-issue-748-storage-utc-instant.md`.

## Scope

In:

- First-class SQLx and ordering support for the existing minimal `UtcInstant`.
- Clean migration of every storage-owned absolute instant, including public
  interfaces, private row shapes, dialects, backup metadata, and fixtures.
- Existing server, web, and host caller migration without wire or behavioral
  changes.
- Dual-backend proof for every migrated timestamp shape.
- The approved ADR draft and architecture projection.

Out:

- Jiff adoption or a broader `UtcInstant` interface redesign.
- Schema, stored-value, precision, timezone, scheduling, expiry, pagination, or
  protocol changes.
- Changes to durations, local wall-clock values, or `SystemTime` suffixes.
- ADR promotion and generated `docs/README.md` changes; those remain outside
  this issue's entire feature and ship path.

## Task outline

- [x] Task 1: Make `UtcInstant` a first-class ordered SQL scalar.
  - Contract: retain the current Chrono-backed representation, serde, parsing,
    display, `value()`, and conversions; add only `PartialOrd`/`Ord` and the
    plain `SqlxBridge`.
  - Verification: focused common/macro checks plus `#[apply(backends)]` scalar
    tests prove ordering, direct bind/decode, and nullable decode on SQLite and
    PostgreSQL.

- [x] Task 2: Migrate user storage instants.
  - Contract: `UserRecord`, private row parts/tuples, SQLx bounds, fixtures, and
    all callers use `UtcInstant`; nullable last-authentication meaning and wire
    serialization remain unchanged.
  - Dependency: Task 1. This task owns the user portion of
    `storage/src/helpers.rs`.
  - Verification: `#[apply(backends)]` user round trips cover required and
    nullable instants; existing web/server user representations remain stable.

- [x] Task 3: Migrate session storage instants.
  - Contract: session records, dialect arguments, row helpers, fixtures, and
    callers use `UtcInstant`; existing created/last-used role wrappers remain
    distinct over `UtcInstant`; explicit clock injection and touch freshness
    boundaries remain unchanged.
  - Dependency: Task 2 releases `storage/src/helpers.rs` first.
  - Verification: `#[apply(backends)]` tests cover both roles, direct
    bind/decode, and stale/exact/fresh touch boundaries without new clock reads.

- [x] Task 4: Migrate invite storage instants.
  - Contract: invite records, create inputs, row helpers, fixtures, and callers
    use `UtcInstant`; created/expires role wrappers remain distinct and nullable
    `used_at` retains its meaning.
  - Dependency: Task 3 releases `storage/src/helpers.rs` first.
  - Verification: `#[apply(backends)]` tests cover role ordering, unused/used,
    exact-expiry, expired, and claimable states; CLI/web behavior is unchanged.

- [x] Task 5: Migrate email-verification and password-reset storage instants.
  - Contract: both credential flows migrate their expiry inputs, shared
    token-state rows/classifiers, SQLx bounds, fixtures, and callers to
    `UtcInstant` together; token use and expiry predicates do not change.
  - Dependency: Task 4 releases `storage/src/helpers.rs` first. This task is the
    final owner of the shared token-state helper migration.
  - Verification: `#[apply(backends)]` tests cover both flows' claimable,
    expired, used, and nullable states plus unchanged web response behavior.

- [x] Task 6: Migrate post and publication storage instants.
  - Contract: post records/revisions, every post cursor and mutation input,
    `PublishUpdate::Publish`, ownership/query rows, fixtures, and all
    server/web/AtomPub callers use `UtcInstant` at the storage seam. ADR-0027's
    explicit `now`, predicates, and nullable publication meaning remain exact.
    This task owns post conversion work in `server/src/feed/regenerate.rs`.
  - Dependency: Task 1; this task owns post-related storage files and callers.
  - Verification: `#[apply(backends)]` tests cover every required/optional
    timestamp, row and cursor round trips, revision ordering, permalink dates,
    feed-window boundary/ordering selection, draft, scheduled, live, immediate,
    and backdated publication, cached-feed go-live, restart catch-up, and
    explicit `now`; AtomPub/web RFC 3339 forms, offset normalization, and
    browser local-wall-clock conversion remain unchanged.

- [x] Task 7: Migrate media storage instants.
  - Contract: media records, private helper rows, fixtures, and
    server/web/AtomPub callers use `UtcInstant`; content and creation semantics
    remain unchanged.
  - Dependency: Task 5 releases `storage/src/helpers.rs` first.
  - Verification: `#[apply(backends)]` media create/get/list round trips include
    `created_at`; existing media wire forms remain unchanged.

- [x] Task 8: Migrate feed-cache storage instants.
  - Contract: updated/generated role wrappers remain distinct over `UtcInstant`;
    rows, fixtures, and callers use them without changing HTTP validator or
    renderer representations. This task owns the remaining cache-row work in
    `server/src/feed/regenerate.rs`.
  - Dependency: Task 6 releases `server/src/feed/regenerate.rs` first.
  - Verification: `#[apply(backends)]` tests prove adjacent-role ordering and
    round trips; feed ETag, Last-Modified, RFC 2822, and cache-window behavior
    remain unchanged.

- [x] Task 9: Migrate feed-event storage instants.
  - Contract: all required/nullable lifecycle fields, dialect arguments,
    fixtures, backend implementations, and callers use `UtcInstant`; lease
    lengths remain durations and claim cutoffs remain explicit instants.
  - Dependency: Task 1.
  - Verification: `#[apply(backends)]` tests cover every lifecycle timestamp,
    pending/claimed/done/failed transitions, exact claim/reclaim boundaries,
    retry scheduling, and restart catch-up.

- [x] Task 10: Migrate subscription storage instants.
  - Contract: subscription records, dynamic row decoding, fixtures, and callers
    use `UtcInstant`; relationship behavior is unchanged.
  - Dependency: Task 1.
  - Verification: `#[apply(backends)]` create/list round trips include
    `created_at` and preserve ordering.

- [x] Task 11: Migrate audience storage instants.
  - Contract: audience records, private summary rows, fixtures, and callers use
    `UtcInstant`; audience membership behavior is unchanged.
  - Dependency: Task 1.
  - Verification: `#[apply(backends)]` create/get/list round trips include
    `created_at` and preserve ordering.

- [ ] Task 12: Migrate backup metadata and close the storage boundary.
  - Contract: `BackupManifest`, remaining storage-private tuples/locals, and
    storage test fixtures use `UtcInstant` wherever they represent absolute
    instants. Production storage has no raw `DateTime<Utc>` absolute-instant
    declaration; durations, local-wall-clock values, `SystemTime` suffixes, and
    explicit non-storage protocol adapters remain.
  - Dependency: Tasks 2–11 complete.
  - Verification: backup/restore preserves backend-specific precision and value
    interoperability; retention pruning preserves its existing cutoff behavior;
    structural/text audit finds no forbidden declaration; focused storage and
    external fixture tests pass.

- [ ] Task 13: Reconcile decision records and run full conformance.
  - Contract: the spec, proposed ADR draft, and architecture projection describe
    the delivered behavior; `CONTEXT.md` and generated `docs/README.md` remain
    untouched. ADR promotion remains a separate post-merge workflow.
  - Dependency: Task 12.
  - Verification: `devtool run -- cargo xtask validate` passes, including
    SQLite/PostgreSQL, protocol, backup, and documentation gates.

## Cross-task contracts

- Task 1 lands before any storage slice consumes `UtcInstant` through SQLx.
- Tasks touching `storage/src/helpers.rs` execute in the stated order; other
  disjoint slices may run concurrently only when their callers and fixtures do
  not overlap.
- Each slice migrates its production declarations, every caller, fixtures, and
  dual-backend tests together. No compatibility alias or raw-Chrono storage shim
  survives a task boundary.
- Each existing role wrapper migrates in its owning slice, never in Task 1.
- Protocol-specific Chrono conversions remain at explicit non-storage seams;
  they do not justify raw Chrono in storage-owned signatures or row types.

## Risk checks

- SQLite and PostgreSQL preserve their existing physical timestamp types,
  timezone normalization, and backend-specific precision.
- `NULL` remains `None`; no optional timestamp receives an epoch/default value.
- Role wrappers continue preventing adjacent timestamp-column transposition.
- Explicit `now` injection and all `<=`/`>` scheduling, lease, freshness, and
  expiry boundaries remain unchanged.
- Pagination ordering and cursor serialization preserve their current values.
- Backup interoperability remains value-based at PostgreSQL microsecond
  resolution; no byte-identical dump claim is introduced.
- External RFC 3339, AtomPub, Syndication Feed, and browser-local
  representations remain unchanged.
- No lint suppression is introduced without explicit user approval, and commits
  carry no `Co-Authored-By` trailer.
