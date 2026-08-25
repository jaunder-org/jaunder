# Issue #747: carry publication updates to storage binding

## Outcome

Post updates carry the existing three-state `PublishUpdate` value through the
storage API to each backend's SQL-binding boundary. The impossible four-state
representation disappears from public and intermediate storage inputs; each
dialect derives scalar SQL bind values locally. Publish, schedule, backdate,
retain-timestamp, and unpublish behavior remain unchanged.

## Load-bearing decisions

- `PublishUpdate` belongs to the posts storage contract and is exported from the
  storage crate surface, rather than being owned by post-service orchestration.
- `UpdatePostInput` carries `PublishUpdate` directly. No public or intermediate
  storage input may represent unpublish together with an explicit timestamp.
- SQLite and PostgreSQL convert the sum only beside their SQL parameter binding;
  both dialects preserve the same three-state semantics required by ADR-0027.
- Existing web and AtomPub boundaries continue deciding which publication state
  the request means; this change does not alter their wire contracts.
- Creation-input work is removed from this issue. ADR-0090's `RenderOutput`
  invariant supersedes the original W1 proposal, so `RenderedPostContent`,
  `render_post_input`, and `CreatePostInput` remain unchanged.
- This reinforces ADR-0027 and ADR-0090; it introduces no new architectural
  decision and requires no ADR.

## Acceptance

- `UpdatePostInput` has one `PublishUpdate` field and no flattened publication
  fields or precedence comment.
- `PublishUpdate` reaches both SQLite and PostgreSQL update implementations
  intact and is converted only at their SQL-binding boundary.
- All production callers and test-support builders construct only one of the
  three valid publication update states.
- Dual-backend tests prove explicit scheduling/backdating, retaining an existing
  timestamp, stamping a previously unpublished Post, and unpublishing.
- The repository's required validation gate passes with backend parity intact.

## Boundaries

- No publication-state behavior, SQL precedence, wire format, or timestamp
  policy changes.
- No post creation-input changes, DTO renames, `summary_label` work, or
  unrelated post-type consolidation.
- No compatibility alias or deprecated path remains after the type move.
