# Promote repeated storage row tuples to named aliases

- Issue: [#700](https://github.com/jaunder-org/jaunder/issues/700)
- Milestone: Domain-value type safety (newtypes)

## Problem

`storage/src` still has inline `sqlx::query_as::<_, (...)>` row tuple shapes
whose positions carry domain meaning that the type name does not state. After
#686, the most important repeated shapes no longer erase id values to `i64`, but
the row tuples are still anonymous:

- token-state diagnostic rows use `(Option<DateTime<Utc>>, DateTime<Utc>)` for
  `used_at` and `expires_at`;
- post ownership/liveness pre-checks use `(UserId, Option<DateTime<Utc>>)` for
  the author and deletion state of a Post;
- tag listing rows use `(TagId, Tag)` for tag identity and canonical tag slug;
- post tag rows use `(PostId, TagId, Tag, TagLabel)` in both backend-specific
  `SELECT_POST_TAGS` queries before conversion through `post_tags_from_rows`.

Several single-use multi-column row tuples also read better with names because
their fields are summaries or key/value surfaces rather than throwaway local
values:

- `list_audiences` uses `(AudienceId, AudienceName, DateTime<Utc>)`;
- `list_subscriber_summaries` uses `(SubscriptionId, String)`;
- SMTP configuration export reads `(String, String)` key/value rows.

The existing storage convention is to give such rows a named boundary when the
row shape is meaningful: `helpers.rs` defines aliases and builders such as
`UserRow`, `InviteRow`, `MediaRow`, and `PostRow`, while local modules already
use named rows where a struct is warranted.

## Decision

Promote the repeated meaningful row shapes to named aliases or row structs in
the smallest module that owns the concept. Shared rows live in `helpers.rs`;
module-local rows live beside their queries. Do not put item definitions in
`mod.rs` files (ADR-0128).

The token-state row gets both a name and a neutral classifier. The classifier
must represent the shared state of a token lookup without baking in one caller's
error enum: missing row, already used, expired, or claimable. Existing
email-verification and password-reset error helpers may continue to map that
neutral state into their public errors, and the atomic password-reset paths may
continue to combine the mapped error with rollback handling.

Post ownership/liveness checks get one named row shape used by the SQLite and
PostgreSQL `update_post` paths. Tag listing gets one named row shape used by
both prefix and no-prefix list queries. Existing post tag rows get a named row
shape used by both backend-specific queries and the existing conversion helper.

Selected single-use multi-column rows also get names where the row name carries
meaning at the storage boundary:

- `AudienceSummaryRow` or equivalent for `list_audiences`;
- `SubscriberSummaryRow` or equivalent for `list_subscriber_summaries`;
- `SiteConfigExportRow` or equivalent for SMTP/site-config key/value export.

One-column tuples are out of scope for this issue. The known `(i64,)` existence
flags remain intentionally primitive unless another issue gives them a domain
type. `String` one-column config reads remain under the config-key work, not
this row-alias cleanup.

## Boundaries

This is a readability and type-boundary cleanup, not a storage behavior change.
SQL text, transaction boundaries, claim ordering, and public error behavior must
remain unchanged.

Do not mechanically name every inline tuple in `storage/src`. A row earns a name
when the tuple positions have durable meaning across a boundary, are repeated,
or are a local summary/key-value surface whose destructuring is clearer with a
name.

No new ADR is required. The work applies existing decisions: ADR-0019 for
generic storage/backend bounds, ADR-0063 and ADR-0071 for domain newtypes and
SQLx row typing, and ADR-0128 for module surfaces.

## Acceptance criteria

- The six token-state lookup sites decode through a named token-state row shape
  instead of spelling `(Option<DateTime<Utc>>, DateTime<Utc>)` inline.
- Token-state classification is centralized behind a neutral helper or enum that
  preserves the existing missing/already-used/expired behavior for email
  verification, password reset, invite validation, and atomic password reset.
- The two `update_post` ownership/liveness pre-checks decode through one named
  row shape instead of spelling `(UserId, Option<DateTime<Utc>>)` inline.
- Tag listing's prefix and no-prefix queries decode through one named row shape
  instead of spelling `(TagId, Tag)` inline.
- Existing post tag queries decode through a named row shape instead of spelling
  `(PostId, TagId, Tag, TagLabel)` inline at both backend sites.
- `list_audiences`, `list_subscriber_summaries`, and site-config key/value
  export use named row shapes without widening public API surface.
- Every generic storage impl that depends on a renamed tuple updates its
  `FromRow` bounds in the same change, per ADR-0019.
- No `mod.rs` gains item definitions.
- Existing storage tests continue to cover the behavior; add focused unit tests
  for any new token-state classifier that has non-trivial branching.
- `cargo xtask check` passes.
