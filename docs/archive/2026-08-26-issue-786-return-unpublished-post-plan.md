# Return the Post Written by Unpublish — Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded work through
> `jaunder-dispatch` when useful. This outline exists because the change carries
> storage-concurrency and dual-backend `UPDATE … RETURNING` invariants.

## Scope

In:

- Atomically guard standalone unpublish by Post ID, owner, and live state.
- Return the complete draft-state `PostRecord` from the mutation on both
  backends.
- Migrate the web endpoint and mocks to consume that record.
- Pin complete-row, no-edit, rejection, and rejected-no-write behavior.

Out:

- Publish, soft delete, AtomPub update, general editing, schema, revision
  policy, wire shape, and permalink rules.
- Any `updated_at` write or follow-up read used to manufacture the returned row.

## Task outline

- [x] Deliver the guarded row-returning unpublish vertical.
  - Contract:
    `PostStorage::unpublish_post(post_id, user_id) -> Result<PostRecord, UpdatePostError>`;
    PostgreSQL and SQLite each execute an owner- and live-filtered
    `UPDATE … RETURNING` with the same full projection, canonical author
    username, and slug-ordered tags. A zero-row update maps live foreign Post to
    `Unauthorized` and missing/deleted Post to `NotFound` without exposing owner
    identity. The mutation clears only `published_at`.
  - Caller contract: `web::posts::unpublish` performs no pre-write Post read or
    local state replay; feed-event inputs and `SavedPost` come from the returned
    row. Storage `Unauthorized` remains masked as not-found at the web seam. All
    mocks and callers cut over with no compatibility path.
  - Verification: dual-backend storage coverage proves the complete row,
    unchanged `updated_at` and revision count, all three rejection variants, and
    unchanged publication state after rejected calls. The existing
    `unpublish_post_returns_the_draft_permalink` test remains unedited and
    passes; the existing non-author integration assertion still proves not-found
    masking; the focused post-update/unpublish integration lane passes.

## Risk checks

- The owner/liveness predicate is part of the write, not a check followed by an
  unrestricted update.
- The returned row is the `RETURNING` result of that write; no later `SELECT`
  can race it.
- PostgreSQL and SQLite projections decode the same `PostRecord`, including
  backend-specific ordered tag JSON.
- Rejected calls write nothing, including the foreign and soft-deleted cases.
- Success does not change `updated_at`, revision count, content, slug, summary,
  tags, audiences, or media references.
- Feed invalidation uses the returned author identity and tags; the client still
  receives the same `SavedPost` and navigates to `/drafts`.
- `publish_post`, `soft_delete_post`, `perform_post_update`, and AtomPub callers
  remain untouched.
