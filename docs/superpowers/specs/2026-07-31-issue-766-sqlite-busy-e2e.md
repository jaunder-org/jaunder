# Spec: e2e sqlite `database is locked` 500s — root cause and fix (issue #766)

- Issue: #766 (e2e sqlite/firefox `database is locked` 500s ejected PR #764 from
  the merge queue)
- Date: 2026-07-31 (rev 2 — mechanism re-derived from the run's OTLP traces
  after review refuted rev 1's seed-transaction story)
- Status: draft — awaiting approval

## Problem

Merge-queue run 30674367840 failed its `e2e (sqlite/firefox)` job: two requests
500'd with `database is locked` at ~5.1s latency (the 5s `busy_timeout`), and
the job failed. Constraint for this cycle: **find the root cause; do not raise
`busy_timeout`** — nothing the app does should need more than 5s, and loosening
the timeout would paper over a findable cause.

## Demonstrated mechanism (from the run's own traces)

Source: the `e2e-diagnostics-sqlite-firefox` artifact of run 30674367840 —
`capture/otel-traces.jsonl` (31,013 server spans, ADR-0011/0037) and
`playwright-report-sqlite.json`. This is measured, not inferred:

- The victims are two `storage.session.authenticate` spans (the per-request
  session-touch `UPDATE`), 07:09.99→07:15.03 (5039ms) and 07:10.70→07:15.83
  (5130ms) — both died with `SQLITE_BUSY` after waiting the full 5s
  busy_timeout.
- Throughout their entire wait, `storage.feed_events.enqueue` — a
  **single-statement autocommit write** (`sqlite/feed_events.rs:52`) — completed
  at **21–117 per second** (07:09: 54/s; 07:11: 117/s; 07:12: 92/s; 07:14:
  63/s), interleaved with long feed writes: `feed_cache.upsert` 739ms,
  `mark_regenerated` 596ms, `mark_pinged` 1716ms.
- There is **no gap >700ms** between consecutive storage-span completions in the
  window — ruling out any multi-second exclusive hold by another process (rev
  1's seed-transaction hypothesis is refuted; the seed batch is only the
  _trigger_).
- The write source is the feed worker's `go_live_pass`
  (`server/src/feed/worker.rs:76-94`): for each newly-live post, for each
  affected feed surface, it awaits one `enqueue` — an individual write-lock
  acquisition per (post × surface). The failing test's first act seeds **51
  published posts** via the seed tool; the next 10s tick fans them out into
  hundreds of enqueue writes in a tight loop.
- Mechanism: SQLite's busy handler is an unfair poll, not a queue. Under a
  sustained stream of short writes, a waiting writer can lose every poll for the
  full 5s. The two session touches starved while the enqueue storm (plus
  register/session writes from the parallel Playwright worker) cycled the lock.
  On a CPU-starved 2-core VM (firefox + WASM + Argon2 registrations) every hold
  stretches, making the storm dense enough to starve for 5s.

This matches ADR-0021's own context — "the always-on feed worker plus live
request handlers writing the single shared db" — recurring in a new shape: not a
lock _upgrade_ (ADR-0021's fix holds; the audit found no regressions), but lock
_churn_.

### Ruled out, with evidence

- **A single long write transaction** (seed batch, backup export): continuous
  server-side write completions through the window bound any foreign hold to
  <700ms. (Backup worker additionally never runs in e2e: it starts only at boot
  when `backup.destination_path` is set — `backup.spec.ts` sets a destination
  mid-suite via `update_settings`, but nothing restarts the worker.)
- **ADR-0021 upgrade regression**: every `begin()`/`BEGIN IMMEDIATE` site
  reachable from SQLite audited; all are write-first or immediate.
- **Pool exhaustion**: that surfaces as `PoolTimedOut`, not
  `database is locked`; and the ~5.0s latencies match busy_timeout exactly.
- Argon2-under-write-lock (`create_user_with_invite`, `confirm_password_reset`):
  present but **intentional** (ADR-0022 DOS cost-gating) and not implicated — no
  invite registration or password reset ran in the window. Out of scope.

## Deliverables

1. **Post the trace-derived diagnosis to issue #766** so the evidence chain
   (span timeline, rates, ruled-out alternatives) is durable.
2. **Fix: batch every feed-event fan-out.** Add
   `FeedEventStorage::enqueue_many`, implemented once in the generic store as a
   **write-first transaction looping the existing single-row INSERT** (no
   hand-built multi-row SQL, dialect-generic, ADR-0021-safe). Convert its three
   per-row fan-out callers (the write-loop audit found the same shape twice more
   beyond the worker):
   - `go_live_pass` steady-state branch (`server/src/feed/worker.rs:85-89`) —
     the #766 storm source;
   - `go_live_pass` catch-up branch (`worker.rs:80-82`);
   - `enqueue_feed_events` (`web/src/feed_events.rs:32-34`) — the **request
     path**: the identical per-surface loop runs synchronously inside all five
     post-mutation server fns (`web/src/posts/api.rs` create/update/publish/
     delete), 6 + 6·ntags autocommit writes per mutation. One `enqueue_many`
     call each — one write-lock acquisition per fan-out. `enqueue` (single)
     remains for one-off callers and fixtures; no production fan-out may loop
     it. Measured (prototype, WAL + 5s busy_timeout): the tx-wrapped loop is
     5–6× faster than the autocommit loop in total (one WAL sync) and bounds a
     concurrent writer's worst-case wait to one ~22ms hold per 1000 rows locally
     — orders of magnitude inside the 5s budget even under heavy CI-slowdown
     multipliers. A multi-row VALUES statement is ~3× faster still but buys no
     needed margin at the cost of hand-constructed SQL.
3. **Deterministic regression tests**: storage-level dual-backend tests for
   `enqueue_many` (multi-row insert lands claimable rows, duplicates pass
   through unchanged — the old loop never deduped and the drain groups by path,
   empty input is a no-op), plus mock-counted call-count tests at each converted
   fan-out (`expect_enqueue_many` called once, `expect_enqueue` never) — the
   structural property that prevents the storm from returning, with no timing
   races.
4. **ADR draft** (numberless, promoted at ship): extend ADR-0021's SQLite
   discipline — background fan-out loops must not issue per-row autocommit
   writes; batch them into bounded statements. Lock _churn_ is a failure mode
   alongside lock _upgrade_ and long _holds_.
5. **Follow-up issues** (filed, not fixed here):
   - The seed tool exports no OTLP and logs no transaction boundaries — its DB
     activity was invisible during diagnosis; give `test-support` the same
     tracing initialization as the server (the instrumentation gap this
     investigation hit).
   - Session-touch write amplification: every authenticated request writes
     `last_used_at`; debouncing would shrink the victim surface for any future
     write-pressure event. (Optional hardening, not this bug's cause.)
   - Per-tag write loops (audit findings, same class, smaller cardinality):
     `web/src/posts/api.rs:221-223` (tag_post per tag on create),
     `storage/src/posts.rs:426-431` (`apply_post_tag_diff`) and its inlined
     duplicate `server/src/atompub/posts.rs:296-301`; plus the notable
     test-tooling loops (`SeedRawPost` per-tag seeding,
     `server/tests/web/web_tags.rs:108` 60-write seed).
   - N+1 read on the regeneration path: `build_feed_items` calls
     `get_tags_for_post` once per post per feed regeneration
     (`server/src/feed/regenerate.rs:132-158`) — 15,594 such spans in the
     failing run's trace. Read-only (no lock), but it multiplies pool
     round-trips on the same hot background path.

## Non-goals

- Raising `busy_timeout` (explicitly rejected).
- Changing ADR-0022's hash-inside-validation placement (intentional DOS
  protection).
- App-side fair write queuing: heavier than needed once the storm is batched,
  and cannot order cross-process writers anyway.

## Cleanup

- `storage/tests/lock_prototype.rs` (the throwaway measurement backing the fix
  choice) must be deleted — or deliberately reshaped into a committed harness —
  by the plan; it must not land by accident.

## Acceptance

- `enqueue_many` exists (generic impl, dual-backend tested); all three
  production fan-outs call it once per pass (mock-counted, deterministic); no
  production per-row `enqueue` loop remains.
- Diagnosis comment on #766; ADR draft recorded; follow-ups filed.
- `cargo xtask validate` green, including all four local e2e combos.
