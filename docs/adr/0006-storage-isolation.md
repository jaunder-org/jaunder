# ADR-0006: Storage Isolation (Shared Ingestion vs. Private Copies)

* Status: accepted
* Deciders: mdorman, Gemini CLI
* Date: 2026-05-14

## Context and Problem Statement

In a multi-user system following external feeds, multiple users might follow the same source. Jaunder needs to manage this efficiently while ensuring user privacy and individual read state.

## Decision Drivers

*   Network Efficiency: Fetch external sources only once.
*   Privacy: One user's view of content must be isolated from another's.
*   Customization: Users must have independent read states, bookmarks, and retention settings.

## Decision Outcome

Chosen option: A tiered storage architecture consisting of a **Shared Ingestion Layer** and a **Private User Content Layer**.

### Implementation Details

1.  **Ingestion Layer (Shared)**: Raw fetched content, feed metadata, and actor caches.
2.  **User Content Layer (Private)**: Per-user copies of items, read state, bookmarks, and notifications.
3.  All user-layer tables carry a `user_id` column, and queries never cross user boundaries.

## Consequences

*   Good: "Good network citizen" behavior by deduplicating fetches.
*   Good: Strong privacy guarantees at the database level.
*   Bad: Higher local storage overhead due to per-user content copies.
*   Bad: Complexity in managing fan-out during ingestion.
