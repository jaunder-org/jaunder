# Issue #339 — Atomic Tag Reconciliation Regression Test: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-09-issue-339-atomic-tag-reconcile.md`
— read it for the _what_ and _why_; this plan is the _how_ and does not restate
it.

**Goal:** Guard the invariant #771 established — that `set_post_tags` takes its
write lock before snapshotting the tags it diffs against — with a dual-backend
concurrency regression test.

**Architecture:** A new `storage::test_support` affordance holds the same write
lock `set_post_tags` takes across await points and can write through it, so a
test can act as a rival writer and force the interleave deterministically. The
test itself lives in `storage/src/posts.rs`'s `#[cfg(test)]` module under
`#[apply(backends)]`.

**Tech Stack:** Rust, sqlx (SQLite + Postgres), tokio, rstest / rstest_reuse,
cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:**

- `storage/src/test_support.rs`: `CloseablePool::lock_post_for_write` and a
  `PostWriteLock` guard with `add_tag` / `commit`.
- `storage/src/posts.rs` `#[cfg(test)] mod tests`: one `#[apply(backends)]`
  test.
- A recorded demonstration that the test fails on the real regression.

**Scope — out:** any change to `set_post_tags` itself; #363's structural
transaction work; concurrency tests for other storage methods.

**Separable concerns:** none found. The spec review verified both backend
implementations against the invariant and found no production hole, so there is
no issue-filing first task. If Task 2's demonstration uncovers one, file it via
`jaunder-issues` and do not fix it here.

**Tasks:**

1. Add the lock-holding test-support affordance and the regression test; commit.
2. Demonstrate the test fails on a read-then-lock regression on both backends;
   record the output in this plan; run the full local gate. No commit of the
   regression.

**Key risks / decisions:**

- The 300ms probe must stay well inside SQLite's 5s `busy_timeout`, or a
  _correct_ implementation returns `SQLITE_BUSY` instead of blocking.
- The test holds one pooled connection while the spawned call takes another from
  the same pool. sqlx's default `max_connections` is 10; at 1 this deadlocks.
- The still-pending probe is a precondition check, not the regression guard. The
  final exact-tag-set assertion is what catches read-then-lock. Both carry
  comments saying so.

## Global Constraints

- Dual-backend storage tests use the `#[apply(backends)]` template; a bare
  `#[tokio::test]` that should be dual-backend fails the `test-backend-pattern`
  guard (`CONTRIBUTING.md`, backend parity).
- Dialect-specific SQL belongs behind a backend match in shared code or in the
  ADR-0019 per-dialect files — never inline in a test body.
- ADR-0053 §1: an `#[apply(backends)]` test proving a `PostStore<DB>` contract
  lives in the generic home module, `storage/src/posts.rs`.
- Coverage policy applies to the new `test_support` code (`CONTRIBUTING.md`).
- No `Co-Authored-By` trailer on commits.
- Run the gate before committing; the pre-commit hook runs the full
  `cargo xtask check`.

---

### Task 1: Lock-holding affordance + the regression test

**Files:**

- Modify: `storage/src/test_support.rs` — add `lock_post_for_write` to
  `impl CloseablePool` (after `execute`, before `postgres()`, ~line 100–133),
  and a new `PostWriteLock` type after the `CloseablePool` impl block.
- Modify: `storage/src/posts.rs` — add the test to `#[cfg(test)] mod tests`
  (module starts line 2601), beside the other `set_post_tags` tests (~line
  2731–2830).
- Test: `storage/src/posts.rs` (in-file `#[cfg(test)]`, per ADR-0053 §1).

**Interfaces:**

- Consumes: `CloseablePool` (`storage/src/test_support.rs:65`); `PostId`,
  `TagId`, `TagLabel`, and `SELECT_TAG_ID_BY_SLUG` from `crate::posts`; the
  `#[apply(backends)]` / `Backend::setup()` harness; `slugs_of`
  (`storage/src/posts.rs:2719`) and `parse_tag_label` (`common::test_support`).
