# Publisher-side WebSub Recovery Implementation Outline

> Execute with `jaunder-iterate`, delegating bounded slices through
> `jaunder-dispatch`. This outline exists because issue #1052 changes durable
> schema, dual-backend queue concurrency, HTTP protocol behavior, operator
> authorization, and public CLI/web interfaces.

## Scope

In:

- One-row, two-phase feed-event lifecycle and dual-backend recovery storage.
- Coherent publisher snapshots, atomic hub/cache mutation, and stale-work
  fencing.
- WebSub response, redirect, and `Retry-After` classification.
- Phase-specific worker budgets and cache-before-publish recovery.
- Operator CLI and Admin WebSub inspection, configuration, and exact-ID redrive.
- Governing architecture projection and observable SQLite/PostgreSQL/protocol/UI
  proof.

Out:

- The transactional Post outbox already delivered by issue #1051.
- Multiple/per-topic hubs, inbound WebSub, exactly-once HTTP, configurable retry
  policy, Syndication Feed windows, and HTTP validators.

## Task outline

- [x] Task 1: Make feed recovery a durable dual-backend lifecycle
  - Contract: migrate each existing `feed_events` row without data loss into
    explicit regeneration/publication phases with independent counters,
    scheduling, diagnostics, and terminal states. Dead-letter queries take an
    explicit failed-phase selector and `PageSize` (default and maximum 50), then
    return event ID, Syndication Feed path, failed phase, phase attempt count,
    terminal time, and bounded operator diagnostic.
  - Contract: dead-letter pages use newest-first `(terminal_at, id)` keyset
    cursors. Exact-ID redrive is atomic. Stored diagnostics are capped at 1,024
    Unicode scalar values.
  - Contract: keep one event row from enqueue through completion; preserve stale
    claim recovery, PostgreSQL `SKIP LOCKED`, SQLite write-first discipline, and
    ADR-0167 terminal cleanup.
  - Verification: dual-backend integration coverage asserts every lifecycle
    transition, independent counters, pagination without skips/duplicates,
    all-or-nothing redrive, corrupt-row handling, and seven-day retention.

- [ ] Task 2: Make hub configuration coherent and race-safe
  - Depends on: Task 1's migrated event/cache schema and typed recovery records.
  - Contract: one publisher snapshot carries `FeedsConfig`, `SiteIdentity`, and
    an opaque monotonic hub generation. Actual normalized hub changes, including
    malformed-value repair, atomically advance the generation and delete every
    cached Syndication Feed; normalized no-ops change neither.
  - Contract: an optimistic generation check returns a typed stale-generation
    outcome when regeneration raced a hub change. A cross-process publisher gate
    serializes the final cache-commit/publish region against successful CLI/web
    hub mutations, without holding a database transaction across rendering or
    HTTP. The existing CLI and new web mutation must share this module rather
    than reproduce its invariants.
  - Contract: record the novel generation-plus-gate decision through
    `jaunder-adr` and project it into `docs/ARCHITECTURE.md`; `CONTEXT.md`
    changes only if implementation reveals genuinely new domain language.
  - Verification: dual-backend race tests prove coherent reads, compare-safe
    malformed repair, atomic invalidation, no-op preservation, typed stale
    detection, and no stale cache commit or old-hub ping after a configuration
    mutation succeeds.

- [x] Task 3: Classify the WebSub HTTP protocol at the client seam
  - Can run after Task 1 in parallel with Task 2; it owns only
    `server/src/websub/*` and focused protocol tests.
  - Contract: the injected WebSub client returns a typed retryable or terminal
    publication failure with an optional bounded retry delay; the worker does
    not decode raw HTTP status or header syntax.
  - Contract: all 2xx responses succeed; transport/timeouts and 408/429/5xx
    retry; other 3xx/4xx terminate. Follow at most three HTTP(S) 307/308 hops
    while preserving POST/form data. Parse delta-seconds and future HTTP-date
    `Retry-After`, cap at 24 hours, and use no remote-provided delay for invalid
    or past values.
  - Verification: deterministic local-hub protocol tests cover every status
    class, redirect method/body/hop/loop/Location rule, both date forms, cap,
    fallback, and typed source preservation.

