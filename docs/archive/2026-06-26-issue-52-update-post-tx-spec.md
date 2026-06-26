# Issue #52 — `update_post` SQLite transaction discipline: design

- Date: 2026-06-26
- Issue: [#52](https://github.com/jaunder-org/jaunder/issues/52) — `storage(sqlite): update_post read-then-write transaction is SQLITE_BUSY-prone (ADR-0021)`
- Status: approved (design); awaiting spec review

## Problem

`PostDialect for Sqlite::update_post` (`storage/src/sqlite/posts.rs:25`) runs a
**read-then-write deferred transaction**:

```
BEGIN                                              -- sqlx pool.begin(): deferred, no lock
SELECT user_id, deleted_at FROM posts WHERE post_id = ?   -- auth: NotFound / Unauthorized
INSERT INTO post_revisions SELECT … FROM posts     -- snapshot the pre-edit revision
UPDATE posts SET … WHERE post_id = ? RETURNING …   -- apply the edit, return the row
replace_post_audiences::<Sqlite>(&mut tx, …)       -- DELETE + INSERTs of audiences
COMMIT
```

The opening `SELECT` takes a shared lock; the first write (`INSERT post_revisions`)
must upgrade it to a reserved lock against a possibly-stale WAL snapshot →
`SQLITE_BUSY` under concurrent writers that `busy_timeout` cannot rescue. This is
the `SQLITE_BUSY`-on-upgrade failure mode documented in **ADR-0021** (the same class
as #18).

This is not a single-statement collapse: three interdependent writes (revision
snapshot, post update, audience replacement) gated by a leading auth `SELECT`. Per
ADR-0021, such a site takes the write lock up front with `BEGIN IMMEDIATE`.

## Decision: take the write lock up front (apply the merged #51 precedent)

Reshape the SQLite implementation only — the Postgres impl is MVCC-safe (`SELECT …
FOR UPDATE`) and unchanged. The reshape mirrors the now-merged
`create_user_with_invite` fix (#51) and the `backup.rs` precedent:

1. Acquire a raw connection and issue **`BEGIN IMMEDIATE`** (write lock up front —
   no shared→reserved upgrade can fail). sqlx's `Transaction` cannot be used: it
   issues its own deferred `BEGIN`. Drive `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK`
   manually on the connection.
2. Run the existing statements against `&mut *conn`. **`replace_post_audiences` is
   unchanged** — it already takes `&mut DB::Connection`
   (`storage/src/posts.rs:1387`), so it is called as `replace_post_audiences::<Sqlite>
   (&mut *conn, post_id, &input.audiences)`.
3. Wrap the body in the `async {}.await` + `match` pattern so **every** exit path
   issues an explicit `COMMIT` or `ROLLBACK`. The two auth-fail paths (`NotFound`,
   `Unauthorized`) become `return Err(...)` inside the block (the outer match
   `ROLLBACK`s), replacing today's explicit `tx.rollback().await.ok()`. A pooled
   connection must never be returned with an open transaction.
4. The `UPDATE … RETURNING` `PostRow` is returned from the block; on success the
   transaction `COMMIT`s and the row converts via `post_record_from_row` **outside**
   the transaction (unchanged from today).

### Resulting shape (SQLite)

```
conn = pool.acquire()
BEGIN IMMEDIATE
  SELECT user_id, deleted_at … → None: NotFound; wrong owner / deleted: Unauthorized
  INSERT INTO post_revisions SELECT … FROM posts
  row = UPDATE posts SET … RETURNING …
  replace_post_audiences::<Sqlite>(&mut *conn, …)
  Ok(row)
COMMIT  → post_record_from_row(row)        -- (ROLLBACK on any Err)
```

## Why no ADR, no hash concerns

Unlike #51, `update_post` performs **no expensive work** (no Argon2 hash) inside the
transaction — it is all fast SQL. So #51's hash-position / DoS-amplification /
write-lock-across-hash tradeoffs (ADR-0022) **do not apply**. This change is the
plain ADR-0021 remedy; ADR-0021 is already on `main`, so no new ADR is written.

## Backend parity

Backend parity is a design invariant (ADR-0019). This change is SQLite-only; the
Postgres `update_post` (`storage/src/postgres/posts.rs:26`) is MVCC-safe and
untouched, so parity is preserved by construction. The behavior is guarded on both
engines by the existing `#[apply(backends)]` `post_update_*` tests (see Testing).

## Error paths preserved

`NotFound` (no row), `Unauthorized` (`owner_id != editor` or `deleted_at` set),
`Internal` (`post_record_from_row` failure, or an unexpected DB error) — each now
`ROLLBACK`s explicitly before returning.

## Testing

- **Behavior unchanged:** the existing dual-backend tests
  (`server/tests/storage/storage.rs`: `post_update_writes_revision_and_updates_record`,
  `post_update_not_found_returns_error`, `post_update_invalid_slug`,
  `update_soft_deleted_post`, all `#[apply(backends)]`) must stay green on **both**
  SQLite and Postgres. They cover the success path (revision written, record
  updated), `NotFound`, and the soft-deleted/`Unauthorized` path.
- **No in-file `#[ignore]` tests** in `storage/src/sqlite/posts.rs` (instrumented
  dialect file).
- **CRAP:** the manual-transaction restructure adds branches, so `update_post`'s CRAP
  baseline is expected to rise (complexity-only). Accept the bump in
  `crap-manifest.json` via the heal, as in #51.
- **New-uncovered:** if the rewrite relocates an untested error arm into the diff,
  add a focused dual-backend (or in-file SQLite) test to cover it rather than
  baseline it.
- **Gate:** `cargo xtask validate --no-e2e` per commit; full `cargo xtask validate`
  (sqlite + postgres e2e) at ship.

## Out of scope

- The Postgres implementation.
- Issue #53 (`tag_post`) — the sibling ADR-0021 follow-up, its own cycle.
- Any change to the SQL statements, error variants, or `replace_post_audiences`.
