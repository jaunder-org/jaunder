# Issue #1178 Implementation Outline

Execution: `jaunder-iterate`; delegate only through `jaunder-dispatch`.
Authoritative spec: `2026-09-02-issue-1178-direct-row-projections-spec.md`.

## Trigger

Storage decode boundaries span shared and dialect-specific queries. The outline
keeps direct-decoder contracts, per-row corruption handling, and helper removal
coherent across the cutover.

## Scope

In:

- Direct handwritten `FromRow` implementations for six final records.
- Every shared, SQLite, and PostgreSQL query/caller named by the spec.
- Removal of obsolete intermediate row shapes and forwarding helpers.
- Private `FeedCacheRowRecord` to `StoredFeedCacheRow` rename.

Out:

- Public contract renames, SQL/schema changes, or new decoder allowlists.
- Feed-cache semantic reconstruction changes.
- Subscriber/session/subscription/feed-event/authentication boundary changes.

## Tasks

- [x] **Audience and tag records decode directly**
  - Implement strict decoders beside `AudienceRecord`, `TagRecord`, and
    `PostTag`.
  - Retarget generic audience/tag queries and both dialects' shared post-tag
    queries.
  - Remove `AudienceSummaryRow`, `TagListRow`, `PostTagRow`, and pure relocation
    maps.

- [x] **Invite and retrieval user records decode directly**
  - Implement strict decoders beside `InviteRecord` and `UserRecord`.
  - Retarget invite listing and both retrieval-only user queries.
  - Remove `InviteRow`, `UserRow`, and their forwarding conversions.
  - Re-check helper reachability: keep the user builder for password-bearing
    authentication; delete invite-only parts/builders only when no caller
    remains.

- [x] **Media records decode directly without widening failures**
  - Implement the strict decoder beside `MediaRecord` and retarget singleton
    queries.
  - Keep list queries raw and call `MediaRecord::from_row` inside the existing
    per-row skip/report loop.
  - Remove `MediaRow`, `media_record_from_row`, and helper-only tests superseded
    by behavior tests.

- [x] **Retain and clarify semantic projections**
  - Rename private `FeedCacheRowRecord` and all its bounds/callsites to
    `StoredFeedCacheRow`; leave its parts, mapper, and error variants intact.
  - Confirm subscriber summary code has no fabricated intermediate and remains
    unchanged.
  - Confirm every listed exclusion remains structurally intact.

- [x] **Verify the complete storage cutover**
  - Run `cargo xtask test-local -- -p storage` for shared dual-backend storage
    behavior, including malformed typed columns and media skip handling.
  - Run the relevant server storage integration filters if the storage package
    does not exercise an affected public path.
  - Run `cargo xtask check`; the sqlx newtype decode gate must accept every new
    handwritten decoder without an allowlist entry.
  - Inspect the branch diff for every obsolete symbol and every direct-query
    migration, then commit through `jaunder-commit`.

## Key contracts

- Every new row access is one explicit typed `try_get`; row-free transforms
  occur only after those bindings; each decoder ends in `Ok(Self { ... })`.
- Column names and decoded role types match the existing SQL projections.
- Query ordering and public return types do not change.
- Media list corruption remains one-row-local and redacted.
- Feed-cache domain errors remain `FeedCacheError`, not collapsed into
  `sqlx::Error`.

## Risk checks

- Compile bounds remain valid for both backend row types.
- No helper deletion removes an authentication construction path.
- No query switches from per-row raw decode to all-or-nothing `query_as` where
  corrupt-row skipping is required.
- No production `allow`/`expect` suppression or decoder allowlist entry is
  added.
