# Issue #53 — `tag_post` SQLite transaction discipline: design

- Date: 2026-06-26
- Issue: [#53](https://github.com/jaunder-org/jaunder/issues/53) — `storage(sqlite): tag_post read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`
- Status: approved (design); awaiting spec review

## Problem

`PostDialect for Sqlite::tag_post` (`storage/src/sqlite/posts.rs:115`) runs a
**read-then-write deferred transaction** with interleaved reads and writes:

```
BEGIN                                              -- sqlx pool.begin(): deferred, no lock
SELECT COUNT(*) > 0 FROM posts WHERE post_id = ?   -- existence check → PostNotFound
INSERT OR IGNORE INTO tags (tag_slug) VALUES (?)   -- ensure the tag row
SELECT tag_id FROM tags WHERE tag_slug = ?         -- read back its id
INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES (?, ?, ?)   -- create join row
COMMIT
```

The opening `SELECT` takes a shared lock; the writes force a reserved-lock upgrade
against a possibly-stale WAL snapshot → `SQLITE_BUSY` under concurrent writers. This
is the `SQLITE_BUSY`-on-upgrade failure mode documented in **ADR-0021** (the same
class as #18).

This is not a single read-feeds-one-write shape (insert tag → read its id → insert
join row), so per ADR-0021 it takes the write lock up front with `BEGIN IMMEDIATE`.

## Decision: take the write lock up front (apply the merged #51/#52 precedent)

Reshape the SQLite implementation only — the Postgres impl is MVCC-safe and
unchanged. The reshape mirrors the now-merged `create_user_with_invite` (#51) and
`update_post` (#52) fixes:

1. Acquire a raw connection and issue **`BEGIN IMMEDIATE`** (write lock up front —
   no shared→reserved upgrade can fail). sqlx's `Transaction` issues its own
   deferred `BEGIN`, so drive `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` manually.
2. Run the existing statements against `&mut *conn` — **no SQL changes**. The
   optional `INSERT … RETURNING` tag-id collapse from the issue is **not** taken: it
   would not remove the transaction (the existence check and the join insert remain)
   and would diverge the SQL from Postgres.
3. Wrap the body in the `async {}.await` + `match` pattern so **every** exit path
   issues an explicit `COMMIT` or `ROLLBACK`. The current inline final-insert match
   folds in: `PostNotFound` (existence check), `AlreadyTagged` (unique violation on
   `post_tags`), and `Internal` (other DB error) become `return Err(...)` inside the
   block (the outer match `ROLLBACK`s); success returns `Ok(())` → `COMMIT`.

### Resulting shape (SQLite)

```
tag = tag_display.parse()?                          -- unchanged, before the tx
conn = pool.acquire()
BEGIN IMMEDIATE
  exists = SELECT COUNT(*)>0 …   → !exists: PostNotFound
  INSERT OR IGNORE INTO tags …
  tag_id = SELECT tag_id …
  INSERT INTO post_tags …  → unique violation: AlreadyTagged; other DB err: Internal
  Ok(())
COMMIT                                               -- (ROLLBACK on any Err)
```

## Why no ADR, no hash concerns

`tag_post` performs no expensive work (no Argon2 hash) inside the transaction — it is
all fast SQL. So #51's hash-position / DoS / ADR-0022 tradeoffs do not apply. This is
the plain ADR-0021 remedy; ADR-0021 is already on `main`, so no new ADR is written.

## Backend parity

Backend parity is a design invariant (ADR-0019). This change is SQLite-only; the
Postgres `tag_post` (`storage/src/postgres/posts.rs:104`, structurally identical,
MVCC-safe) is untouched, so parity is preserved by construction. Behavior is guarded
on both engines by the existing `#[apply(backends)]` tag tests (see Testing).

## Error paths preserved

`PostNotFound` (existence check false), `AlreadyTagged` (unique violation on the
`post_tags` insert), `Internal` (tag parse failure before the tx, or an unexpected DB
error) — each in-transaction failure now `ROLLBACK`s explicitly before returning.

## Testing

- **Behavior unchanged:** the existing dual-backend tests
  (`server/tests/storage/storage.rs`: `tag_post_nonexistent_post_error` →
  `PostNotFound`, `retag_same_post_with_same_tag_fails` → `AlreadyTagged`, plus the
  many tagging success/round-trip cases, all `#[apply(backends)]`) must stay green on
  both SQLite and Postgres.
- **No in-file `#[ignore]` tests** in `storage/src/sqlite/posts.rs`.
- **CRAP:** the manual-transaction restructure adds branches, so `Sqlite::tag_post`'s
  CRAP baseline is expected to rise (complexity-only). Accept the bump in
  `crap-manifest.json` via the heal, as in #51/#52.
- **New-uncovered:** if the rewrite relocates the untested generic-DB-error `Internal`
  arm of the `post_tags` insert into the diff (the analogue of #51's line 210), add a
  focused in-file SQLite test that forces a non-unique DB error on that insert (e.g.
  rename a `post_tags` column so the insert fails) and asserts `Internal`, rather than
  baseline it.
- **Gate:** `cargo xtask validate --no-e2e` per commit; full `cargo xtask validate`
  (sqlite + postgres e2e) at ship.

## Out of scope

- The Postgres implementation.
- Any change to the SQL statements (no `INSERT … RETURNING` collapse).
- This is the last of the three ADR-0021 SQLite follow-ups (#51, #52, #53).