- Produces:
  - `CloseablePool::lock_post_for_write(&self, post_id: PostId) -> Result<PostWriteLock<'_>, sqlx::Error>`
  - `enum PostWriteLock<'a> { Sqlite(PoolConnection<Sqlite>), Postgres(Transaction<'a, Postgres>) }`
  - `PostWriteLock::add_tag(&mut self, post_id: PostId, label: &TagLabel) -> Result<(), sqlx::Error>`
  - `PostWriteLock::commit(self) -> Result<(), sqlx::Error>`

---

- [ ] **Step 1: Write the failing test**

Add to `storage/src/posts.rs`'s `mod tests`, after
`set_post_tags_rejects_missing_post_but_allows_soft_deleted`:

```rust
    /// #339: `set_post_tags` must take its write lock **before** snapshotting the
    /// tags it diffs against, and hold it through the writes — so two writers on
    /// one post serialize and the committed result is exactly the desired set.
    ///
    /// The interleave is forced, not raced: the test holds the same lock
    /// `set_post_tags` takes and acts as a rival writer. A hopeful two-task race
    /// would pass or fail on scheduling and prove nothing.
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_locks_before_snapshotting(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let post = SeedRawPost::new(user).seed(&env.state).await.post_id;

        env.state
            .posts
            .set_post_tags(post, &[parse_tag_label("alpha")])
            .await
            .expect("seed tags");

        // The rival writer: holds the post write lock and adds "beta", uncommitted.
        let mut rival = env
            .base
            .pool()
            .lock_post_for_write(post)
            .await
            .expect("take post write lock");
        rival
            .add_tag(post, &parse_tag_label("beta"))
            .await
            .expect("rival adds a tag");

        // Two pooled connections are live at once — this one and the spawned
        // call's — so the pool must allow >= 2. sqlx's default max_connections is
        // 10 and neither backend overrides it; at 1 this would deadlock, not fail.
        let posts = Arc::clone(&env.state.posts);
        let mut racer =
            tokio::spawn(async move { posts.set_post_tags(post, &[parse_tag_label("gamma")]).await });

        // PRECONDITION, not the regression guard: this proves mutual exclusion
        // exists at all. A read-then-lock implementation still blocks here on its
        // writes, so this assertion alone does not catch it — the final one does.
        //
        // 300ms sits well inside SQLite's 5s busy_timeout
        // (storage/src/sqlite/mod.rs), so a correct implementation is still
        // retrying — not failing with SQLITE_BUSY — when the lock is released below.
        assert!(
            tokio::time::timeout(Duration::from_millis(300), &mut racer)
                .await
                .is_err(),
            "set_post_tags completed while another writer held the post write lock; \
             its read-diff-write is not serialized (#339)"
        );

        rival.commit().await.expect("rival commits");
        racer
            .await
            .expect("racer task panicked")
            .expect("set_post_tags failed");

        // THE REGRESSION GUARD. A correct implementation snapshots after the lock
        // is granted, so it sees {alpha, beta}, puts both in `to_remove`, and
        // leaves exactly {gamma}. A read-then-lock implementation snapshots
        // {alpha} before the rival commits, never removes "beta", and leaves
        // {beta, gamma}.
        assert_eq!(slugs_of(&*env.state.posts, post).await, vec!["gamma"]);
    }
```

Add these imports to `mod tests` — verified absent from both `use super::*`'s
reach (`storage/src/posts.rs:3-27`) and the tests module's own `use` list
(2602–2615), so Step 1 does not compile without them:

```rust
    use std::sync::Arc;
    use std::time::Duration;
```

