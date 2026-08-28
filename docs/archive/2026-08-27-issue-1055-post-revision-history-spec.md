# Issue #1055: Full Local Post Revision History

## Outcome

Owners can inspect a complete, immutable history of every meaningful state
transition for their Posts, including Deleted Posts. Revision persistence is
atomic with each mutation, suppresses semantic no-ops, and retains historical
media protection across SQLite and PostgreSQL.

## Load-bearing decisions

- A **Post Revision** is the complete state of a local Post immediately before
  one meaningful accepted mutation. It is not an event, diff, backup, or revert
  operation.
- Content, tag, audience, media-reference, publish, unpublish, and soft-delete
  mutations each create exactly one prior-state revision when they change the
  full semantic state.
- One top-level mutation that changes several fields creates one revision. Tags,
  audiences, media references, lifecycle state, and core Post fields therefore
  commit through one storage-owned transaction rather than independent writes.
- A semantic no-op creates no revision and changes nothing, including
  `updated_at`. Collection equality is order-independent after canonical domain
  normalization.
- Every complete revision preserves authored source and format, rendered
  representation, title, slug, summary, tags, audiences, exact media references,
  immutable Post creation time, prior modification time, publication state/time,
  and deletion state/time.
- A revision has its own immutable ID and capture time. Lists order newest-first
  by revision ID and use opaque keyset pagination; offset pagination is not
  introduced.
- Post Revisions are retained indefinitely under ADR-0136 and cannot be edited
  or deleted through product storage interfaces.
- Existing partial revision rows require no compatibility, backfill, synthesis,
  version marker, or UI fallback; deployment data contains none. The migration
  establishes the complete revision schema directly.
- Revision media references persist with an exact subject discriminator:
  `Current(post_id)` or `Revision(post_id, revision_id)`, plus media identity,
  reference kind, and complete form. Resolver evidence and the final conditional
  predicate match that complete subject key, so evidence for a current row can
  never exempt a concurrent Revision row.
- Current and Revision references participate in one ordinary media guard. Owner
  advisory reporting deduplicates Post IDs across subjects; another owner’s Post
  IDs are never disclosed.
- Explicit web force remains the administrative override and may knowingly break
  reconstruction of retained history. No purge or legal-erasure policy is
  introduced.
- Complete revisions and their child state participate in backup and restore
  with the same fidelity and backend parity as current Post state.

## Owner history surface

- `/history` is an authenticated, owner-only, newest-first revision list across
  all owned Posts. Each row contains `revision_id`, `post_id`, snapshot title
  and slug, capture time, snapshot lifecycle state, and whether the Post is
  currently Deleted. Snapshot lifecycle is derived using the capture time, so it
  is stable.
- `/posts/{post_id}/history` is owner-only. Its non-revision **Current state**
  summary contains Post ID, current title, slug, format, created/updated/
  published/deleted timestamps, and the derived
  draft/scheduled/published/Deleted state, followed by that Post’s newest-first
  revision metadata rows.
- `/posts/{post_id}/history/{revision_id}` is owner-only and displays the
  complete immutable prior-state snapshot, including authored source and
  rendered representation.
- The authenticated sidebar includes **History**. Active Post owner actions also
  include **History**; the global page remains the entry point for Deleted Posts
  that no longer have an ordinary edit/detail surface.
- History lists use the repository’s standard page-size bounds, overfetch by
  one, opaque cursor, and append-style **Load more** interaction.
- Anonymous, cross-owner, mismatched Post/revision, and nonexistent history
  reads return the same generic Post-not-found behavior. Public web, feeds, and
  AtomPub expose no revision data.

## Transaction and backend contract

- Revision capture, full-state equality, mutation application, and all revision
  child rows occur in one transaction after owner/deletion checks and the
  backend’s Post/media locks. Tags are canonically reconciled inside it, with
  all multi-tag locks acquired in ascending normalized-slug order.
- SQLite uses its immediate single-writer discipline. PostgreSQL locks the Post
  row, normalized tag slugs, and media identities in their established stable
  orders. Both produce the same observable state and revision IDs/order
  semantics.
- The storage interface accepts one complete desired mutation or dedicated
  lifecycle transition and owns compare/snapshot/apply. Callers cannot assemble
  partial revisions or write tags after the revision transaction.
- Publication-only, unpublication, tag-only, and soft-delete paths use that same
  revision discipline rather than bypassing it through standalone updates.
- Current and historical media references share one deletion/reclamation policy;
  no second revision-only guard is introduced.
- Expected authorization/not-found outcomes remain typed. Unexpected storage and
  serialization failures retain typed causes and cross web boundaries through
  existing masked internal errors.

## Acceptance

- Both backends store one complete prior-state revision for each meaningful
  content, tag, audience, media-reference, publish, unpublish, and soft-delete
  mutation, including multi-field mutations as one revision.
- Repeating any mutation with the same canonical full state creates no revision
  and leaves all Post timestamps unchanged.
- Revision detail round-trips every scalar and child field exactly; revisions
  are immutable and survive subsequent edits and Post deletion.
- Owner global and per-Post lists are newest-first, cursor-paginated without
  gaps or duplicates, and detail rejects a revision belonging to another Post.
- Owners can list and inspect history for a Deleted Post; anonymous and other
  users receive the generic Post-not-found response.
- The current-state summary reflects active, draft, scheduled, published, and
  Deleted Post states without treating current state as a revision.
- Current, Deleted Post, and Post Revision media references guard deletion and
  reclamation; owner reporting returns unique Post IDs, while web force remains
  the explicit reconstruction-breaking override.
- Backup and restore preserve complete revisions and all child collections.
- Focused end-to-end coverage exercises sidebar history, a per-Post history
  view, complete detail, pagination, a semantic no-op, and Deleted Post owner
  access.

## Boundaries

- No revert, restore-to-revision, revision edit/delete, public history, AtomPub
  history, inbound `ajr_entry_versions`, hard purge, or legal-erasure workflow.
- No legacy partial-revision compatibility or backfill.
- This issue supplies retained Post Revision media references required by issue
  #755; it does not implement #755’s AtomPub deletion response or per-user Media
  Record materialization.