- [ ] Task 4: Drive independent regeneration and publication attempts
  - Depends on: Tasks 1-3.
  - Contract: each grouped FeedPath attempt acquires one publisher snapshot and
    passes it through regeneration and publication. Snapshot read failures use
    the seven-attempt regeneration schedule; a typed stale-generation outcome
    immediately requeues from a fresh snapshot without charging either budget.
    Successful cache commit advances to publication; ordinary publication
    retries do not regenerate.
  - Contract: publication uses its ten-attempt local schedule and valid
    `Retry-After` override. No-hub completes after regeneration. Missing cache
    in publication returns the row to regeneration with a fresh regeneration
    budget. Terminal response, exhaustion, metrics, traces, and diagnostics name
    the correct phase.
  - Verification: worker tests prove both budgets and boundaries, no-hub
    completion, Retry-After scheduling, immediate terminal publication,
    missing-cache recovery, uncharged stale-generation restart, grouping, stale
    claims, continuation reporting, and cache-before-ping ordering.

- [x] Task 5: Expose scriptable WebSub recovery through the CLI
  - Depends on: Tasks 1, 2, and 4; owns CLI definitions, command dispatch, and
    focused command tests.
  - Contract: preserve `site-config set/unset` as the scriptable hub mutation
    path and add bounded dead-letter list/redrive commands over the shared
    recovery storage contract. Output distinguishes phase, emits every specified
    dead-letter field, and carries the stable cursor needed for the next page.
  - Verification: CLI tests cover set/change/unset/no-op, both phase filters,
    pagination, exact-ID atomic redrive, and scriptable errors for stale or
    invalid selections.

- [x] Task 6: Expose operator WebSub recovery through the admin web surface
  - Depends on: Tasks 1, 2, and 4; owns the new `websub` web module, route,
    navigation, server functions, and focused integration/e2e coverage.
  - Contract: add `/admin/websub` as an operator-only page with editable typed
    hub URL, separate regeneration/publication dead-letter views, stable
    pagination, exact-ID selection, all-or-nothing redrive feedback, and no
    internal detail exposed to nonoperators.
  - Verification: server integration tests cover typed arguments and
    authorization. Browser/e2e proof exercises configuration persistence,
    dead-letter paging/filtering, successful redrive, stale-selection rejection,
    and nonoperator denial while preserving the existing exact WebSub ping wave.

## Key contracts

- Task 1 owns durable event phase/state, counters, dead-letter records/cursors,
  diagnostic bounding, and redrive result types. Later tasks consume them
  without issuing queue SQL.
- Task 2 owns publisher snapshots, hub generation, cache invalidation, and the
  cross-process gate. CLI, web, and worker call it; none recreate comparison or
  fencing logic.
- Task 3 owns HTTP interpretation. The worker receives only typed disposition
  and optional retry timing.
- Task 4 owns scheduling policy and phase transitions. Operator surfaces request
  storage operations but do not implement recovery policy.
- Tasks 5 and 6 own presentation and authorization only; CLI and web behavior
  must agree because they consume the same deep storage interfaces.

## Risk checks

- Existing `feed_events` rows migrate deterministically: completed remains
  completed; exhausted remains a regeneration dead letter unless persisted
  regeneration proves publication was the failed phase; pending/claimed retain
  safe replay semantics.
- No database transaction spans rendering, filesystem work, or remote HTTP.
- The publisher gate works across the running server and separate CLI processes,
  has one stable acquisition order, releases on cancellation/panic/process exit,
  and cannot deadlock a hub mutation behind its own write scope.
- SQLite keeps bounded write-lock occupancy; PostgreSQL keeps nonblocking batch
  claims and exact-ID redrive serialization.
- Hub generation prevents ABA changes from validating an older snapshot.
- Redirect handling never changes POST to GET, never exceeds three hops, and
  never follows a non-HTTP(S) location.
- Wire inputs use existing validated newtypes and server-function decode policy;
  operator diagnostics remain masked, bounded, and secret-free.
- Every production hub mutation callsite migrates to the shared atomic
  operation; no generic set/delete path can silently bypass invalidation for the
  hub key.
- `FeedEventStorage`/write-transaction census, backup/restore schema validation,
  migration tests, metrics, docs, and both backend adapters move with the
  schema.
