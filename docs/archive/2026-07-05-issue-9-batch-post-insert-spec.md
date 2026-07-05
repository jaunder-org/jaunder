# Spec — issue #9: single-transaction batch post insert

- **Issue:** [#9](https://github.com/jaunder-org/jaunder/issues/9) — "tests: add
  single-transaction batch insert to `PostStorage` so `seed_posts` seeds N posts
  in one transaction (not the existing per-call direct-seed loop)"
- **Type:** test-infra / perf (storage)
- **Date:** 2026-07-05
- **Worktree/branch:** `worktree-issue-9-batch-post-insert`

## Problem

`seed_posts` (`storage/src/test_support.rs:574`) seeds N posts by looping a
single-row write per iteration. The chain is single-row all the way down:

```
seed_posts (loop N)
  └ seed_rendered_post        recipe: fixes 5 args (Public audience, markdown, published…)
      └ create_rendered_post   render body → build CreatePostInput → write
          └ create_post        1 row, opens its OWN pool.begin()   ← per-call transaction
```

Each `create_post` (`storage/src/posts.rs:597`) opens its own `pool.begin()`, so
seeding 26–55 rows for the pagination tests pays N transaction round-trips. The
goal is **one** transaction for all N.

The helper only holds `&dyn PostStorage` (the pool is hidden), and ADR-0019
deliberately keeps `Transaction<'_, DB>` inside the concrete `PostStore<DB>` — a
tx cannot cross the `dyn` boundary. So the batch **must** be a new trait method;
the layers above it only need to _produce_ N `CreatePostInput`s rather than
write them one at a time.

## Constraint driving the design: no duplicated row-write

The naïve "add `create_posts` that mirrors `create_post`" duplicates the insert
SQL, the unique-violation→`SlugConflict` mapping, and the
`replace_post_audiences` call. That is explicitly rejected. Instead, the
single-row write is extracted so the single and batch methods share it and
differ only by "once vs. loop".

## Design

### Write side (storage) — extract, then batch

1. **Preparatory refactor (behavior-preserving):** extract the single-row write
   out of `create_post` into a shared helper on the same executor/transaction:

   ```rust
   // storage/src/posts.rs — modelled on the existing replace_post_audiences::<DB>,
   // runs on a caller-supplied transaction connection so it joins the open tx.
   async fn write_post_in_tx<DB>(
       conn: &mut DB::Connection,          // exact receiver type mirrors replace_post_audiences
       input: &CreatePostInput,
   ) -> Result<i64, CreatePostError>
   where DB: PostDialect /* + the PostStore where-bounds it needs */ {
       let now = Utc::now();
       let format = input.format.to_string();
       let post_id = sqlx::query_scalar::<_, i64>(
           "INSERT INTO posts (…) VALUES ($1..$10) RETURNING post_id")
           /* .bind ×10 */
           .fetch_one(&mut *conn).await
           .map_err(|e| match e {
               sqlx::Error::Database(e) if e.is_unique_violation() => CreatePostError::SlugConflict,
               e => CreatePostError::Internal(e),
           })?;
       replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;
       Ok(post_id)
   }
   ```

   `create_post` becomes pure orchestration; on `Err` the `?` drops `tx`, which
   sqlx auto-rolls-back (equivalent to the current explicit `tx.rollback()`):

   ```rust
   let mut tx = self.pool.begin().await?;
   let id = write_post_in_tx::<DB>(&mut tx, input).await?;   // drop-on-Err = rollback
   tx.commit().await?;
   Ok(id)
   ```

   This step is committed on its own with no behavior change — existing
   `create_post` tests stay green.

2. **Add the batch method** to the `PostStorage` trait and implement it **once**
   on the generic `PostStore<DB>` — one transaction, loop the shared helper,
   commit once:

   ```rust
   async fn create_posts(&self, inputs: &[CreatePostInput]) -> Result<Vec<i64>, CreatePostError> {
       if inputs.is_empty() { return Ok(Vec::new()); }        // no tx for empty input
       let mut tx = self.pool.begin().await?;
       let mut ids = Vec::with_capacity(inputs.len());
       for input in inputs {
           ids.push(write_post_in_tx::<DB>(&mut tx, input).await?);  // any Err → whole batch rolls back
       }
       tx.commit().await?;
       Ok(ids)
   }
   ```

   **Semantics:** all-or-nothing. A `SlugConflict` (or any error) on row _k_
   rolls back the entire transaction and returns the error — nothing persists.
   **Zero** lines added to `storage/src/sqlite/posts.rs` or
   `storage/src/postgres/posts.rs`; no new `PostDialect` const (the only backend
   divergence, `INSERT_POST_AUDIENCE` / `DELETE_POST_AUDIENCES`, is already
   parameterized and reused via `replace_post_audiences`).

### Build side (test seeding) — mirror the same extract

3. **Extract `render_post_input(...) -> CreatePostInput`** (free fn in
   `storage/src/post_service.rs`): the render-and-build half of
   `create_rendered_post`. `create_rendered_post` becomes
   `create_post(&render_post_input(...))` — behavior identical, but callers can
   now obtain an _input_ without writing it.

4. **Turn the recipe into a builder:**
   `seed_post_input(user_id, slug, body, published) -> CreatePostInput` — the
   single source of truth for the timeline-visible recipe (Public audience,
   markdown, published-now-iff-published), returning the input instead of
   persisting it. `seed_rendered_post` collapses to
   `create_post(&seed_post_input(...))`, staying the shared recipe both seeders
   use.

5. **Both seeders batch** — the in-process and out-of-process seeders currently
   share the same single-row loop, so both get the same build-then-batch shape:
   - **`seed_posts`** (`storage/src/test_support.rs:574`) builds
     `Vec<CreatePostInput>` from `seed_post_input` over `0..count`, one
     `create_posts(&inputs)`.
   - **`seed_posts_for_user`** (`test-support/src/lib.rs:57`, the out-of-process
     e2e fixture seeder) does the same, mapping its per-prefix `seed_slug`/
     `seed_body` (with the slug-parse error handled while building the Vec)
     through `seed_post_input`, then one `create_posts(&inputs)`. Same
     per-call-transaction cost removed for e2e fixture seeding too.

   ```
   {seed_posts, seed_posts_for_user} → (seed_post_input × N) → Vec<CreatePostInput>
                                       → create_posts(&inputs)   ← ONE tx
   ```

   Returned ids stay in creation order (the loop preserves order). Distinct
   per-row `Utc::now()` values (µs apart, monotonic in creation order) keep
   `created_at` ordering deterministic for the pagination tests. Atomic rollback
   is fine for both — seed slugs are unique by construction, so no conflict is
   expected; a failure now rolls back the whole seed instead of leaving a
   partial one.

## Scope

**In:** `write_post_in_tx` extract + `create_post` delegation; `create_posts`
trait method + generic impl; `render_post_input` / `seed_post_input` extracts;
**both** `seed_posts` and `seed_posts_for_user` rewired to batch; tests.

**Out (noted as candidate follow-up, not done here):**

- A true single-statement multi-row `VALUES` insert — rejected: `post_audiences`
  rows can't join the `posts` statement, so audiences loop regardless, and the
  transaction-per-call cost (the actual target) is already removed by step 2.

## Testing

- **Backend parity:** `create_posts` is exercised on both backends. `seed_posts`
  already runs under `#[apply(backends)]` via the pagination tests, which now
  drive `create_posts`. Add a direct integration test (`server/tests/storage/`)
  under `#[apply(backends)]` covering:
  - empty slice → `Ok(vec![])`, no rows written;
  - happy path → N ids, N rows + N audience rows, ids in order;
  - mid-batch slug conflict → `Err(SlugConflict)` and **nothing** persisted
    (atomic rollback).
- **Coverage:** the stateless gate requires every executable line covered. The
  extracts and `create_posts` are all reached by the tests above on both
  backends; no `cov:ignore` expected. CRAP stays well under T=30 (each method is
  short and straight-line). The plan must confirm the rewritten
  `seed_posts_for_user` (in the `test-support` crate) stays as covered/exempt as
  it is today — i.e. batching it introduces no new gate-visible uncovered line.
- **Gate:** `cargo xtask validate --no-e2e` must be green (static + clippy +
  coverage). No e2e surface changes.

## Backends / migrations / ADR

- **No migration** — no schema change.
- **No new ADR** — the change lives entirely within the existing ADR-0019
  generic-store pattern; it adds no new dialect divergence. (If review
  disagrees, a short ADR draft can be added, but none is anticipated.)
- **Backend parity rule satisfied:** the trait change is implemented once on the
  generic store (both backends), and the new test covers both.

## Open questions

None outstanding — design agreed in the pre-spec interview.
