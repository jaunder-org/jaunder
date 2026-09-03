# Issue #1178: Decode mechanical row projections directly

## Outcome

Storage queries decode six mechanical SQL projections directly into their
storage-owned records on both SQLite and PostgreSQL. Semantic reconstruction,
corrupt-row recovery, public storage contracts, and authentication behavior stay
unchanged; ambiguous private feed-cache nomenclature is corrected.

## Load-bearing decisions

### Direct-decoder eligibility

- A final record replaces an intermediate only when its handwritten
  `sqlx::FromRow` implementation satisfies the repository's strict grammar: flat
  local bindings, every row access written as a typed
  `row.try_get::<T, _>(column)?`, row-free transformations only after decode,
  and a final `Ok(Self { ... })`.
- `AudienceRecord` directly decodes `audience_id`, `name`, and `created_at`.
  Remove `AudienceSummaryRow` and its tuple relocation.
- `InviteRecord` directly decodes the five invite columns. Preserve the existing
  role-specific timestamp wrappers and unwrap them only after typed decode.
  Remove `InviteRow` and its forwarding conversion; retain any builder still
  used by non-retrieval paths.
- `MediaRecord` directly decodes its eight domain-typed columns. Remove
  `MediaRow` and `media_record_from_row`.
- `PostTag` directly decodes the shared four-column `SELECT_POST_TAGS` shape.
  Both dialect modules query `PostTag` directly, and the forwarding row mapper
  disappears.
- `TagRecord` directly decodes `tag_id` and `tag_slug`. Remove `TagListRow` and
  its tuple relocation.
- Retrieval-only `UserRecord` directly decodes the nine non-password columns
  used by `get_user` and `get_user_by_username`. Remove `UserRow` and its
  forwarding conversion, while retaining password-bearing authentication rows
  and any shared builder they still require.

### Intentional semantic boundaries

- `FeedCacheRow` does not become a direct `FromRow` target. Its stored columns
  require path recovery, representation reconstruction, and domain-specific
  `FeedCacheError::{UnrecoverableStoredPath,MismatchedStoredMetadata}` results
  that cannot be preserved by `sqlx::FromRow`'s error contract.
- Rename private `FeedCacheRowRecord` to `StoredFeedCacheRow`. This
  distinguishes the stored SQL shape from the established public `FeedCacheRow`
  contract and removes the contradictory `RowRecord` suffix without broadening
  the API.
- `SubscriberSummaryRecord` stays procedurally decoded from raw rows. There is
  no current `SubscriberSummaryRow` intermediate; the loop derives fallback
  labels and skips/reports invalid subscriber references without failing later
  valid rows.
- `SessionRow` keeps lossy `SessionLabel` repair. Subscription record decoding
  keeps identity/status derivation. `ClaimedRow` keeps corrupt feed-event
  classification and purge behavior.
- Password-bearing authentication rows and scalar/decision tuples remain outside
  this refactor.

### Naming

- Existing public contract names remain unchanged, including `FeedCacheRow` and
  `PostTag`; suffix uniformity does not justify an API rename.
- Final storage-owned records decode directly under their established names. A
  retained private SQL shape must describe its semantic role rather than use an
  inverted `RowRecord` compound; `StoredFeedCacheRow` is the only such rename in
  scope.

### Error and recovery behavior

- Valid rows produce byte-for-byte equivalent field values and ordering.
- Typed newtype decoding continues to surface the same `ColumnDecode` failures.
- `MediaRecord::from_row` is invoked per raw row in list operations so one
  corrupt media row is still warned about and skipped; singleton reads may query
  `MediaRecord` directly.
- No decoder moves semantic validation into a derive, adds an allowlist entry,
  or weakens the handwritten-`FromRow` structural gate.

## Acceptance

- The six adopted final records implement the strict handwritten decoder grammar
  and every corresponding query targets the final record directly.
- Their six obsolete intermediate aliases/structs and forwarding conversions are
  removed wherever no retained semantic caller needs them.
- The private feed-cache decoder is named `StoredFeedCacheRow`, and its existing
  semantic mapper and public error variants remain intact.
- Subscriber summary decoding and every listed exclusion remain structurally and
  behaviorally unchanged.
- Backend-parity tests preserve malformed-newtype errors, tag/audience/user/
  invite retrieval, and media corrupt-row skipping.
- The strict sqlx newtype decode gate accepts every new decoder without an
  allowlist exception.
- Focused storage tests and `cargo xtask check` pass.

## Boundaries

- No schema, migration, SQL projection, ordering, wire, rendering, HTML-trust,
  authentication, or public storage API change.
- No public `*Row` to `*Record` renames.
- No flattening of feed-cache, subscription, session, feed-event, password, or
  scalar decision boundaries.
