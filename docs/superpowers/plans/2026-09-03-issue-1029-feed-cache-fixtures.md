# Feed-cache fixture consolidation implementation outline

> Execute with `jaunder-iterate`; delegated tasks use `jaunder-dispatch`. This
> outline exists because the shared test-support API owns storage persistence
> and its callers exercise storage transaction semantics across multiple crates.

## Scope

In:

- Add the approved `SeedFeedCache` construction and persistence contract with
  direct fixture tests.
- Migrate every valid fixture named by the approved specification.
- Preserve explicit malformed storage inputs and direct operations under test.

Out:

- Production cache API or persistence-policy changes.
- Independent representation overrides or invalid-row fixture support.
- Structural work owned by #950 or #963 beyond the focused fixture module and
  export.

## Task outline

- [x] Task 1: Deliver the shared feed-cache fixture contract
  - Contract: `SeedFeedCache::new(FeedPath)`; fluent `body`, `etag`,
    `updated_at`, and `generated_at` overrides; `build() -> FeedCacheRow`;
    `seed(&AppState).await -> FeedCacheRow`. Path-derived format/content type
    and a valid body specific to RSS, Atom, or JSON; one captured default
    instant; confirmed real storage write.
  - Verification: focused fixture contract tests cover each format's valid
    path-derived representation and body defaults, equal default timestamps,
    every override, non-persisting `build`, and both-backend `seed` round trips.
- [x] Task 2: Migrate storage and mock-handler fixture consumers
  - Contract: use `build` in pure row fixtures and wherever cache write or
    commit behavior is under test; use `seed` only for ordinary persisted setup.
    Preserve direct mismatched construction and raw content-type, ETag, and
    feed-path mutations.
  - Verification: focused `storage::feed_cache`, post continuation-reporting,
    storage listing, and feed-handler mock tests retain their existing
    assertions and pass on applicable backends.
- [ ] Task 3: Migrate server feed integration fixtures
  - Contract: handler and worker tests use `seed`; semantic body, ETag,
    updated-at, and generated-at values remain explicit through approved
    setters. Remove superseded local construction/persistence helpers only when
    their final caller is migrated.
  - Verification: focused feed-handler and feed-worker integration tests pass on
    every configured backend.

## Risk checks

- `FeedPath` remains the sole source of feed format and content type.
- `build` performs no write; `seed` performs exactly one confirmed write and
  returns the same row identity and values.
- Operation-under-test cases retain caller-owned write scopes and observable
  `MutationOutcome`, rollback, and indeterminate-commit behavior.
- Deliberate corrupt database values remain raw and visible at their test sites.
- Existing freshness thresholds, conditional-request inputs, and timestamp
  precision remain unchanged.
- No parallel local valid-row constructor survives in the named scope.
- `cargo xtask check` passes after all slices are integrated.
