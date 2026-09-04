# Feed Window Settings Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch`. This outline exists because publisher generations,
> transactional cache invalidation, and dual-backend concurrency invariants must
> remain coherent.

## Scope

In:

- Configurable `feeds.min_items` and `feeds.min_days` snapshots for Syndication
  Feed selection, plus publisher-owned mutations from the existing CLI surface.
- SQLite/PostgreSQL-parity storage, generation fencing, cache invalidation, and
  redacted invalid-configuration diagnostics.

Out:

- WebSub delivery, web UI, schema migration, and new or revised ADR work.
- Any change to ADR-0139's hybrid-union selection policy.

## Task outline

- [x] Task 1: Make hybrid-window cutoff arithmetic total
  - Contract: `HybridWindow` keeps its existing inclusive, eligible-item,
    minimum-items-or-minimum-days union semantics; an unrepresentably old
    day-based cutoff represents all eligible history rather than an overflow.
  - Verification: focused `#[apply(backends)]` feed-selection coverage proves
    normal union behavior remains unchanged and the all-history overflow case
    neither panics nor narrows selection.

- [x] Task 2: Validate one coherent publisher/feed configuration snapshot
  - Contract: granular reads reject a malformed stored minimum; grouped
    feed-configuration and publisher reads obtain both minimums from one
    database snapshot and expose nothing when either is malformed. Their shared
    typed diagnostic carries the configuration key and validation reason, never
    the stored value; validation checks `feeds.min_items` before
    `feeds.min_days`. Defaults apply only when the respective value is absent:
    20 items or 30 fixed UTC 24-hour days.
  - Verification: focused `#[apply(backends)]` storage tests prove independent
    present/absent defaults, separate typed/redacted corruption failures for
    each key, jointly corrupt precedence, and coherent paired reads across a
    concurrent setting mutation.

- [x] Task 3: Move feed-window CLI mutations behind the publisher transaction
  - Contract: one publisher-owned mutation boundary accepts typed min-item/day
    set and unset operations; all production callers of those keys cut over to
    it. Within one `WriteTransaction`, every accepted operation—including a
    semantic no-op—persists the requested configuration change, advances the
    publisher generation, and deletes all cached Syndication Feeds before a
    confirmed result is observable. Generation-snapshot cache writes fence stale
    regenerations; failed writes preserve the prior coherent snapshot and
    indeterminate commits remain indeterminate to callers. The generic
    site-config API remains available only for unrelated keys.
  - Verification: focused `#[apply(backends)]` integration coverage proves CLI
    set/unset retains the companion value or restores its default and changes
    selected feeds; no-op invalidation/generation advancement; stale concurrent
    regeneration rejection; atomic rollback; and confirmed versus
    commit-indeterminate outcomes. Verify all exported min-setting callers use
    this boundary and run the repository gates through `jaunder-iterate` after
    the focused evidence.

## Risk checks

- Transaction-local invalidation deletes every seeded Syndication Feed cache
  entry with one zero-bind operation on both SQLite and PostgreSQL.
- Configuration write, generation advance, and cache invalidation share the same
  transaction: rollback exposes the old coherent snapshot, while an
  indeterminate commit is never reported as confirmed.
- A no-op mutation still establishes the new generation fence, so older feed
  work cannot commit after it; the cache writer compares its starting snapshot
  with the current generation.
- Diagnostics redact stored values and defaults, retain typed key-plus-reason
  information, and deterministically give `feeds.min_items` precedence.
- The cutover migrates every exported production caller for the two feed keys;
  no alternate mutation path can bypass publisher invalidation.
- ADR-0139 eligibility, ordering, inclusive cutoff, defaults, regeneration
  snapshot semantics, and minimum-window union selection remain unchanged.
