# ADR-0092: Bounded write-lock occupancy on the SQLite path

- Status: accepted
- Date: 2026-08-01
- Issue: [#766](https://github.com/jaunder-org/jaunder/issues/766)

## Context

SQLite has one write lock per database, and waiting on it is an unfair poll:
`busy_timeout` retries on a timer, it does not queue. Issue #766 demonstrated
(from the failing run's own OTLP traces) the failure mode this enables: the feed
worker's go-live pass fanned 51 newly-live posts out as **hundreds of individual
autocommit `enqueue` writes** — 21–117 write completions per second through the
incident window — and two concurrent session-touch writes lost every retry for
the full 5s `busy_timeout` and surfaced as `database is locked` 500s. No single
hold exceeded ~700ms; the _churn_ did the starving.

ADR-0021 already disciplines transaction _shape_ (no deferred read-then-write
upgrades). This decision covers the two remaining dimensions of write-lock
occupancy that ADR-0021 does not:

- **hold duration** — how long one acquisition keeps the lock;
- **acquisition count** — how many times a single logical operation takes it.

Raising `busy_timeout` was rejected: nothing the application does legitimately
needs more than 5s, and loosening it would mask real occupancy bugs.

A prototype measured the alternatives for a fan-out of N single-row INSERTs
(WAL, 5s busy_timeout): one write-first transaction around the loop is 5–6×
faster in total than N autocommits (one WAL sync instead of N) and bounds a
concurrent writer's worst-case wait to a single ~22ms hold per 1000 rows; a
hand-built multi-row `VALUES` statement is ~3× faster still but buys no margin
that matters at these sizes.

## Decision

On any path a SQLite backend can execute, write-lock occupancy is bounded in
both dimensions:

- **No per-row write loops.** A fan-out or batch of writes issues **one**
  batched storage call — a write-first transaction looping the single-row
  statement (the `FeedEventStorage::enqueue_many` template), or an equivalent
  bounded statement. Per-item autocommit write loops are prohibited in
  production code, whatever the layer (storage, server, web).
- **No slow work inside a write transaction.** CPU-heavy computation (password
  hashing, rendering), unbounded batches, and foreign I/O do not run between a
  write transaction's first write and its commit. (Deliberate, documented
  exceptions stand: ADR-0022 keeps Argon2 inside the invite/reset claim windows
  as DOS cost-gating — a bounded, low-frequency path.)
- Batches are **bounded**: a batched transaction's size is capped by
  construction (a tick's fan-out, a page of rows), never unbounded input.

## Consequences

- `FeedEventStorage::enqueue_many` is the reference implementation; the feed
  worker's go-live pass dedupes its fan-out and enqueues it in bounded chunks
  (`ENQUEUE_CHUNK` URLs per transaction — a post-outage catch-up becomes many
  bounded holds, never one unbounded one), and the web request-path fan-out
  (`web::feed_events::enqueue_feed_events`) issues one call per mutation — both
  pinned by mock-counted tests (`times(1)` on `enqueue_many`, `times(0)` on
  `enqueue`).
- The remaining known per-row write loops are tracked for the same treatment:
  per-tag writes (#771). Related occupancy pressure: session-touch write
  amplification (#770) and the regeneration N+1 read (#772).
- Single-row `enqueue` remains for genuinely single-item callers and test
  fixtures; new fan-outs must not loop it.
- Builds on ADR-0021 (transaction shape) and ADR-0022 (validate before expensive
  work); changes neither.
