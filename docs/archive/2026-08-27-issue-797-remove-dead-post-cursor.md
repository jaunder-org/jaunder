# Remove the dead flat Post cursor parser

## Outcome

Jaunder no longer exposes or tests a storage helper for reconstructing a Post
pagination cursor from two independent optional values. Current pagination
behavior and wire contracts remain unchanged.

## Load-bearing decisions

- Delete `storage::parse_post_cursor`; its former half-cursor validation
  boundary became unreachable when post-listing server functions adopted one
  typed `PageCursor` argument in #569.
- Delete all three unit tests whose only subject is that helper, including the
  round-trip test that routes through it.
- Keep `PostCursor` and its legitimate constructors and projections. Storage
  queries still consume it, while `keyset_cursor`, `to_post_cursor`, and direct
  construction in storage tests remain valid.
- Keep half-cursor rejection at argument decode. A missing `PageCursor` field is
  rejected before a server-function body runs, consistent with ADR-0063 and
  ADR-0065.
- Correct comments that cite the deleted unit test; the server-boundary tests
  themselves continue to pin malformed and half-cursor rejection.
- Do not retain a generic parser for hypothetical flat inputs. A future protocol
  with that shape must introduce a boundary for its own cursor semantics then.
- No ADR or domain glossary change is warranted: this removes an obsolete,
  reversible implementation seam without changing architecture or vocabulary.

## Acceptance

- No production or test reference to `parse_post_cursor` remains.
- The helper and its three self-referential tests are absent together.
- Existing Post listing pagination and cursor decode tests pass unchanged apart
  from correcting the stale explanatory comment.
- The repository validation gate passes.

## Boundaries

- No wire schema, endpoint signature, storage query, or pagination ordering
  changes.
- No change to AtomPub `CollectionCursor`, scheduled-post cursor handling, or
  client traversal of opaque next links.
- No speculative replacement helper or compatibility alias.
