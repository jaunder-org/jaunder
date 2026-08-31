# Bounded Transient Data Retention Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independent
> slices. This outline exists because the approved spec changes storage schemas,
> protocol concurrency semantics, credential retention, startup behavior, and
> both SQL dialects.

## Scope

In:

- Four approved surfaces: AtomPub Idempotency Keys, expiring credentials,
  terminal Syndication Feed events, and `media/tmp` crash recovery.
- Fixed policy windows, exact semantic cutoffs, bounded backlog draining,
  transition telemetry, backup/restore behavior, and SQLite/PostgreSQL parity.
- The proposed retention ADR and its architecture projection.

Out:

- Durable Posts, Post Revisions, tombstones, referenced media, sessions/App
  Passwords, `feed_cache`, and external diagnostic artifacts.
- Operator retention configuration or a generic retention framework.
- Feed retry classification, dead-letter UI, or redrive behavior owned by #1052.

## Task outline

- [x] Task 1: Add dual-backend schema support for bounded pruning
  - Contract: matching SQLite/PostgreSQL migration `0029` adds nullable
    `feed_events.terminal_at` in each backend's canonical instant type plus the
    cutoff indexes required by all database retention predicates. Non-terminal
    legacy rows keep `NULL`. Completed legacy rows use
    `COALESCE(pinged_at, migration_now)`; exhausted legacy rows use the single
    migration application instant, preserving a fresh seven-day window rather
    than disappearing on upgrade. Future transitions persist an explicit
    application-supplied instant.
  - Verification: backend-parametric migration tests prove schema parity, every
    status backfill, constraints, and indexes; backup/restore validation accepts
    and preserves `terminal_at`.

- [ ] Task 2: Enforce the one-hour AtomPub idempotency window atomically
  - Contract: Idempotency Key lookup and keyed Post creation receive an explicit
    `now`. Before the inclusive cutoff, replay is unchanged. At or after it, the
    stale mapping is retired and one concurrent create atomically establishes a
    fresh mapping without extending the old replay window. Post lifecycle data
    is untouched. Domain-owned pruning drains expired mappings in bounded
    batches.
  - Verification: dual-backend storage concurrency tests and AtomPub protocol
    tests cover before-cutoff replay, exact-cutoff reuse, competing creates,
    Deleted Posts, physical pruning, and restored expired mappings. Bounded
    metrics distinguish creation, replay, and expiry without key material.

- [ ] Task 3: Prune terminal credential rows without changing credential
      security
  - Contract: invite, email-verification, and password-reset stores each own a
    bounded cleanup operation taking explicit `now`. Consumed rows are eligible
    immediately; unused rows are eligible 24 hours after `expires_at`. Existing
    atomic claim, cheap-reject, and non-expiring session/App Password behavior
    remain unchanged. One failed credential store cannot suppress later stores.
  - Verification: backend-parametric tests cover exact boundaries, consumed and
    unused rows, rejection after pruning, failure isolation, and backup restore.
    Structured consumption telemetry uses stable non-secret identifiers only.

- [ ] Task 4: Bound terminal Syndication Feed event retention
  - Contract: completion and exhaustion persist their terminal instant.
    Completed rows are eligible immediately; exhausted rows are eligible at
    seven days. Cleanup never selects pending, claimed, or retryable rows and
    exposes no new inspection or redrive surface. Domain-owned cleanup drains
    bounded batches.
  - Verification: backend-parametric queue-transition and cleanup tests cover
    exact cutoffs, every non-terminal status, legacy backfill, restored rows,
    and completion/exhaustion telemetry without feed URLs or other unbounded
    fields.

- [ ] Task 5: Establish a clean media temporary directory at startup
  - Contract: after the runtime single-instance guard is successfully acquired
    but before upload handling is prepared, startup removes all artifacts under
    `media/tmp` and recreates a usable empty directory. A competing live
    instance refuses startup before cleanup and its files remain untouched.
    Cleanup failure is a typed fatal startup error; finalized and referenced
    media paths are unreachable from this operation.
  - Verification: filesystem behavior tests cover absent, empty, populated,
    nested, and cleanup-failure cases. Server preparation tests prove successful
    guard acquisition precedes cleanup, live-instance refusal performs no
    cleanup, and cleanup completes before uploads are accepted.

- [ ] Task 6: Integrate startup and daily database maintenance
  - Contract: the composition root runs database maintenance once during startup
    and schedules it every 24 hours. One explicit `now` freezes each run's
    eligible set. Every domain repeats fixed-size statements until that set is
    drained, releases locks between statements, reports counts/failures, and
    allows later domains to continue after one failure. Scheduling depends on
    domain interfaces directly; extract shared machinery only where Tasks 2–4
    have demonstrated the same contract.
  - Verification: worker tests with controlled time and failing stores prove
    startup/daily cadence, finite catch-up, per-domain continuation,
    retry-on-next-run, and bounded metric cardinality. Backend-parametric tests
    against real storage prove an independent writer progresses between cleanup
    batches, not merely that a fake store receives repeated calls. Server
    startup remains successful after database maintenance errors.

- [ ] Task 7: Reconcile the delivered architecture and full contract
  - Contract: the proposed ADR, `docs/ARCHITECTURE.md`, issue #393, and code
    state agree; no generic retention surface, configuration key, or
    durable-data purge entered the diff. Every exported-symbol change has all
    callers migrated.
  - Verification: focused storage/protocol/server suites cover the spec first;
    then `jaunder-commit` owns each commit gate and the final branch follows the
    repository validation ladder before review.

## Key contracts

- All cutoff comparisons use inclusive `cutoff <= now` semantics.
- Semantic expiry never waits for the cleanup scheduler.
- A maintenance run freezes one `now`, then drains the finite eligible set
  through repeated bounded statements with locks released between batches.
- Cleanup counts are bounded metrics; structured transition telemetry carries no
  token, Idempotency Key, email, feed URL, body, or unbounded attribute.
- Database cleanup is best-effort and isolated per domain; `media/tmp` startup
  cleanup is fatal.

## Risk checks

- Preserve `UNIQUE(user_id, key)` while making exact-cutoff idempotency reuse
  atomic under both database transaction models.
- Follow ADR-0021 and ADR-0092: no deferred SQLite read-to-write upgrade,
  per-row write loop, slow work inside a write transaction, or unbounded lock
  hold.
- Preserve backup/restore round trips and ensure restored timestamps cannot
  reactivate expired behavior.
- Keep credential classification and hash-work ordering from ADR-0018/ADR-0022.
- Do not race active uploads or traverse outside `media/tmp`.
- Keep #1052's recovery/redrive ownership intact.
- Maintain backend-parametric tests with `#[apply(backends)]`; no
  SQLite-memory-only substitute.
