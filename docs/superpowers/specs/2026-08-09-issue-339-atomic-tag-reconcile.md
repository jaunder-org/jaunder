# Issue #339 — regression-test the atomicity of `set_post_tags`

## Status

Spec — awaiting approval.

## Background

Issue #339 asked for two things: make post-tag reconciliation atomic, and prove
it with a dual-backend concurrency regression test.

**The first half already landed**, via #771, which replaced the
non-transactional `apply_post_tag_diff` with a per-backend `set_post_tags`:

- `storage/src/sqlite/posts.rs:148` — `pool.acquire()`, `BEGIN IMMEDIATE`, then
  existence check, snapshot read, diff, writes, explicit `COMMIT` / `ROLLBACK`.
  `IMMEDIATE` takes the write lock **before** the read, so the read is not a
  shared→reserved upgrade (ADR-0021).
- `storage/src/postgres/posts.rs:138` — `pool.begin()`, then
  `SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE`, which locks the
  post row before the snapshot read and holds it through the writes under READ
  COMMITTED. It doubles as the existence check.

**The second half is missing.** Nothing in the repo exercises two interleaved
reconciliations. The invariant #771 established is therefore unguarded: a future
refactor that takes the snapshot before acquiring the lock — the pre-#771 shape
— or that drops `IMMEDIATE` / `FOR UPDATE` altogether, reintroduces the
lost-update window with a green suite.

This cycle delivers only the missing test and the test-support affordance it
needs. No production behavior changes.

## The invariant under test

> `set_post_tags` acquires its write lock **before** taking the tag snapshot it
> diffs against, and holds it through the writes. So its reconciliation is
> serialized against any other writer to the same post, and the committed result
> is exactly the desired set — never a union of two writers' sets, never a
> partially-applied diff.

Note the emphasis on **before**. Moving the snapshot read onto a pooled
connection but leaving it _after_ the lock acquisition is not a regression: the
read only runs once the lock is granted, so it still sees the rival's committed
writes. The regression this test guards against is specifically
**read-then-lock** (or no lock).

## Design

### Why not a plain race

Spawning two opposing `set_post_tags` calls and hoping they interleave is not a
regression test: on a broken implementation the pass/fail outcome depends on
scheduling, so a green run proves nothing. (`storage/src/media.rs:777`'s
`try_delete_media_holds_under_concurrent_reference_writes` is the repo's
existing stress-test precedent, and it explicitly disclaims proving atomicity —
it is a useful model for the spawn plumbing, not for the assertion.)

The test instead **forces** the interleave by holding, from the test itself, the
same lock `set_post_tags` takes.

### Shape

```
seed:     post with tags = {a}
held tx:  take the post write lock; add tag b to the post; do NOT commit
spawn:    set_post_tags(post, [c])
probe:    assert the spawned call has NOT completed within 300ms
held tx:  COMMIT
await:    the spawned call returns Ok
assert:   the post's tags are exactly {c}
```

The held transaction acts as a **rival writer**, which is what makes the final
assertion load-bearing:

- A correct implementation cannot take its lock until the rival commits. It then
  reads `{a, b}`, diffs against `[c]`, removes both, inserts `c` → **exactly
  `{c}`**.
- A **read-then-lock** implementation takes its snapshot first, on a connection
  that is not blocked. Under SQLite WAL and Postgres READ COMMITTED alike it
  sees the rival's _pre-commit_ state `{a}`, so `b` is never in `to_remove` and
  survives the reconcile → `{b, c}`. The final assertion fails.
- A **no-lock** implementation additionally completes during the probe window,
  so it fails the earlier assertion too.

### What each assertion is for

The two assertions are not redundant, and they are not equally strong:

- The **probe** (still-pending) is a _precondition_ check: it proves mutual
  exclusion exists at all. A read-then-lock implementation still blocks on its
  writes, so this assertion alone does not catch it.
- The **final tag set** is the _regression guard_: it is the assertion that
  distinguishes lock-then-read from read-then-lock.

The spec calls this out so a future reader does not delete the weaker-looking
one under the impression the other covers it, or vice versa.

### Lock granularity differs per backend, deliberately

- **SQLite**: `BEGIN IMMEDIATE` takes a database-wide write lock, so the test's
  held transaction excludes _any_ concurrent writer, not just one on this post.
- **Postgres**: `SELECT … FOR UPDATE` locks the post row, so exclusion is
  per-post.

Both are sufficient to serialize two writers on the same post, which is the
invariant. The test asserts the invariant, not the mechanism, so one test body
covers both backends; the difference is recorded in a comment on the affordance.

### Test-support affordance

`CloseablePool::execute` returns its connection to the pool as soon as the
statement finishes, so it cannot hold a lock across await points.
`storage/src/test_support.rs` gains, alongside `close()` / `execute()`:

```rust
/// Takes the same write lock `set_post_tags` takes, and holds it until the
/// returned guard commits or drops.
pub async fn lock_post_for_write(&self, post_id: PostId)
    -> Result<PostWriteLock<'_>, sqlx::Error>;

pub enum PostWriteLock<'a> {
    Sqlite(PoolConnection<Sqlite>),          // BEGIN IMMEDIATE issued
    Postgres(Transaction<'a, Postgres>),     // SELECT … FOR UPDATE taken
}

impl PostWriteLock<'_> {
    /// Adds one tag to the post from inside the held lock — the rival writer.
    pub async fn add_tag(&mut self, post_id: PostId, label: &TagLabel)
        -> Result<(), sqlx::Error>;
    pub async fn commit(self) -> Result<(), sqlx::Error>;
}
```