- [ ] **Step 2: Run the test, verify it fails**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_locks_before_snapshotting
```

Expected: **FAIL** — compile error,
`no method named lock_post_for_write found for reference &CloseablePool`.
(`devtool pg run` starts the ephemeral cluster the postgres case needs and tears
it down after; a bare `cargo nextest run` fails the postgres case with a
connection error.)

- [ ] **Step 3: Implement the affordance**

Add to `storage/src/test_support.rs`. The body cannot be pinned by tests — it
_is_ the harness — so it is written out in full.

Imports — each one verified against the tree, because two of the obvious guesses
are wrong:

- `PostId` (line 22) and `TagLabel` (line 30) are **already imported**.
  Re-adding them to the `use crate::posts::{…}` list is an `E0252` duplicate
  import.
- `SELECT_TAG_ID_BY_SLUG` — add to the existing `use crate::posts::{…}`. It is
  `pub(crate)` (`storage/src/posts.rs:298`) and `test_support` is in the same
  crate, so it is reachable.
- `TagId` — import from **`common::ids`**, not `crate::posts`. `posts.rs:9`
  brings it in via a _private_ `use`, which is not a re-export, so
  `crate::posts::TagId` is `E0603`. `sqlite/posts.rs:11` and
  `postgres/posts.rs:11` both take it from `common::ids`; match them.
- `sqlx::{Postgres, Sqlite, Transaction, pool::PoolConnection}` — add as needed.

Inside `impl CloseablePool`, after `execute`:

```rust
    /// Takes the same write lock `set_post_tags` takes and holds it until the
    /// returned guard commits or drops — which `execute` cannot do, since it
    /// returns its connection to the pool as soon as the statement finishes.
    ///
    /// The lock's granularity differs per backend, deliberately:
    ///
    /// * `SQLite` — `BEGIN IMMEDIATE` takes a **database-wide** write lock, so the
    ///   guard excludes any concurrent writer, not just one on `post_id`.
    /// * Postgres — `SELECT … FOR UPDATE` locks the **post row**, so exclusion is
    ///   per-post.
    ///
    /// Both serialize two writers on the same post, which is the invariant tests
    /// built on this assert. `post_id` is taken on both arms even though only the
    /// Postgres arm needs it, so callers stay backend-agnostic and the intent is
    /// visible at the call site.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the connection cannot be acquired or the lock
    /// cannot be taken (including when `post_id` does not exist on Postgres).
    pub async fn lock_post_for_write(
        &self,
        post_id: PostId,
    ) -> Result<PostWriteLock<'_>, sqlx::Error> {
        match self {
            CloseablePool::Sqlite(pool) => {
                let mut conn = pool.acquire().await?;
                // IMMEDIATE, mirroring `SqlitePostStorage::set_post_tags`: takes
                // the write lock up front rather than upgrading a shared lock,
                // which `busy_timeout` cannot rescue (ADR-0021).
                sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
                Ok(PostWriteLock::Sqlite(conn))
            }
            CloseablePool::Postgres(pool) => {
                let mut tx = pool.begin().await?;
                // Mirrors `PostgresPostStorage::set_post_tags`.
                sqlx::query_scalar::<_, PostId>(
                    "SELECT post_id FROM posts WHERE post_id = $1 FOR UPDATE",
                )
                .bind(post_id)
                .fetch_one(&mut *tx)
                .await?;
                Ok(PostWriteLock::Postgres(tx))
            }
        }
    }
```

After the `impl CloseablePool` block:

```rust
/// A held post write lock, from [`CloseablePool::lock_post_for_write`].
///
/// **The two arms do not behave the same on drop.** The Postgres arm is a real
/// `Transaction`, which rolls back when dropped. The `SQLite` arm's
/// `BEGIN IMMEDIATE` was issued as a raw statement, so sqlx's transaction-depth
/// tracking never saw it: dropping the guard returns the connection to the pool
/// **with the write transaction still open**, holding a database-wide write lock.
/// A test that panics between `lock_post_for_write` and [`commit`] therefore
/// wedges the rest of that test's writes rather than failing cleanly. Commit (or
/// end the test) promptly.
pub enum PostWriteLock<'a> {
    Sqlite(PoolConnection<Sqlite>),
    Postgres(Transaction<'a, Postgres>),
}

