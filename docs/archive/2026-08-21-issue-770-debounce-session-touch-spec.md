# #770 — debounce session touch writes

Issue: [#770](https://github.com/jaunder-org/jaunder/issues/770). Milestone:
Correctness & data integrity. Provenance: #766 and ADR-0092.

## Summary

Every successful `SessionStorage::authenticate` call currently writes
`sessions.last_used_at`. That makes every authenticated request a database
writer, including read-only server functions, AtomPub requests, protected media
requests, and telemetry intake. On SQLite this is avoidable write-lock pressure:
#766 showed two session-touch writes starving for the full 5s `busy_timeout`
while another subsystem produced sustained write churn.

`last_used_at` is operator-facing session metadata. It does not participate in
authentication, expiry, or authorization, so it can be bounded-stale without
changing the session model. The fix is to touch only when the stored
`last_used_at` is older than a fixed freshness window.

## Context

- `SessionStorage::authenticate` hashes the raw token, captures `Utc::now()`,
  and delegates to `SessionDialect::touch_and_load`.
- SQLite implements `touch_and_load` as an unconditional update inside a
  transaction, followed by a joined load. The comment already exists because the
  operation is dialect-sensitive under SQLite concurrency.
- Postgres implements `touch_and_load` as one data-modifying CTE that updates
  and returns the joined session row.
- All ordinary authenticated request paths funnel through
  `SessionStorage::authenticate`, so this change intentionally applies to
  browser session cookies, Bearer tokens, Basic/App Password requests, and the
  separate telemetry intake guard.
- ADR-0092 identifies session-touch write amplification as remaining SQLite
  write-pressure after #766's feed-event batching fix. ADR-0021 remains
  load-bearing: SQLite must not reintroduce a read-then-write deferred
  transaction upgrade.

## Decisions

| ID     | Decision                                                                                                                                                                                                                                                                         |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **D1** | `last_used_at` is bounded-stale operator metadata with a fixed **60 second** freshness window. The value is still persisted in `sessions.last_used_at`; no new schema or configuration is introduced.                                                                            |
| **D2** | The debounce applies uniformly to every `SessionStorage::authenticate` caller: browser cookies, Bearer tokens, Basic/App Passwords, and telemetry intake. There is no transport-specific policy.                                                                                 |
| **D3** | A fresh authentication returns the stored `last_used_at`, not a synthesized `now`. Callers see the same value `list_sessions` would report.                                                                                                                                      |
| **D4** | SQLite keeps fresh authentication read-only. It first loads the session row outside any write transaction; if the stored timestamp is within the freshness window, it returns that row without issuing an `UPDATE`.                                                              |
| **D5** | SQLite stale authentication writes with a conditional update keyed by `token_hash` and stale cutoff, then reloads the row. If another request refreshes the same session first, the conditional update may affect zero rows; the reload returns the current persisted timestamp. |
| **D6** | Postgres preserves equivalent semantics with a dialect-specific conditional update and fallback load: update only while the current stored row is stale, otherwise return the existing fresh row.                                                                                |
| **D7** | Concurrency invariant: after a successful stale authentication, the persisted `last_used_at` is within the freshness window. It is not required that every stale caller's exact `now` win.                                                                                       |
| **D8** | No ADR. This is an implementation refinement of ADR-0092's named follow-up, not a new hard-to-reverse architectural rule. The durable architecture view should still mention bounded-stale session metadata.                                                                     |

## Acceptance criteria

- **AC1 — public semantics are updated.** `SessionStorage::authenticate` and the
  `SessionRecord.last_used_at` field doc no longer promise an update on every
  successful authentication. They and the session architecture docs state that
  `last_used_at` is bounded-stale operator metadata with a 60 second freshness
  window.

- **AC2 — fresh authentication is read-only on SQLite.** A dual-backend storage
  test proves a second immediate authentication returns the original stored
  `last_used_at`. On SQLite the implementation path for that case performs no
  `UPDATE`; the test or code structure must make this fresh no-write property
  reviewable rather than relying on timing.

- **AC3 — stale authentication advances the timestamp.** A dual-backend storage
  test sets or creates a session whose `last_used_at` is older than the 60
  second freshness window, authenticates it, and observes a persisted
  `last_used_at` within the freshness window.

- **AC4 — callers see persisted timestamps.** When authentication is skipped as
  fresh, the returned `SessionRecord.last_used_at` equals the stored timestamp;
  it is not replaced with the caller's `now`.

- **AC5 — stale races converge without extra churn.** The stale-path logic is
  conditional on the stored timestamp still being older than the cutoff, so a
  concurrent refresh can cause a later stale caller to reload and return the
  already-refreshed row instead of writing again.

- **AC6 — all auth surfaces share the policy.** The change stays inside
  `SessionStorage::authenticate` / `SessionDialect`; web, AtomPub, Bearer,
  Basic/App Password, and telemetry callers do not grow separate touch policies.

- **AC7 — SQLite transaction discipline is preserved.** The SQLite
  implementation does not perform a read followed by a write inside the same
  deferred transaction. Fresh rows are read-only; stale rows write with a
  write-first conditional update.

- **AC8 — the gate is green.** `cargo xtask validate --no-e2e` passes before
  pushing. The full local `cargo xtask validate` remains the ship gate before
  merge.

## Risks

- **Clock-bound tests can become flaky.** Tests should create or directly age a
  session row by more than the fixed freshness window instead of sleeping.
- **A conditional SQLite `UPDATE` alone may still contend as a writer.** The
  fresh path must load first and skip the update entirely when the stored row is
  fresh.
- **Postgres and SQLite may drift.** The public trait stays shared and the
  storage tests remain `#[apply(backends)]` so both dialects prove the same
  observable semantics.
