# Spec — #478: `FeedEventId` newtype

- Issue: [#478](https://github.com/jaunder-org/jaunder/issues/478) (sub-issue of
  the umbrella [#457](https://github.com/jaunder-org/jaunder/issues/457))
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../../adr/0063-domain-value-newtype-convention.md)
  (numeric-ID trailer)
- Date: 2026-07-16

## Problem

The feed-regeneration queue's row id (`feed_events.id`) is a bare `i64` — on
`FeedEventRecord.id`, `enqueue`'s return, and the `&[i64]` id-batch params of
the `mark_*` markers. Applies the umbrella's `IdNewtype` pattern (#471/…/#474)
to the **last** ID class. A genuinely distinct id — unrelated to `FeedItem.id`
(a `PostId`).

## Decision

Introduce `FeedEventId` per the shared convention; thread it through
`feed_events.rs`, both dialects, and the feed worker. Type-only; behavior and
wire shapes unchanged.

### The type

```rust
// common/src/ids.rs
/// A feed-regeneration queue row id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, IdNewtype)]
pub struct FeedEventId(i64);
```

`common::ids::FeedEventId`; From/Into/Display/FromStr + transparent serde. No
`Ord`.

### sqlx boundary — convert at the edge (the dialect wrinkle)

- **Reads:** `enqueue`'s `RETURNING id` scalar wraps `FeedEventId::from(id)`
  (`FeedEventStore::enqueue`); the `claim_pending_batch` decode wraps
  `FeedEventId::from(id)` at each `FeedEventRecord {…}` construct (both
  dialects).
- **Writes (the `mark_*` id-batch bind):**
  - **Postgres** uses `WHERE id = ANY($n)` with a **slice binding** —
    `.bind(ids)` needs `&[i64]`. With `ids: &[FeedEventId]`, convert first:
    `let raw: Vec<i64> = ids.iter().map(|id| i64::from(*id)).collect();` then
    `.bind(&raw)`. (4 markers: regenerated/pinged/failed/exhausted.)
  - **SQLite** builds a dynamic `IN (?, ?, …)` and binds each element:
    `q = q.bind(*id)` → `q = q.bind(i64::from(*id))`. (4 markers.)
- Generic bounds untouched; no migration; SQL column names unchanged.

## Scope

1. **common** — define `FeedEventId` in `ids.rs`.
2. **storage `feed_events.rs`** — `FeedEventRecord.id: FeedEventId`;
   `FeedEventStorage` object trait **and** `FeedEventDialect` dispatch trait:
   `enqueue(...) -> Result<FeedEventId>`;
   `mark_regenerated`/`mark_pinged`/`mark_failed`/`mark_exhausted`
   `ids: &[FeedEventId]`; `FeedEventStore::enqueue` wraps the scalar, its
   `mark_*` forward `&[FeedEventId]` to the dialect (the `if ids.is_empty()`
   guards are slice-generic — no change). The `mockall::automock` regenerates.
3. **storage `{sqlite,postgres}/feed_events.rs`** — `claim_pending_batch` decode
   wraps `FeedEventId::from(id)` at the `FeedEventRecord {…}` construct; the 4
   `mark_*` `ids: &[FeedEventId]` params + the per-dialect bind conversion
   (Postgres `Vec<i64>` + `.bind(&raw)`; SQLite `.bind(i64::from(*id))`). The
   `#[cfg(test)]` `enqueue`/`claim` reproduction test flows.
4. **server `feed/worker.rs`** —
   `let ids: Vec<FeedEventId> = recs.iter().map(|r| r.id).collect()` (`:153`);
   the `mark_*(ids/&ids)` calls flow; the test helper
   `event(id: i64, …) -> FeedEventRecord` keeps its `i64` param and wraps
   `FeedEventId::from(id)` at construct (mirrors the tag/post precedent); the
   `mockall` `expect_mark_*(...).returning(|_| Ok(()))` closures ignore the arg.

**Do not over-reach / deliberate leaves:**

- **`purge_corrupt(ids: &[i64])`** (both dialects) **stays `i64`**: the ids of
  rows whose `feed_url` failed to parse — collected raw during the claim decode,
  they never become a valid `FeedEventRecord`, and this best-effort
  `DELETE … WHERE id = ANY/IN` cleanup crosses no `FeedEventId`-typed boundary.
  (A raw-internal-cleanup path.)
- `attempts: i32` (not an id); `feed_path: FeedPath` (already a newtype). `web`
  consumers (`enqueue_feed_events`, the worker's self-enqueue) **discard**
  `enqueue`'s return (`… .await?;` as a statement) — no change.

## Acceptance criteria

- **AC1** `common::ids::FeedEventId` exists, derived per convention;
  unit-tested.
- **AC2** No feed-event id field/param/return is bare `i64` —
  `FeedEventRecord.id`, `enqueue` return, `mark_*` `&[FeedEventId]`, `worker`
  `ids` — except the `purge_corrupt` raw-cleanup and SQL strings. Plan edit-map
  is the completeness surface.
- **AC3** Wire byte-identical — `FeedEventRecord` is
  `#[derive(Debug, Clone, PartialEq, Eq)]` only (not `Serialize`); no client
  wire. Behavior unchanged (both dialects' id binds produce the same values).
- **AC4** Both backends compile and pass (the Postgres `= ANY` + SQLite `IN`
  paths both bind the same `i64` values); no migration.
- **AC5** `cargo xtask validate --no-e2e` clean; e2e green in CI.

## Tests

Construct via `FeedEventId::from(n)` (the `worker` `event()` helper). No new
behavioral tests.

## Risks

- The Postgres `= ANY` `Vec<i64>` conversion is the one non-mechanical spot
  (documented). This is the final umbrella track — after it, all 8 ID newtypes
  are in.