impl PostWriteLock<'_> {
    /// Adds one tag to the post from inside the held lock — a rival writer, for
    /// tests that must interleave a competing write with a storage method.
    ///
    /// Three statements, not one: `post_tags` carries a foreign key to
    /// `tags(tag_id)`, so the tag row must exist and its id be read back before
    /// the join row can be inserted. The conflict-tolerant spelling also diverges
    /// per dialect (`INSERT OR IGNORE` vs `ON CONFLICT DO NOTHING`). Dispatching
    /// it here is what keeps dialect SQL out of test bodies.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if any of the three statements fails.
    pub async fn add_tag(
        &mut self,
        post_id: PostId,
        label: &TagLabel,
    ) -> Result<(), sqlx::Error> {
        let slug = label.slug();
        match self {
            PostWriteLock::Sqlite(conn) => {
                sqlx::query("INSERT OR IGNORE INTO tags (tag_slug) VALUES ($1)")
                    .bind(&slug)
                    .execute(&mut **conn)
                    .await?;
                let tag_id = sqlx::query_scalar::<_, TagId>(SELECT_TAG_ID_BY_SLUG)
                    .bind(&slug)
                    .fetch_one(&mut **conn)
                    .await?;
                sqlx::query(
                    "INSERT OR IGNORE INTO post_tags (post_id, tag_id, tag_display) \
                     VALUES ($1, $2, $3)",
                )
                .bind(post_id)
                .bind(tag_id)
                .bind(label)
                .execute(&mut **conn)
                .await?;
            }
            PostWriteLock::Postgres(tx) => {
                sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1) ON CONFLICT DO NOTHING")
                    .bind(&slug)
                    .execute(&mut **tx)
                    .await?;
                let tag_id = sqlx::query_scalar::<_, TagId>(SELECT_TAG_ID_BY_SLUG)
                    .bind(&slug)
                    .fetch_one(&mut **tx)
                    .await?;
                sqlx::query(
                    "INSERT INTO post_tags (post_id, tag_id, tag_display) VALUES ($1, $2, $3) \
                     ON CONFLICT DO NOTHING",
                )
                .bind(post_id)
                .bind(tag_id)
                .bind(label)
                .execute(&mut **tx)
                .await?;
            }
        }
        Ok(())
    }

    /// Commits the held transaction, releasing the lock and persisting whatever
    /// was written through it.
    ///
    /// # Errors
    ///
    /// Returns the `sqlx::Error` if the commit fails.
    pub async fn commit(self) -> Result<(), sqlx::Error> {
        match self {
            PostWriteLock::Sqlite(mut conn) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
            }
            PostWriteLock::Postgres(tx) => tx.commit().await?,
        }
        Ok(())
    }
}
```

The borrow spellings are correct as written: `PoolConnection<Sqlite>` derefs to
`SqliteConnection` and `Transaction<'_, Postgres>` to `PgConnection`, and
`&mut Connection` is the `Executor`.

- [ ] **Step 4: Run the test, verify it passes**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_locks_before_snapshotting
```

Expected: **PASS**, two cases — `::sqlite` and `::postgres`.

- [ ] **Step 5: Run the gate**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-339-atomic-tag-reconcile -- cargo xtask check
```

Expected: exit 0. Read `.xtask/last-result.json` `.steps` if it fails. Follow
`jaunder-commit` — do not commit on a partial check.

- [ ] **Step 6: Commit**

```bash
git add storage/src/test_support.rs storage/src/posts.rs
git commit -m "test(storage): pin that set_post_tags locks before snapshotting (#339)"
```

---

### Task 2: Demonstrate the test catches the real regression (AC5)

The test is only worth its line count if it fails when the invariant breaks.
This task proves that, per spec AC5, on **both** backends. Nothing here is
committed except the recorded evidence in this plan.

**Files:**

- Temporarily modify (then revert): `storage/src/sqlite/posts.rs:159-176`,
  `storage/src/postgres/posts.rs:143-163`.
- Modify: this plan — append the observed output under "## AC5 evidence".

**Interfaces:** none produced.

- [ ] **Step 1: Reshape SQLite to read-then-lock**

In `SqlitePostStorage::set_post_tags`, move the `SELECT_POST_TAGS` read (and its
`post_tags_from_rows` / `post_tag_diff`) to run against `pool` **before**
`sqlx::query("BEGIN IMMEDIATE")`, leaving the writes inside the transaction.
This is the pre-#771 shape.

Note the read must move _before_ the lock, not merely onto the pool. A pooled
read placed _after_ `BEGIN IMMEDIATE` still runs only once the lock is granted,
sees the rival's committed writes, and the test correctly passes.

