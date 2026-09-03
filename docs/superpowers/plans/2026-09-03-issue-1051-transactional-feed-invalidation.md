# Transactional Public Syndication Feed Invalidation Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded tasks through
> `jaunder-dispatch` when useful. This outline exists because the change carries
> shared storage-transaction, concurrency, protocol-parity, and SQL-ranking
> risk.

Authoritative contract:
[`2026-09-03-issue-1051-transactional-feed-invalidation.md`](../specs/2026-09-03-issue-1051-transactional-feed-invalidation.md)

## Scope

In:

- Old/new anonymous/Public projection classification at one explicit operation
  instant.
- Transactional feed-event ownership for Web and AtomPub Post mutations.
- Public eligibility in both due-time discovery paths.
- Viewer eligibility before HybridWindow ranking on SQLite and PostgreSQL.
- Backend-parametric integration evidence and the architecture projection.

Out:

- WebSub recovery, configuration invalidation, HTTP validator, serializer, and
  private/account-associated feed URL work.
- Revision-history, Deleted Post replay, audience-model, and schema changes.

## Task outline

- [x] Task 1: Correct viewer ranking and due-time discovery
  - Contract: `PostStorage::list_published_in_window` keeps its
    `ViewerIdentity`; each `FeedSurface` query applies the existing resolution
    predicate inside the ranked CTE before `ROW_NUMBER()`. The steady-state
    `(after, upto]` query and feed-relative startup catch-up admit only Posts
    with a Public audience.
  - Ownership: `storage/src/posts.rs`, focused worker expectations, and
    `server/tests/storage/listing.rs`.
  - Verification: `#[apply(backends)]` evidence covers anonymous and
    authenticated private-row crowd-out, all four surfaces, Deleted/non-Public
    due exclusions, deterministic ordering, and both due-time branches.

- [ ] Task 2: Make the storage mutation boundary own projection invalidation
  - Contract: one storage-owned transition policy derives old and new public
    projections at the request's explicit `UtcInstant`, deduplicates the exact
    Site/User/old-and-new-Tag paths, and calls `FeedEventStorage::enqueue_many`
    in the mutation's `WriteScope`. Empty path sets perform no feed write.
  - Contract: create, rendered update, publish, unpublish, and soft delete
    expose service operations that preserve their existing typed errors and
    `MutationOutcome`; old state is observed under the backend-appropriate write
    lock rather than supplied by a protocol handler.
  - Ownership: `storage/src/post_service.rs` and only the necessary Post storage
    primitives in `storage/src/posts.rs`; do not introduce a second feed-path
    grammar beside `host::feed`.
  - Verification: backend-parametric integration evidence covers Public, Draft,
    future-scheduled, Private, Subscribers-only, Named-only, audience
    transitions, semantic no-op, repeated lifecycle, reverse rescheduling, Tag
    union/deduplication, and rollback on injected event insertion failure.

- [ ] Task 3: Converge Web and AtomPub on the shared mutation services
  - Dependency: Task 2's service operations and error contracts are stable.
  - Contract: `web/src/posts/api.rs` and `server/src/atompub/posts.rs` retain
    transport authorization, conditional-request, response, and metrics policy;
    neither snapshots old Tags nor computes/enqueues public feed paths.
  - Verification: backend-parametric Web and AtomPub integration scenarios prove
    equivalent create/update/delete/publication/unpublication transitions
    produce the same concrete paths and absence cases produce none. Existing
    ETag, idempotency-replay, status, and `MutationOutcome` behavior remains.
  - Documentation: update `docs/ARCHITECTURE.md` to describe transactional,
    protocol-independent invalidation, corrected due filtering, and
    eligibility-before-ranking; remove issue #1051 implementation-debt claims.

## Risk checks

- One explicit operation instant controls mutation timestamps and both
  projection classifications; no ambient clock read occurs inside the
  classification.
- SQLite keeps `BEGIN IMMEDIATE`; PostgreSQL retains operation-specific row
  locking. No read-before-transaction old-state snapshot remains.
- Rendering, media locking, metrics, and remote WebSub I/O stay outside the
  database write transaction as required by their existing contracts.
- Idempotency replay and storage semantic no-op paths enqueue nothing and do not
  advance feed-visible update state.
- Public-to-Public Tag-only changes still invalidate Site/User plus old/new Tag
  surfaces in RSS, Atom, and JSON Feed.
- Future-to-future scheduling is quiet; visible-to-future invalidates
  immediately; due processing is duplicate-safe and at-least-once, not
  exactly-once.
- Resolution SQL is not duplicated into a competing audience policy; all viewer
  kinds continue through the existing resolution helper.
- Every database-backed acceptance case runs through `#[apply(backends)]` or an
  equivalent backend-parametric integration harness.
- Exported service or trait changes migrate every caller and mock expectation;
  no aliases, compatibility wrappers, or deprecated paths remain.
- Each task reaches `jaunder-commit` after its focused evidence. The commit hook
  owns the single `precommit` run; commits contain no `Co-Authored-By` trailer.
