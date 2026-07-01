# ADR-0004: Pagination Strategy

- Status: accepted
- Deciders: mdorman, Gemini CLI
- Date: 2026-05-14

## Context and Problem Statement

Timeline views (Local, User, Federated) can contain thousands of items. Jaunder
needs an efficient way to paginate these results without the performance issues
associated with offset-based pagination.

## Decision Drivers

- Performance: Consistent query speed regardless of depth.
- User Experience: Support for endless scroll in the web frontend.
- Data Integrity: Avoiding "skipped" or "duplicate" items when new content is
  added during navigation.

## Considered Options

- Offset-based pagination (`LIMIT 20 OFFSET 100`): Easy to implement, but
  becomes slow as offset increases and is unstable if content changes.
- Cursor-based pagination: Uses an opaque cursor (typically a timestamp or ID)
  to fetch the next batch.

## Decision Outcome

Chosen option: "Cursor-based pagination", because it provides stable performance
at scale and ensures a smooth "endless scroll" experience without duplicate
items.

### Implementation Details

- API endpoints return an opaque cursor alongside each batch of results.
- The web frontend and other clients pass this cursor back to fetch subsequent
  batches.

## Consequences

- Good: Fixed-cost database queries.
- Good: Resilient to concurrent updates.
- Bad: Does not support "jumping" to a specific page number (which is irrelevant
  for timelines).
