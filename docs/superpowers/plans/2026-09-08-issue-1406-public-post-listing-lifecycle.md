# Public Post Listing Lifecycle Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` when delegation is
> useful. This outline exists because asynchronous first-page replacement and
> Load-more completion require an explicit concurrency contract across the
> host-tested timeline state and wasm listing wiring.

Specification:
`docs/superpowers/specs/2026-09-08-issue-1406-public-post-listing-lifecycle.md`

## Scope

In:

- Generation-scoped timeline pagination claims that reject stale completions.
- One typed public Post listing lifecycle for User, site-tag, and User-tag
  routes.
- Migration from raw mutation revision signals to `Invalidator`.
- Existing host and browser coverage updated only where needed to prove the
  consolidated contract.

Out:

- Endpoint, PageSeed, cursor, projector, theme, or mutation wire changes.
- Optimistic row reconciliation or new mutation response data.
- A generic callback-driven listing framework or keyed reactive `Store`.
- Changes to home or cockpit presentation behavior.

## Task outline

- [x] Task 1: Make timeline pagination completions generation-scoped.
  - Contract: `TimelineState::advance_generation` returns an opaque
    `TimelineGeneration`. `LoadMoreClaim` captures that generation and the
    cursor; `TimelineState::append` requires the claim and applies success or
    failure only while its generation is current. The three-route public listing
    resource source advances the generation synchronously before it constructs
    each replacement fetch future. Other timeline callers do not advance
    generations and retain their existing behavior. No caller can append without
    presenting its claim.
  - Verification: host tests advance the generation before replacement fetch
    dispatch and cover every ordering between Load more, generation advance,
    replacement success, and replacement failure. They prove stale success and
    failure are inert while current-generation append/failure and
    duplicate-dispatch behavior remain unchanged. Compile checks migrate every
    `TimelineState` and `spawn_load_more` caller; focused existing behavior
    checks cover the unchanged home and cockpit paths.

- [x] Task 2: Replace the three public route lifecycles with one typed
      lifecycle.
  - Depends on: Task 1's `TimelineGeneration` and `LoadMoreClaim` settlement
    contracts.
  - Contract: `ListingRoute` remains the exhaustive route value and owns typed
    validation, seed matching, endpoint choice, and presentation data. The
    host-testable lifecycle owns route/seed/transition decisions; wasm wiring
    owns `Resource`, `Effect`, spawning, and view construction. The resource
    source tracks one local `Invalidator`, advances the timeline generation
    synchronously, and passes the typed route into the replacement fetch.
    Confirmed and commit-indeterminate Post outcomes notify the invalidator;
    rollback-confirmed operation failures do not.
  - Verification: host tests prove the route matrix, Username-first User-tag
    validation, exact seed matching, first-page replacement, and pre-dispatch
    generation advance. Host coverage exercises the actual Post settlement
    callback path for confirmed, commit-indeterminate, and rollback-confirmed
    outcomes, then proves successful revalidation replaces prior rows, cursor,
    and `has_more`. The focused web host suite and wasm checks pass. Existing
    browser scenarios prove all three route shells, discovery/RSD/subscription,
    Tag contexts, empty states, mutation refresh, Load more, and layout-shift
    behavior.

## Risk checks

- Advance generation synchronously in the replacement resource source before
  constructing its fetch future, not from the later destination `Effect`.
- Ignore stale success and stale failure; neither may modify rows, cursor,
  `has_more`, or status.
- Preserve current first-page failure behavior and Load-more retry behavior.
- Preserve theme adoption ordering and superseded-destination handling.
- Preserve malformed-route no-fetch behavior and User-tag error precedence.
- Keep route distinctions as validated `Username` and `Tag` values; no string
  branching or callback-injected endpoint policy.
- Keep host logic outside wasm-only component files and server functions in
  their existing vertical/API locations.
- Isolate generation advancement to the three public Post listing routes; shared
  home and cockpit callers receive only the claim-signature migration and retain
  their existing invalidation, identity, pagination, and paint behavior.
- Remove raw `mutate_version` wiring and every superseded per-route lifecycle;
  retain no alias, adapter, or compatibility path.
