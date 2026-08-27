# Issue #1055: Full Local Post Revision History — Implementation Outline

> Execute in order with `jaunder-iterate`; delegate each bounded task through
> `jaunder-dispatch` when useful. This outline exists because the approved spec
> changes paired schemas, storage transactions, authorization, media safety, and
> owner-facing web interfaces.

## Scope and contracts

**In:** complete revision schema and backup shape; unified mutation/revision
transactions; retained current/Revision media references; owner history reads;
web routes/components; SQLite/PostgreSQL parity; integration/e2e proof.

**Out:** revert, revision mutation/deletion, public or AtomPub history, inbound
`ajr_entry_versions`, purge/erasure, legacy revision support, and issue #755’s
AtomPub delete response or per-user Media Record materialization.

- **Snapshot:** one immutable full prior state per meaningful top-level
  mutation; semantic no-op means no writes or timestamp movement.
- **Mutation:** storage owns locked read → canonical full-state comparison → one
  revision and children → mutation. Content plus tags/audiences/media is one
  transaction; dedicated publish/unpublish/delete follow the same discipline.
- **Schema:** scalar revision state plus normalized immutable tag/audience child
  rows. Current and Revision media references share one relation with an exact
  `Current(post_id)` or `Revision(post_id, revision_id)` subject key.
- **Evidence:** ownership evidence and conditional media predicates include the
  complete subject key; current evidence cannot exempt a concurrent Revision.
  Owner reporting deduplicates Post IDs across current/revision subjects.
- **Locks:** SQLite retains immediate single-writer transactions. PostgreSQL
  locks the Post row, normalized tag slugs, and media identities in established
  stable orders before snapshot/application.
- **Reads:** global/per-Post metadata rows contain `revision_id`, `post_id`,
  snapshot title/slug, capture time, snapshot lifecycle derived at capture time,
  and current-Post Deleted status. Current summary contains current Post ID,
  title, slug, format, created/updated/published/deleted timestamps, and
  lifecycle derived at request time. Detail returns the complete snapshot. Every
  query binds authenticated owner, Post ID, and revision ID in storage and masks
  absence/mismatch alike.
- **Pagination:** newest-first immutable revision-ID keyset with standard
  bounded page size, overfetch-one, opaque cursor, and append-style Load more.

## Ordered work

- [x] **1. Establish the complete revision schema and immutable read model.**
  - Add identical numbered SQLite/PostgreSQL migrations for every scalar field,
    normalized revision tags/audiences, and revision-qualified media subjects
    with integrity constraints and current/revision uniqueness.
  - Replace the partial `PostRevisionRecord` with complete snapshot,
    metadata-row, current-summary, page, and detail types. No legacy
    discriminator or backfill.
  - Extend backup/restore typing, validation, ordering, and integrity for every
    revision table/field.
  - Verification: backend-parametrized migration/schema and round-trip tests
    prove nullable fields, all child collections, subject integrity,
    immutability, and backup/restore fidelity.

- [x] **2. Unify full-state mutation, no-op suppression, and revision capture.**
  - Deepen the Post storage interface around complete desired-state mutation and
    dedicated lifecycle transitions. Callers cannot construct revisions or write
    tags after the transaction.
  - For content updates, compare canonical scalar/tag/audience/media state under
    lock; on change, capture exactly one full prior snapshot and children before
    applying all changes. Copy exact current media rows into the Revision
    subject before replacing current rows; never re-extract historical
    references. Acquire normalized tag locks by ascending slug and media locks
    in stable identity order.
  - Move web and AtomPub content/tag callsites into the unified operation. Keep
    creation atomic with tags but revision-free because no prior state exists.
  - Bring publish, unpublish, tag-only, and soft-delete operations through the
    same owner-checked snapshot/apply discipline; one accepted multi-field
    action creates one revision.
  - Verification: focused dual-backend tests cover each mutation class,
    multi-field single-revision grouping, complete child round-trip including
    exact historical media rows, normalized collection equality, semantic no-ops
    including unchanged `updated_at`, authorization, stale-write behavior, and
    reversed tag/media order deadlock resistance.

- [ ] **3. Retain Revision media references in the ordinary guard.**
  - Consume the revision-qualified subjects established by Task 2 throughout
    persisted reference keys, resolver evidence, global snapshots, conditional
    delete/reclaim predicates, and owner reporting. Treat concurrent unseen
    Revision rows conservatively under ADR-0154.
  - Preserve ADR-0136: current active, Deleted Post, and Revision references
    guard media; owner reporting returns unique Post IDs; explicit web force may
    break reconstruction. Do not implement #755 wire behavior.
  - Verification: dual-backend guard/reclaim tests cover current-only,
    revision-only, Deleted current, current+revision deduplication, cross-owner
    nondisclosure, proven-foreign evidence, concurrent revision insertion, and
    explicit force.

- [ ] **4. Add owner-only history storage and web surfaces.**
  - Add owner-bound global/per-Post list and exact detail storage reads,
    including Deleted Posts, with newest revision-ID cursor semantics and
    mismatched Post/revision rejection.
  - Project exact DTOs: metadata has revision/Post IDs, snapshot title/slug,
    capture time, capture-time snapshot lifecycle, and current Deleted status;
    Current state has current ID/title/slug/format, all lifecycle timestamps,
    and request-time lifecycle; detail has the complete snapshot.
  - Add server functions for `/history`, `/posts/{post_id}/history`, and
    `/posts/{post_id}/history/{revision_id}`. Register every wire endpoint and
    preserve typed/masked errors.
  - Build sidebar History navigation, active Post History actions, global list,
    per-Post Current state plus revisions, complete detail, and canonical Load
    more behavior. Render stored HTML only through the existing trusted sink.
  - Verification: storage and HTTP integration tests assert every field’s source
    and lifecycle clock, pagination gaps/duplicates, Deleted owner access,
    anonymous/stranger generic not-found behavior, route registration, and
    rendering safety.

- [ ] **5. Prove the full owner workflow and project implemented architecture.**
  - Focused Playwright coverage exercises sidebar and per-Post entry points,
    complete detail, Load more, semantic no-op, and Deleted Post history.
  - Re-run focused mutation/media tests as one integrated contract and verify
    SQLite/PostgreSQL behavior through repository-native lanes.
  - Update `docs/ARCHITECTURE.md` from implementation debt to current behavior;
    update CONTEXT/ADR citations only if implementation changes the approved
    language or decision. Do not touch generated ADR indexes.

## Completion criteria

- [ ] Every meaningful mutation has one atomic complete prior-state revision;
      semantic no-ops have none and move no timestamps.
- [ ] Revision reads are owner-only, complete, paginated, immutable, and
      available for Deleted Posts through all three approved routes.
- [ ] Current/Revision media references share one exact evidence/guard policy,
      with deduplicated owner Post IDs and explicit force behavior.
- [ ] Backup/restore, both backends, HTTP integration, and focused e2e prove the
      complete observable contract; #755 can consume retained Revision
      references.
