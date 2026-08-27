# Return the post written by unpublish

## Outcome

Unpublishing is one owner- and liveness-guarded storage mutation that returns
the complete draft-state `PostRecord` it wrote. The web endpoint derives feed
events, permalink, and `SavedPost` from that returned row instead of replaying
the write onto a pre-write record.

## Load-bearing decisions

- Change `PostStorage::unpublish_post` to accept `post_id` and the authenticated
  `user_id`, returning `Result<PostRecord, UpdatePostError>`.
- Match `publish_post` error semantics: a missing or soft-deleted Post is
  `NotFound`; another user's live Post is `Unauthorized`.
- Put owner and live-Post predicates in the `UPDATE` itself. Authorization is
  not a preceding read followed by an unrestricted write.
- Clear only `published_at`. Unpublishing remains a publication-state transition
  rather than an edit: it does not change `updated_at`, create a revision, or
  touch content, slug, summary, audiences, tags, or media references.
- Return the complete `PostRecord` from the mutation's `RETURNING` projection,
  including canonical author username and ordered tags. Do not update and then
  reconstruct or re-read an independently mutable row.
- Keep the shared `PostStore` contract and the two backend SQL projections in
  parity. PostgreSQL and SQLite may differ only where their existing JSON tag
  aggregation requires it, under ADR-0019.
- When the guarded update matches nothing, distinguish `Unauthorized` from
  `NotFound` using the same pure-existence rule as `publish_post`; never read or
  expose the foreign owner's identity.
- Remove the web endpoint's pre-write `get_post_by_id`, owner/deleted check,
  mutable local record, and `published_at = None` replay.
- Build feed-event inputs and `SavedPost` exclusively from the returned
  draft-state record. Its permalink therefore uses `created_at` through
  `PostRecord::permalink()` by construction.
- Leave `publish_post` unchanged: it already returns a guarded `PostRecord`.
- Leave `soft_delete_post` unchanged. Delete returns no post payload and
  intentionally uses pre-deletion publication data for feed invalidation; it
  does not have unpublish's derived-permalink replay.
- No new ADR or domain glossary entry is needed; this applies existing
  generic-storage, SQLite transaction, Post, and permalink decisions.

## Acceptance

- `unpublish_post` has the guarded
  `(post_id, user_id) -> Result<PostRecord, UpdatePostError>` contract, and
  every caller and mock is migrated with no compatibility shim.
- Both PostgreSQL and SQLite execute an owner- and live-Post-filtered
  `UPDATE … RETURNING` that decodes the same complete `PostRecord` shape.
- A successful return has `published_at = None` and otherwise reflects the
  stored row, including author username and ordered tags.
- Missing and soft-deleted Posts return `NotFound`; a foreign live Post returns
  `Unauthorized` at the storage seam and remains masked as not-found at the web
  seam.
- `web::posts::unpublish` performs no pre-write Post read and no local field
  assignment; it uses the returned record for tag feed events, author identity,
  slug, publication state, and permalink.
- `unpublish_post_returns_the_draft_permalink` passes unmodified. Its
  backdated-publish setup still proves the returned permalink is the created-at
  draft permalink and differs from the prior published permalink.
- Focused dual-backend storage coverage pins the complete returned record,
  including canonical author username and slug-ordered tags, plus unchanged
  `updated_at` and revision count.
- The same coverage pins missing, soft-deleted, and foreign-live error variants
  and proves every rejected call leaves publication state unchanged, without
  weakening existing web integration coverage.
- The `SavedPost` wire shape, client navigation to `/drafts`, and feed-event
  behavior are unchanged.
- The applicable xtask verification ladder is green.

## Boundaries

- No `SavedPost`, endpoint, storage-schema, revision, tag, audience,
  media-reference, or permalink-rule change.
- No `updated_at` write during unpublish.
- No redesign of publish, soft delete, AtomPub update, or general post editing.
- No follow-up `SELECT` used to manufacture the returned mutation result.