`post_id` is taken on both arms even though only the Postgres arm needs it:
SQLite's lock is database-wide. Passing it uniformly keeps the caller
backend-agnostic and documents the intent at the call site.

`add_tag` exists because the rival write is **not** one statement and **not**
one dialect. `post_tags` is `(post_id, tag_id, tag_display NOT NULL)` with a
foreign key to `tags(tag_id)`, so the rival must insert-or-ignore into `tags`,
select the `tag_id`, then insert into `post_tags` — and the upsert spelling
diverges (`INSERT OR IGNORE` vs `ON CONFLICT DO NOTHING`). Hiding that behind
`add_tag` is what lets the test body stay backend-agnostic.

It lives in `test_support` rather than the test module because the same "hold
the write lock and race a storage method" pattern is what #363 (make mutating
storage methods take a transaction) will need, and because backend dispatch
already belongs there.

### Environment facts the test depends on

These are load-bearing and currently implicit; the test carries them as comments
so a later change to any of them fails loudly rather than mysteriously.

1. **Pool capacity ≥ 2.** The test holds one pooled connection while
   `set_post_tags` acquires another from the _same_ pool (`TestBase.pool` is a
   clone of the `AppState` pool). Neither backend sets `max_connections`, so
   sqlx's default of 10 applies. At `max_connections = 1` the test would
   deadlock rather than fail.
2. **Blocked SQLite does not park a tokio worker.** The repo's `#[tokio::test]`
   default is the current-thread runtime. This is safe only because sqlx-sqlite
   runs each connection on its own OS thread, so a `BEGIN IMMEDIATE` waiting in
   SQLite's busy handler blocks that thread, not the runtime.
3. **SQLite `busy_timeout` is 5s** (`storage/src/sqlite/mod.rs:107-109`). A
   blocked `set_post_tags` retries for five seconds before returning
   `SQLITE_BUSY`, so the held lock must be released well inside that. The 300ms
   probe leaves ample margin. Note ADR-0021's caveat: `busy_timeout` does
   **not** rescue a shared→reserved upgrade, so a "drop `IMMEDIATE`" regression
   fails fast with `SQLITE_BUSY` rather than blocking — which the test also
   reports as a failure, just a different one.

The 300ms probe's failure direction is sound: a broken implementation completes
promptly inside the window and fails. A slow machine only delays a correct
implementation, which still passes.

### Placement — ADR-0053 §1

The test is `#[apply(backends)]` and proves a contract of `PostStore<DB>`, so
per ADR-0053 §1 it belongs in the **generic home module**,
`storage/src/posts.rs`, beside the other `set_post_tags` tests. #771 already
moved that family out of `server/tests/storage/mod.rs` for exactly this reason —
the tombstone comments at `server/tests/storage/mod.rs:3883` and `:4037` record
the move.

### No ADR

This introduces no new architectural decision. It locks in ADR-0021's existing
transaction discipline, and follows ADR-0019 (shared SQL, per-dialect files) and
ADR-0053 §1 (test placement) rather than amending either.

## Acceptance criteria

1. **AC1 — the affordance exists, is dual-backend, and hides dialect
   divergence.** `storage/src/test_support.rs` exposes `lock_post_for_write`
   returning a guard with `add_tag` and `commit`, with both a SQLite arm
   (`BEGIN IMMEDIATE`) and a Postgres arm (`SELECT … FOR UPDATE`). The guard's
   doc comment records the granularity difference. The test body contains no
   dialect-specific SQL.

2. **AC2 — the regression test exists, runs on both backends, in the right
   place.** A test in `storage/src/posts.rs`'s test module carries
   `#[apply(backends)]`, so it runs as both a `sqlite` and a `postgres` case.
   Its name states the property (e.g.
   `set_post_tags_locks_before_snapshotting`).

3. **AC3 — it proves mutual exclusion (precondition).** While the test holds the
   lock, the spawned `set_post_tags` is asserted not to have completed,
   expressed as a `tokio::time::timeout` of **300ms** that is expected to
   elapse, with a failure message naming the invariant. A comment marks this as
   the precondition check, not the regression guard.

4. **AC4 — it proves the snapshot is taken under the lock (the regression
   guard).** After the held transaction commits and the spawned call returns
   `Ok`, the post's tags are asserted **equal to exactly the desired set** — the
   rival writer's tag is gone. A read-then-lock implementation leaves the
   rival's tag behind and fails here.

5. **AC5 — the test is demonstrated to fail on the real regression.** During the
   cycle, `set_post_tags` is temporarily reshaped **read-then-lock** (snapshot
   read moved onto the pool _before_ `BEGIN IMMEDIATE` / `FOR UPDATE`) on each
   backend in turn, and the test observed to fail. The revert is not committed.
   The PR description **quotes the observed failure output for each backend** —
   an assertion that it was done is not sufficient.

6. **AC6 — no production behavior change.** The only non-comment changes are in
   `storage/src/test_support.rs` (the new affordance) and
   `storage/src/posts.rs`'s `#[cfg(test)]` module (the new test), plus planning
   docs. The non-test bodies of `storage/src/posts.rs`,
   `storage/src/sqlite/posts.rs`, and `storage/src/postgres/posts.rs` are
   unchanged except for comments.

7. **AC7 — the gate is green.** `devtool run -- cargo xtask validate` passes,
   including the coverage policy. (Both backend cases run within it; there is no
   separate per-backend coverage gate.)

## Out of scope

- Any change to `set_post_tags` itself. If the AC5 demonstration uncovers a real
  hole in either implementation, it is filed as a separate issue rather than
  fixed here.
- #363's structural "mutating methods take a transaction" work, which will later
  reshape these signatures.
- Concurrency tests for other storage methods.
