# Plan — #478: `FeedEventId` newtype

- Spec:
  [2026-07-16-issue-478-feed-event-id.md](../specs/2026-07-16-issue-478-feed-event-id.md)
- Issue: [#478](https://github.com/jaunder-org/jaunder/issues/478)

## Commit strategy (two commits, per precedent)

- **Commit 1 — define** `common::ids::FeedEventId` + unit test. Unused → green.
- **Commit 2 — thread** through storage + dialects + worker + tests. Ripples
  across crates → atomic. Lean on `cargo check --all-features --all-targets`
  **and full `cargo xtask check`** (both-backend tests — the Postgres `= ANY`
  path only compiles/runs under the pg feature).

## Task 1 — Define `FeedEventId`

- [ ] Append `pub struct FeedEventId(i64)` (doc-commented, derives) to
      `common/src/ids.rs`.
- [ ] Unit test (mirror `RevisionId`).
- **Verify:** `cargo test -p common ids::`; commit
  `refactor(common): add FeedEventId id newtype (#478)`.

## Task 2 — Thread `FeedEventId`

Edit-map (lines from `wt-base-issue-478`; verify before editing):

- [ ] **storage/feed_events.rs** — `FeedEventRecord.id` (:35);
      `FeedEventStorage` trait `enqueue -> Result<FeedEventId>` (:57),
      `mark_regenerated`/`mark_pinged`/`mark_failed`/ `mark_exhausted`
      `ids: &[FeedEventId]` (:73/:76/:81/:89); `FeedEventDialect` trait same
      `mark_*` params (:117/:120/:123/:131); `FeedEventStore::enqueue` —
      **retype the signature line `-> Result<FeedEventId, FeedEventError>`
      (:169)** AND wrap `Ok(FeedEventId::from(id))` (~:175); its `mark_*` impls
      forward `&[FeedEventId]` (the `if ids.is_empty()` guards unchanged).
      Import `FeedEventId`. `mockall` auto-regenerates. **In-module
      `#[cfg(test)] mod tests`** (AC2 surface — do NOT miss): `assert!(id > 0)`
      (~:275) → **`assert!(i64::from(id) > 0)`** (FeedEventId has NO `Ord` —
      `> 0` won't compile);
      `let ids: Vec<i64> = claimed.iter().map(|r| r.id).collect()` (~:402) →
      `Vec<FeedEventId>`. (`mark_failed(&[id], …)`/`mark_exhausted(&[id], …)`
      already match.)
- [ ] **storage/postgres/feed_events.rs** — `claim_pending_batch` decode wraps
      `FeedEventId::from(id)` at the `FeedEventRecord {…}` (~:79); the 4
      `mark_*` `ids:     &[FeedEventId]` params +
      **`let raw: Vec<i64> = ids.iter().map(|id| i64::from(*id)).collect();     … .bind(&raw)`**
      replacing `.bind(ids)` in the `= ANY($n)` queries (~:100/:110/:130/:143).
      **LEAVE `purge_corrupt(ids: &[i64])`** (~:20) — raw-cleanup, per spec.
- [ ] **storage/sqlite/feed_events.rs** — `claim_pending_batch` decode wraps
      `FeedEventId::from(id)` (~:86); the 4 `mark_*` `ids: &[FeedEventId]`
      params + `q.bind(*id)` → `q.bind(i64::from(*id))` (~:109/:122/:142/:158).
      **LEAVE `purge_corrupt`** (its `q.bind(*id)` at ~:33 stays `i64`). The
      `#[cfg(test)]` reproduction test (`enqueue`/`claim_pending_batch`,
      ~:200/:210) flows.
- [ ] **server/feed/worker.rs** — `let ids: Vec<i64>` → `Vec<FeedEventId>`
      (:153); **the two intermediate helper fns `ids` transits —
      `ping_websub(…, ids: &[i64], …)` (:198) and
      `on_regen_failure(…, ids: &[i64], …)` (:250) — retype their `ids` params
      to `&[FeedEventId]`** (`&ids` is passed into both at :180/:184; the
      `mark_*(ids)` calls inside them then match the new trait); test helper
      `event(id: i64, …) -> FeedEventRecord` — keep `i64` param, wrap
      `FeedEventId::from(id)` at the `FeedEventRecord {…}` construct (~:318);
      `mockall` `expect_mark_*().returning(|_| Ok(()))` closures ignore the arg.
- [ ] **web / server (enqueue return)** — `enqueue_feed_events`
      (`web/feed_events.rs`) and the worker self-enqueue **discard** the
      returned id (`… .await?;`) — verify no change needed.
- **Verify:**
  1. `cargo check --all-features --all-targets` green.
  2. AC2 edit-map struck; grep (`id: i64`, `ids: &\[i64\]`, `-> Result<i64`)
     over `feed_events`
     - `worker.rs` — each remaining hit is `purge_corrupt` / SQL string / a
       non-feed-event id.
  3. **`cargo xtask check` green** (clippy, coverage, **both-backend tests** —
     exercises both the Postgres `= ANY` and SQLite `IN` id binds).
- **Commit:** `refactor: thread FeedEventId through storage/worker (#478)`.

## Task 3 — Ship

- [ ] `cargo xtask validate --no-e2e`; cold-blind pre-merge review; rebase; PR
      (Closes #478); green CI; **HALT for merge approval**. (Completes the #457
      umbrella.)

## AC coverage

| AC  | Task                            |
| --- | ------------------------------- |
| AC1 | Task 1                          |
| AC2 | Task 2 (edit-map)               |
| AC3 | Task 1 serde test + Task 3      |
| AC4 | Task 2 (both backends) + Task 3 |
| AC5 | Task 3                          |
