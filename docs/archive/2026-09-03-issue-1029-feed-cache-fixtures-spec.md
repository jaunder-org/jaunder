# Issue #1029 — centralize feed-cache fixtures

## Outcome

Tests construct valid feed-cache rows through one opinionated shared fixture.
Persisted setups use that fixture's storage-owning terminal operation, while
pure row tests can build without creating application state. Deliberately
malformed storage inputs remain explicit.

## Load-bearing decisions

- Add `SeedFeedCache` to storage test support with a required typed `FeedPath`.
- Infer feed format and content type from the path. Callers cannot independently
  override representation metadata or create a path/format mismatch through the
  fixture.
- Provide valid format-specific body, ETag, and timestamp defaults. Capture one
  current instant for both default timestamps.
- Expose fluent overrides only for `body`, `etag`, `updated_at`, and
  `generated_at`, matching current semantic variation.
- Provide `.build() -> FeedCacheRow` for pure row fixtures and tests that own
  the cache write or commit semantics under test. Provide
  `.seed(&AppState).await -> FeedCacheRow` for ordinary persisted setup; `seed`
  constructs through `build`, writes through the real cache storage operation,
  confirms success, and returns the inserted row.
- Migrate every current valid fixture setup in:
  - `server/tests/feed/feed_handlers.rs`, including
    `handler_cache_hit_serves_stored_body_without_regeneration`,
    `handler_rejects_corrupt_cache_hit_without_serving_or_rewriting_it`,
    `handler_if_none_match_returns_304`, and
    `handler_if_modified_since_returns_304_when_unchanged`;
  - `server/tests/feed/feed_worker.rs`, including
    `startup_catchup_regenerates_feed_for_go_live_while_down` and
    `startup_catchup_ignores_nonpublic_posts`;
  - `server/tests/storage/listing.rs::feed_urls_needing_catchup_returns_stale_feeds`;
  - `storage/src/posts/store.rs::continuation_reporting_feed_urls_needing_catchup_skips_a_row_whose_feed_url_no_longer_parses`;
  - `server/src/feed/handlers.rs::tests::sample_row`;
  - `storage/src/feed_cache.rs::tests::sample`,
    `upsert_then_get_roundtrips_adjacent_timestamp_roles_at_microsecond_precision`,
    and `second_upsert_updates_existing_body`.
- Preserve each caller's explicit values whenever they establish an assertion,
  cache freshness boundary, conditional-request precondition, or stored-row
  contract. Use fixture defaults only for incidental setup.
- Keep deliberate raw mutations of content type, ETag, and feed path explicit
  outside the fixture. Only each test's coherent precursor row uses the fixture.
- Keep
  `storage::feed_cache::tests::construction_rejects_representation_mismatching_feed_path`
  on direct construction because the path/representation mismatch is the
  constructor contract under test.
- Remove superseded local row-construction and persistence helpers.
- This test-support consolidation introduces no domain term or durable
  architectural decision; domain context, ADRs, and architecture documentation
  remain unchanged.

## Acceptance

- All current valid feed-cache fixtures named above use `SeedFeedCache`; no
  parallel local fixture constructor remains at those sites.
- Pure row fixtures and tests exercising cache write or commit semantics use
  `build` and retain their direct operation under test. Ordinary persisted setup
  uses `seed` and does not separately invoke the cache write operation.
- Format/content-type consistency is derived from the typed path, and the
  builder exposes only the four approved overrides.
- Deliberate malformed content type, ETag, and feed paths remain explicit in
  their storage-contract tests, as does the direct mismatched-construction test.
- Existing assertions, freshness boundaries, malformed-input behavior, and
  public production interfaces are unchanged.
- Affected focused storage and server tests pass on every configured backend.
- Focused `SeedFeedCache` contract tests cover RSS, Atom, and JSON path-derived
  representation defaults; equal default timestamps; all four overrides;
  non-persisting `build`; and a both-backend `seed` round trip that returns the
  inserted row.
- `cargo xtask check` passes.

## Boundaries

- No production cache API or persistence-policy change.
- No test-support module split owned by #963 and no structural move owned by
  #950 beyond the new fixture's normal focused module/export.
- No generalized representation builder, invalid-row escape hatch, or unrelated
  fixture migration.
