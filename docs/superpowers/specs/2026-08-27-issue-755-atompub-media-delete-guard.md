# Issue #755: Guard AtomPub Media Deletion

## Outcome

AtomPub media Member deletion follows the same guarded storage decision as the
web library. It never offers force. A refusal identifies only the authenticated
owner's retained Posts and revisions when that is safe; otherwise it discloses
nothing. Web force is the explicit override for an owner retained-history guard,
not for global safety.

## Domain and lifecycle

- A **Media Record** is one persistent, per-user record for one exact
  `MediaRef`, created by upload or qualifying active-Post content writes. It is
  not a cache of current references and survives reference/Post deletion until
  its owner explicitly deletes it.
- Every active content create/update (draft and scheduled states included) sends
  the `RenderOutput`-derived `MediaReference` forms to a new pre-write resolver
  before storage locks. Its only constructible result is the capability
  `ProvenLocalMediaRefs`, carrying exact local identities: relative forms
  qualify intrinsically; absolute and scheme-relative forms qualify only after
  exact live-instance proof. Foreign, unknown, malformed, ambiguous, and
  otherwise unproven forms do not enter the capability.
- Post service carries that capability—not caller-supplied references, reused
  `PersistedMediaReference` rows, or `PostId` evidence—through
  `perform_post_creation`/`perform_post_update` into both `PostStorage`
  transactions. A qualifying identity idempotently creates the Post author's
  record only by copying a canonical exact source row in the same Post
  transaction. Canonical means earliest `created_at`, breaking ties by lowest
  `user_id`. The copied record preserves that row's `source`, `content_type`,
  `size_bytes`, `source_url`, and `created_at`; tests must observe equality of
  every field. An absent source succeeds without a record. No fetch or invented
  metadata.
- Publication-only writes do not resolve or materialize. This change neither
  backfills nor migrates existing Posts/reference rows.
- A cross-user qualifying reference gets its author's independent record and
  never pins, claims, or blocks deletion of the source owner's record.

## Delete semantics

- AtomPub `DELETE /atompub/{username}/media/{sha}/{filename}` is guarded. It
  accepts no force query/header/retry confirmation/other override.
- Storage makes deletion and safety one conditional delete, then—on failure—
  classifies missing, owner retained-history, or global safety under the same
  transaction and target media lock. Reclaim obtains a storage-owned
  `ReclaimGuard` lease before it unlinks: its transaction and media lock remain
  live until the manager finishes the unlink and releases/finalizes the guard.
  The manager owns filesystem unlink, never storage; SQLite provides the same
  lifetime with its immediate/single-writer transaction.
- Owner retained history is the authenticated owner's current active Post,
  Deleted Post, or Post Revision reference. It returns the unique ascending IDs
  of those Posts (an ID appears once even if current and revisions reference
  it). Cross-user qualifying references have independent records and do not pin.
- Global safety has no reportable IDs. It covers local, legacy, owned, unknown,
  ambiguous, near-match, concurrent, and foreign/unrecorded cross-user
  references that cannot safely be exempted. Proven foreign evidence alone does
  not refuse.
- AtomPub success is `204 No Content`; missing is `404 Not Found`. The web
  library's force action may delete past only an owner retained-history refusal,
  including the owner's final Media Record, knowingly breaking retained history.
  It never overrides global safety.

## AtomPub conflict representation

Every guarded refusal is `409 Conflict`,
`Content-Type: application/problem+json`, with exactly:

- `type`: `https://jaunder.org/problems/media-delete-conflict`
- `title`: `Media deletion refused`
- `status`: JSON number `409`
- `detail`: one exact string below
- `post_ids`: unique ascending JSON array of numeric Post IDs

Owner retained-history refusal:

```text
Media is referenced by retained Posts or revisions. Use Jaunder's web media library to review references before deleting.
```

`post_ids` contains only the authenticated owner's current/revision Post IDs.

Global-safety refusal:

```text
Media deletion is blocked because Jaunder cannot prove that removing this record would preserve referenced media.
```

`post_ids` is `[]`. This does not promise web force success.

Unexpected storage/serialization failures retain typed internal causes and use
the existing masked AtomPub internal-error path. Evidence uncertainty is a
normal fail-closed refusal.

## Backend and concurrency contract

- SQLite and PostgreSQL have identical observable materialization, copied
  metadata, deletion, refusal-reason, and Post-ID results.
- Ownership proof completes before locks. PostgreSQL uses one shared per-media
  namespace and stable global order; SQLite uses immediate/single-writer
  discipline. Post create locks proposed identities; content update locks
  old/new union; delete and `ReclaimGuard` lock their targets in that same
  order. The Post write transaction includes reference persistence and
  materialization.
- The serialized result is deliberately delete-wins: if delete or reclaim
  removes the last matching source before the writer acquires that media lock,
  the Post write still succeeds with its broken link and creates no Media
  Record. If a matching source exists under the writer's lock, the write
  materializes its record before releasing that lock.

## Acceptance

- Upload and qualifying active content creates/updates materialize persistent
  per-user records from an existing exact source and visibly preserve all five
  copied metadata fields; absent source succeeds without a record.
- Relative/local-proof cases materialize; foreign/uncertain cases do not.
  Publication-only writes no-op; no historical backfill occurs.
- AtomPub unreferenced/missing deletion is `204`/`404`. Retained current,
  Deleted Post, and Revision owner references yield the exact owner `409`,
  preserve the record, and report unique ascending IDs. Global safety yields the
  exact global `409` and `[]`.
- AtomPub has no override. Web force can knowingly delete past owner retained
  history (including a final record), but global safety remains non-overridable.
- Both backends prove the two allowed concurrent Post write/delete and Post
  write/reclaim results: delete-wins is a successful broken-link write with no
  Media Record when no matching source remains under the writer lock;
  writer-wins materializes the record when one does. Reclaim's storage guard
  remains live across unlink, and unexpected storage/serialization failures
  preserve their typed internal causes through AtomPub's masked internal-error
  path.

## Boundaries

In scope: the lifecycle/materialization, guarded deletion and AtomPub wire
contract, truthful web force, both backends, and their tests. Not in scope:
reference extraction/proof semantics, a protocol force extension, the Emacs
client, Post Member deletion, generic problem details, retention policy, or
backfill. Post Revision media references are supplied by issue #1055; this issue
is blocked until that prerequisite lands and does not implement revision
history.
