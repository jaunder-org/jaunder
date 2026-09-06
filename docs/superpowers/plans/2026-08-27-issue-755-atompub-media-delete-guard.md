# Issue #755: AtomPub Media Delete Guard — Implementation Outline

> Execute in order with `jaunder-iterate`; delegate bounded implementation work
> through `jaunder-dispatch` only where it improves isolation or review.

## Scope and contracts

**In scope:** ownership proof; per-user Media Record materialization; atomic
delete/reclaim outcomes and callers; AtomPub's exact conflict document; web
force disclosure; SQLite/PostgreSQL parity; focused integration/browser proof;
and the domain-document projection. **Out:** extraction/proof-semantic changes,
protocol force, Emacs, Post Member deletion, generic AtomPub problems, retention
policy, backfill, migration, generated indexes, and `docs/README.md`.

**Prerequisite:** issue #1055 supplies full Post Revision media references.
Issue #755 is natively blocked by it and does not implement revision history.

- **Proof:** a pre-write resolver accepts only `RenderOutput`-derived
  `MediaReference` forms and returns capability-only `ProvenLocalMediaRefs` with
  exact identities. Relative is local; absolute/scheme-relative is local only
  after exact live-instance proof. Foreign/unknown/malformed/unproven forms do
  not enter it. Probe before locks.
- **Materialization:** Post service carries that capability, not caller-supplied
  references, `PersistedMediaReference`, or `PostId` evidence, through
  `perform_post_creation`/`perform_post_update` into `PostStorage`. Active
  content create/update idempotently copies the canonical exact source
  row—earliest `created_at`, breaking ties by lowest `user_id`—and preserves
  `source`, `content_type`, `size_bytes`, `source_url`, and `created_at`.
  Missing source succeeds; publication-only does nothing; no backfill.
- **Retention:** the owner's current active/Deleted Post and Revision references
  trigger the ordinary owner guard. Qualifying cross-user references have their
  own records and do not pin. Foreign/unknown/legacy cross-user rows without a
  record remain nondisclosing global safety.
- **Delete/reclaim:** one locked atomic storage operation reports deleted,
  missing, owner retained-history, or global safety. Owner result has unique
  ascending current/revision Post IDs; global has `[]`. Reclaim takes a
  storage-owned `ReclaimGuard`; its transaction and media lock remain live while
  the manager owns filesystem unlink, including with SQLite's immediate
  transaction.
- **Concurrency:** delete wins if delete/reclaim removes the last matching
  source before a Post writer acquires its media lock: the write succeeds with a
  broken link and no Media Record. If a matching source exists under that lock,
  writer-wins materializes the record before unlock.
- **Locks:** Post create locks proposed identities; update locks old/new union;
  delete and reclaim lock targets in the same stable per-media order (PostgreSQL
  advisory locks; SQLite immediate/single writer).
- **Surfaces:** AtomPub exposes no force. Web force may knowingly delete the
  owner's final record past retained history, but never global safety.

## Ordered work

- [x] **1. Establish and prove one proof/materialization seam.**
  - Add the resolver from `RenderOutput`-derived `MediaReference` forms whose
    only constructible output is capability-only `ProvenLocalMediaRefs`; keep
    exact identities private to it. Carry it through
    `perform_post_creation`/`perform_post_update` to both `PostStorage`
    transactions, never admitting caller-supplied refs, persisted rows, or
    Post-ID evidence as a substitute.
  - Insert the author-owned row from the canonical exact source after ordered
    locks, in the Post transaction with references. Select earliest
    `created_at`, then lowest `user_id`; copy every named source metadata field
    unchanged rather than fetching, rederiving, or inventing metadata.
  - Add backend-parametrized checks for local/proven versus unproven forms,
    earliest-`created_at`/lowest-`user_id` selection, equality of all five
    copied fields, missing-source success, publication no-op, no backfill, and
    persistence through reference/Post deletion.

- [x] **2. Make delete/reclaim one rich locked outcome, prove it, and migrate
      callers.**
  - Keep the conditional delete as policy. On failure, classify under the same
    lock/transaction: missing, owner current/Deleted Post/Revision guard, or
    global safety. Deduplicate/sort owner Post IDs.
  - Make reclaim use the same namespace/order through storage-owned
    `ReclaimGuard`; hold its transaction/media lock through manager-owned
    filesystem unlink, with an equivalent SQLite immediate-transaction lifetime.
    Do not create a second ownership-policy path or move unlink into storage.
  - Migrate AtomPub and web callers. AtomPub maps `204`/`404`/the exact `409`
    documents. Preserve typed storage/serialization causes into the existing
    masked AtomPub internal-error path; prove that failure separately from a
    normal fail-closed refusal. Web force overrides owner retained history only;
    it must not claim or implement an override of global safety.
  - Add focused checks for owner current/Deleted/Revision IDs (unique
    ascending), qualifying cross-user non-pinning, global `[]`, exact AtomPub
    details, no protocol override, web-force success past owner retained history
    including a final record versus global refusal, reclaim-guard lifetime
    across unlink, and typed masked internal errors.

- [x] **3. Prove integrated concurrency and project the domain.**
  - In both backends, race Post writes with delete and reclaim; assert the two
    allowed serialized results: delete-wins yields a successful broken-link
    write with no Media Record when the source vanished before the writer lock,
    while writer-wins materializes when a matching source exists under it.
  - Add focused AtomPub integration and deployed browser-flow proof; update ADR,
    architecture projection, and glossary without generated indexes.

## Completion criteria

- [x] Every caller consumes the rich outcome; no boolean-force bypass or second
      ownership-policy path remains.
- [x] Both backends prove materialization metadata, current/Deleted/Revision
      reporting, global nondisclosure, reclaim-guard unlink lifetime, typed
      masked internal errors, and the two Post-write/delete/reclaim outcomes.
- [x] AtomPub's owner detail is exactly
      `Media is referenced by retained Posts or revisions. Use Jaunder's web media library to review references before deleting.`;
      its global detail is exactly
      `Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.`
