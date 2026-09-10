# ADR-0186: Creation returns a classified Saved Post extension

- Status: accepted
- Date: 2026-09-10
- Issue: [#1441](https://github.com/jaunder-org/jaunder/issues/1441)

## Context

[ADR-0097](0097-post-dto-content-weight-axis.md) requires Post wire types to be
named for the content weight they carry and records one `SavedPost` response for
create, update, publish, and unpublish. That shared response identifies the
saved Post and carries `published_at`, but it does not carry the server clock
against which a future publication instant was classified.

The new-Post form labels a confirmed creation as Draft, published, or Scheduled.
Recomputing that outcome later against the browser clock is incorrect at the due
boundary and under client/server clock skew. Adding request-clock metadata to
`SavedPost` would ship creation-only classification content on the other three
mutation paths.

## Decision

Create returns `ClassifiedSavedPost`, a content-oriented extension that nests
the existing `SavedPost` core and adds the server's closed `CreatePublication`
classification. The server derives that classification from the persisted
`published_at` and the creation request clock. Update, publish, and unpublish
continue to return `SavedPost` alone.

`CreatePublication` follows the repository closed-string-enum convention and
uses the lowercase wire tokens `draft`, `published`, and `scheduled`.

This amends ADR-0097's statement that one `SavedPost` serves all four Post
mutations. It retains that ADR's content-weight naming rule and its prescribed
shared-core-plus-extension shape: the extra field is sent only where a consumer
needs it, and the type name describes the additional content rather than the
transaction that produced it.

## Consequences

- Creation summaries remain consistent with the server's authoritative decision
  even when the selected instant becomes due before the browser renders the
  response.
- Existing update, publish, and unpublish consumers and wire payloads do not
  gain unused request-clock classification data.
- Creation consumers unwrap the nested `SavedPost` core for identity,
  publication instant, and permalink fields.
- Adding another creation classification requires changing the closed enum and
  its wire-contract tests rather than accepting an arbitrary string.
