# Issue #804: Seed DTO shape

## Outcome

Post seed data has one publication-state signal: `published_at`. Draft
presentation is derived from that signal, and timeline and unpublished listings
share one generic pagination envelope without otherwise changing their endpoint
payloads.

## Load-bearing decisions

- `RenderedPost` does not serialize a separate `is_draft` field.
- `RenderedPost::is_draft()` is exactly `published_at.is_none()`; a scheduled
  Post has a future publication time and is not a draft even when an unpublished
  listing includes it.
- Every live caller uses the derived method. Contradictory states are not
  representable in fixtures or production values.
- The two existing page structs become direct instantiations of one public
  `Page<Row>` envelope. The obsolete concrete names are removed rather than
  retained as aliases.
- The generic envelope preserves the existing `posts`, `next_cursor`, and
  `has_more` JSON keys, declaration order, values, and serde behavior for each
  endpoint independently.
- The distinct rendered and unpublished row DTOs remain distinct under ADR-0097;
  sharing their mechanical pagination envelope does not merge their content
  tiers.
- This change needs no new domain term or ADR. ADR-0097 already records the
  governing row distinction, while the envelope has no independent domain
  meaning.

## Acceptance

- Serialized `RenderedPost` values omit `is_draft`, and deserialized values
  derive draft state solely from `published_at`.
- Published and scheduled Posts report not-draft; Posts without `published_at`
  report draft.
- The contradictory `published_at: Some` plus `is_draft: true` fixture is gone.
- Draft-banner and Publish/Unpublish action behavior remains correct across
  draft and published transitions in the existing browser flow.
- Golden serialization proof shows each generic page instantiation emits exactly
  the same bytes its corresponding concrete page emitted before this change.
- All former `TimelinePage` and `UnpublishedPage` callers use `Page<Row>`
  directly; neither obsolete name remains exported or referenced.
- `cargo xtask validate` passes.

## Boundaries

- AtomPub's separate parsed `is_draft` lifecycle input is unchanged.
- The rendered and unpublished row DTO fields, content tiers, and endpoint
  semantics are unchanged.
- Listing membership is unchanged: unpublished listings may continue to contain
  both drafts and scheduled Posts.
- No backward-compatibility shim is provided for old consumers of Jaunder's
  internal seed/server-function JSON.