- [ ] **Step 2: Observe the sqlite case fail**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_locks_before_snapshotting
```

Expected: **FAIL**, `::sqlite` case, at the final assertion — left
`["beta", "gamma"]`, right `["gamma"]`. Copy the assertion output verbatim.

Use `devtool pg run` here, not a bare `cargo nextest run`: without the cluster
the postgres case also fails, with an unrelated connection error, and the
recorded evidence then can't distinguish the demonstrated regression from a
missing database.

- [ ] **Step 3: Revert SQLite, reshape Postgres the same way**

```bash
git checkout -- storage/src/sqlite/posts.rs
```

Then in `PostgresPostStorage::set_post_tags`, move the `SELECT_POST_TAGS` read
to run against `pool` **before** `pool.begin()`.

- [ ] **Step 4: Observe the postgres case fail**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_locks_before_snapshotting
```

Expected: **FAIL**, `::postgres` case, same assertion, same left/right. Copy it
verbatim.

- [ ] **Step 5: Revert Postgres and confirm green**

```bash
git checkout -- storage/src/postgres/posts.rs
git status --porcelain
```

Expected: only this plan file modified. Then:

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_locks_before_snapshotting
```

Expected: **PASS**, both cases.

- [ ] **Step 6: Record the evidence and commit**

Append to this plan:

```markdown
## AC5 evidence

Both failures observed with the snapshot read moved before the lock acquisition
(the pre-#771 read-then-lock shape); the reshape was reverted, not committed.

### sqlite

<paste the verbatim assertion output>

### postgres

<paste the verbatim assertion output>
```

`jaunder-ship` quotes this section in the PR description, which is what AC5
requires.

```bash
git add docs/superpowers/plans/2026-08-09-issue-339-atomic-tag-reconcile.md
git commit -m "docs(plan): record the #339 regression demonstration (#339)"
```

- [ ] **Step 7: Run the full local gate (AC7)**

`cargo xtask check` in Task 1 is the iterate-time gate; AC7 names `validate`.
Run it here so the criterion has an owner rather than being deferred to ship:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-339-atomic-tag-reconcile -- cargo xtask validate --no-e2e
```

Expected: exit 0, including the coverage policy. This change touches no web or
browser surface, so `--no-e2e` is the right scope here; `jaunder-ship` runs the
full `validate` before the PR.

Coverage note: there is no `test_support` exclusion in `xtask/src/coverage/`, so
the new affordance is measured. Both `lock_post_for_write` arms and both
`add_tag` arms are exercised — one per backend case — so line coverage should
close. If the gate still flags the `Err` paths (which no test drives), add the
uncovered-error justification the coverage policy prescribes rather than
inventing a fault-injection test the spec did not authorize.

---

## Self-review

**Spec coverage:**

| Spec AC | Task                                                                                                       |
| ------- | ---------------------------------------------------------------------------------------------------------- |
| AC1     | Task 1 Step 3 (affordance, both arms, granularity doc)                                                     |
| AC2     | Task 1 Step 1 (`#[apply(backends)]` in `storage/src/posts.rs`)                                             |
| AC3     | Task 1 Step 1 (300ms `tokio::time::timeout`, precondition comment)                                         |
| AC4     | Task 1 Step 1 (final `assert_eq!` on the exact tag set)                                                    |
| AC5     | Task 2 (both backends, evidence recorded verbatim)                                                         |
| AC6     | Task 1 Step 6 / Task 2 Step 5 (`git status` confirms the revert)                                           |
| AC7     | Task 2 Step 7 (`cargo xtask validate --no-e2e`, incl. coverage); Task 1 Step 5 is the iterate-time `check` |

**Placeholders:** none — every step carries real Rust or a real command. The two
`<paste …>` markers in Task 2 Step 6 are outputs to be captured during
execution, not undefined behavior.

**Type consistency:** `lock_post_for_write` / `PostWriteLock` / `add_tag` /
`commit` are spelled identically in the Interfaces block, the affordance code,
and the test body. `slugs_of` takes `&dyn PostStorage`, and the test passes
`&*env.state.posts`, matching `storage/src/posts.rs:2719`.
