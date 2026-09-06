# ADR-DRAFT: Per-user Media Records from local Post references

- Status: proposed
- Date: 2026-08-27
- Issue: [#755](https://github.com/jaunder-org/jaunder/issues/755)

## Context

[ADR-0090](../0090-media-references-extracted-at-render.md) makes sanitized
rendered HTML authoritative for media references, and
[ADR-0154](../0154-media-reference-live-ownership.md) establishes live-instance
proof for absolute and scheme-relative forms.
[ADR-0136](../0136-local-post-lifecycle.md) retains current Deleted Posts and
Post Revisions, and says their media references use the ordinary guard. None
establishes the per-user record a cross-user reference retains, or reconciles
that record with retained history.

Treating a reference as a claim on someone else's record transfers control: one
author could pin another's library entry. Deleting a source record after the
original reference disappears can likewise discard bytes a second author chose
to use. The distinction must survive concurrent Post writes, deletion, and
reclaim across SQLite and PostgreSQL.

## Decision

This decision amends ADR-0090 decisions 5 and 8, ADR-0154's shared-lock/delete
contract, and ADR-0136's retained-media consequence. Other decisions continue.

A **Media Record** is one persistent per-user record for one exact `MediaRef`.
Upload or a Post content create/update (including draft and scheduled states)
can create it; deleting references or Posts cannot. A pre-write resolver
consumes only `RenderOutput`-derived `MediaReference` forms and returns
capability-only `ProvenLocalMediaRefs`, whose exact identities are not
caller-constructible. Relative references qualify intrinsically. Absolute and
scheme-relative references qualify only after ADR-0154 exact live-instance
proof. Foreign, unknown, ambiguous, malformed, and unproven forms create no
record.

Post service carries `ProvenLocalMediaRefs`—not caller-supplied references,
`PersistedMediaReference` rows, or `PostId` evidence—through
`perform_post_creation_with_media_ownership`/
`perform_post_update_with_media_ownership` into both `PostStorage` transactions.
For a qualifying identity, the transaction idempotently inserts its author's
record only from the canonical exact source row: earliest `created_at`, breaking
ties by lowest `user_id`. It copies that row's `source`, `content_type`,
`size_bytes`, `source_url`, and `created_at` unchanged; these fields are
observable and must be tested. Missing source rows leave the Post write
successful, with no invented record or metadata. Publication-only writes do not
materialize, and existing Posts/reference rows are not backfilled.

The record is independent: a qualifying cross-user reference creates the
referencing author's record and never pins, claims, or blocks deletion of the
source owner's record. Foreign/unknown/legacy cross-user references that lack a
qualifying record instead remain nondisclosing global-safety evidence.

Post content writes, delete, and reclaim share the same media-key lock order:
Post create locks proposed identities, update locks old/new union, and
delete/reclaim lock targets. PostgreSQL uses transaction-scoped advisory locks;
SQLite uses its immediate/single-writer discipline. Proof finishes before locks.
The conditional `DELETE … WHERE … NOT EXISTS … RETURNING` remains the policy;
failed deletion is classified under its transaction and target lock. Reclaim
uses a storage-owned `ReclaimGuard` lease: its transaction and media lock remain
live while the manager performs filesystem unlink, then the manager
releases/finalizes it. Storage owns the guard and policy, not unlink; SQLite
maintains the same lifetime with its immediate transaction.

ADR-0136's owner references from a retained current active Post, Deleted Post,
or Post Revision trigger the ordinary owner guard and report unique ascending
owner Post IDs. The web force action is the explicit override: it may delete
even the owner's final Media Record, knowingly breaking retained history.
AtomPub offers no force. Global safety remains non-overridable: foreign,
unknown, legacy, local, owned, near-match, or concurrent evidence that cannot be
safely exempted refuses without IDs. Proven foreign evidence alone does not
refuse.

## Consequences

- Retention is per-user and explicit. Removing a reference, deleting a Post, or
  publishing is not cleanup; no migration/backfill path exists for older rows.
- A qualifying copy neither fetches content nor fabricates metadata. Its tested
  metadata equality lets a user's retained record represent the actual source.
- Shared locks serialize Post writes with deletion and reclaim on both backends.
  The intentional race outcome is delete-wins: if delete/reclaim removes the
  final matching source before a writer acquires its media lock, the Post write
  succeeds with a broken link and no Media Record. If a matching source exists
  under the writer's lock, it materializes the record before releasing it.
- Retained owner history remains protected by the ordinary guard, but its
  reconstructibility is deliberately not absolute under explicit web force.
  Cross-user independence does not weaken global fail-closed safety.
- Storage returns a richer refusal, but disclosure is surface-specific: AtomPub
  can show only owner retained-history IDs or `[]`, never raw evidence or
  another user's IDs.
