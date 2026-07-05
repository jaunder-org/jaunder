# Plan — issue #9: single-transaction batch post insert

Spec:
[`docs/superpowers/specs/2026-07-05-issue-9-batch-post-insert.md`](../specs/2026-07-05-issue-9-batch-post-insert.md)
(read it for the what/why; this plan is the how).

## Review header

**Goal.** Let both post seeders write N posts in **one** transaction instead of
N, without duplicating any row-write logic — by extracting the single-row write
and input-building so a single and a batch path share them.

**Scope.**

- _In:_ extract `write_post_in_tx::<DB>` + delegate `create_post`; add
  `PostStorage::create_posts` (trait + one generic impl); extract
  `render_post_input` / `seed_post_input` builders + delegate the single-post
  paths; rewire `seed_posts` and `seed_posts_for_user` to batch; tests.
- _Out:_ a true multi-row `VALUES` statement (rejected — see spec); any schema
  change; any ADR.

**Tasks (one line each).**

- [x] 1. Extract `write_post_in_tx::<DB>` from `create_post`; `create_post`
     delegates. Behavior identical. _(commit d443f702; added a dual-backend
     FK-violation test covering the surfaced `Internal` arm rather than
     `cov:ignore`-ing it.)_
- [x] 2. Add `PostStorage::create_posts` (trait decl + generic `PostStore<DB>`
     impl) + `#[apply(backends)]` integration tests (empty / happy /
     mid-batch-conflict-rolls-back). _(commit c33ac035)_
- [x] 3. Extract `render_post_input` + `seed_post_input`; `create_rendered_post`
     and `seed_rendered_post` delegate. Behavior identical. _(commit bae3ba2b)_
- [x] 4. Rewire `seed_posts` and `seed_posts_for_user` to build
     `Vec<CreatePostInput>` and call `create_posts` once; **delete the
     now-orphaned `seed_rendered_post`** + repoint its stale doc/manifest refs.
     _(commit 39f5e999; net −16 lines)_
- [x] 5. Full gate green — `cargo xtask validate` (incl. all four e2e combos)
     passed; code review APPROVE (nits only).

**Key risks / decisions.**

- **No new row-write copy.** Tasks 1 & 3 are behavior-preserving extractions
  committed on their own, so the batch additions (2 & 4) are provably thin. This
  is the entire point of the issue — do not add a second insert body.
- **Atomicity.** `create_posts` is all-or-nothing: one `tx`, `?` on any row
  drops `tx` → sqlx auto-rollback, matching `create_post`'s current
  rollback-on-error. Test asserts nothing persists on mid-batch conflict.
- **Zero backend-file churn.** `create_posts` and `write_post_in_tx` live only
  on the generic `PostStore<DB>`; `storage/src/sqlite/posts.rs` and
  `storage/src/postgres/posts.rs` are untouched, no new `PostDialect` const.
- **Coverage.** `create_posts` (incl. its empty-slice branch) is covered by task
  2's dual-backend test. `seed_posts_for_user` lives in the `test-support` crate
  (out-of-process, e2e-only) — task 4 must confirm batching it introduces no new
  host-coverage-gate failure and matches whatever exemption it has today.

**For agentic workers:** execute via **`jaunder-iterate`**, delegating a task to
a subagent via **`jaunder-dispatch`** where useful. Tick checkboxes in real
time.

## Global constraints

- Commit type `refactor` for tasks 1 & 3 (no behavior change), `feat`/`test` for
  task 2, `refactor`/`test` for task 4. `Refs: #9` trailer. **No
  `Co-Authored-By` trailer** (global preference).
- Every task ends green on `cargo xtask check` (the pre-commit hook runs it) —
  run it before committing (**`jaunder-commit`**). Storage tests need a
  reachable PostgreSQL: run via `devtool pg run -- …` (or
  `devtool run -- cargo xtask check` for the gate, which supplies its own
  ephemeral PG).
- All paths below are worktree-relative to
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-9-batch-post-insert/`.
- No `.unwrap()`/`.expect()` in `storage/`/`test-support/` production code
  (permitted in `#[cfg(test)]` and integration tests).

---

## Task 1 — extract `write_post_in_tx::<DB>`, `create_post` delegates

**File:** `storage/src/posts.rs`

**Interface (new free fn, `pub(crate)`, mirrors `replace_post_audiences`'s
receiver + bound shape at `posts.rs:1569`):**

```rust
/// Writes one post row and its audience rows onto a caller-supplied transaction
/// connection, so it joins whatever transaction is open. The single place that
/// knows the post INSERT + unique-violation→SlugConflict mapping; `create_post`
/// (one) and `create_posts` (many) are pure transaction orchestration over it.
pub(crate) async fn write_post_in_tx<DB>(
    conn: &mut DB::Connection,
    input: &CreatePostInput,
) -> Result<i64, CreatePostError>
where
    DB: PostDialect,
    for<'q> i64: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<i64>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> &'q str: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<String>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> DateTime<Utc>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    for<'q> Option<DateTime<Utc>>: sqlx::Encode<'q, DB> + sqlx::Type<DB>,
    (i64,): for<'r> sqlx::FromRow<'r, DB::Row>,
    for<'c> &'c mut DB::Connection: sqlx::Executor<'c, Database = DB>,
    for<'q> DB::Arguments<'q>: sqlx::IntoArguments<'q, DB>,
{
    let now = Utc::now();
    let format = input.format.to_string();
    let post_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO posts (user_id, title, slug, body, format, rendered_html, created_at, updated_at, published_at, summary)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING post_id",
    )
    .bind(input.user_id)
    .bind(input.title.clone())
    .bind(input.slug.as_str())
    .bind(input.body.as_str())
    .bind(format.as_str())
    .bind(input.rendered_html.as_str())
    .bind(now)
    .bind(now)
    .bind(input.published_at)
    .bind(input.summary.clone())
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => CreatePostError::SlugConflict,
        e => CreatePostError::Internal(e),
    })?;

    replace_post_audiences::<DB>(conn, post_id, &input.audiences).await?;
    Ok(post_id)
}
```

**`create_post` becomes** (replaces the body at `posts.rs:597-637`; keep the
`#[tracing::instrument]` attribute):

```rust
async fn create_post(&self, input: &CreatePostInput) -> Result<i64, CreatePostError> {
    let mut tx = self.pool.begin().await?;
    // On any error the `?` drops `tx`, which sqlx rolls back — equivalent to the
    // previous explicit tx.rollback() before returning.
    let post_id = write_post_in_tx::<DB>(&mut tx, input).await?;
    tx.commit().await?;
    Ok(post_id)
}
```

Note `&mut tx` coerces to `&mut DB::Connection` for the helper, exactly as the
old `replace_post_audiences::<DB>(&mut tx, …)` call did.

**Test:** none new — this is behavior-preserving. The existing
`create_rendered_post_*` and `create_post` coverage
(`server/tests/storage/storage.rs`, incl.
`create_rendered_post_slug_conflict_returns_storage_error:6373`) must still
pass.

**Run (expect PASS, unchanged):**

```
devtool pg run -- cargo nextest run -p jaunder --test storage
```

**Commit:** `refactor(storage): extract write_post_in_tx from create_post` — run
`devtool run -- cargo xtask check` first (must be clean).

---

## Task 2 — add `PostStorage::create_posts` + generic impl + tests

**File:** `storage/src/posts.rs`

**Trait declaration** (add after `create_post` at `posts.rs:308`):

```rust
    /// Creates `inputs.len()` posts in a single transaction, returning their new
    /// ids in input order. All-or-nothing: any failure (e.g. a slug conflict)
    /// rolls the whole batch back and nothing persists. An empty slice is a
    /// no-op that returns an empty vec without opening a transaction.
    async fn create_posts(&self, inputs: &[CreatePostInput]) -> Result<Vec<i64>, CreatePostError>;
```

**Generic impl** (add in the `impl<DB> PostStorage for PostStore<DB>` block,
after `create_post`; the block's `where` clause already carries every bound
`write_post_in_tx` needs):

```rust
    #[tracing::instrument(
        name = "storage.posts.create_batch",
        skip(self, inputs),
        fields(db.system = DB::DB_SYSTEM, count = inputs.len())
    )]
    async fn create_posts(&self, inputs: &[CreatePostInput]) -> Result<Vec<i64>, CreatePostError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await?;
        let mut ids = Vec::with_capacity(inputs.len());
        for input in inputs {
            // `?` drops `tx` on error → whole-batch rollback (atomic seed).
            ids.push(write_post_in_tx::<DB>(&mut tx, input).await?);
        }
        tx.commit().await?;
        Ok(ids)
    }
```

**Tests** — append to `server/tests/storage/storage.rs` (dual-backend via
`#[apply(backends)]`, matching the file's convention). Build inputs directly as
`CreatePostInput` (no render needed for these) so the test exercises the trait
method in isolation.

```rust
// =============================================================================
// create_posts (batch insert) integration tests — issue #9
// =============================================================================

#[apply(backends)]
#[tokio::test]
async fn create_posts_empty_slice_is_noop(#[case] backend: Backend) {
    let env = backend.setup().await;
    let ids = env.state.posts.create_posts(&[]).await.unwrap();
    assert!(ids.is_empty());
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_batches_all_rows_in_order(#[case] backend: Backend) {
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("batch_alice"), &password("password123"), None, false)
        .await
        .unwrap();

    let inputs: Vec<CreatePostInput> = (0..3)
        .map(|i| CreatePostInput {
            user_id,
            title: Some(format!("Batch {i}")),
            slug: format!("batch-{i}").parse().unwrap(),
            body: format!("body {i}"),
            format: PostFormat::Markdown,
            rendered_html: format!("<p>body {i}</p>"),
            published_at: Some(Utc::now()),
            summary: None,
            audiences: vec![AudienceTarget::Public],
        })
        .collect();

    let ids = state.posts.create_posts(&inputs).await.unwrap();
    assert_eq!(ids.len(), 3);

    // Each id resolves to the matching row, with its Public audience honored
    // (visible to Anonymous).
    for (i, id) in ids.iter().enumerate() {
        let rec = state
            .posts
            .get_post_by_id(*id, &ViewerIdentity::Anonymous)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rec.title.as_deref(), Some(format!("Batch {i}").as_str()));
    }
}

#[apply(backends)]
#[tokio::test]
async fn create_posts_conflict_rolls_back_whole_batch(#[case] backend: Backend) {
    use storage::CreatePostError;
    let env = backend.setup().await;
    let state = &env.state;
    let user_id = state
        .users
        .create_user(&username("batch_bob"), &password("password123"), None, false)
        .await
        .unwrap();

    let mk = |slug: &str, i: usize| CreatePostInput {
        user_id,
        title: Some(format!("Row {i}")),
        slug: slug.parse().unwrap(),
        body: format!("body {i}"),
        format: PostFormat::Markdown,
        rendered_html: format!("<p>body {i}</p>"),
        published_at: Some(Utc::now()),
        summary: None,
        audiences: vec![AudienceTarget::Public],
    };
    // Rows 0 and 2 collide on slug — the batch must fail on row 2 and undo row 0/1.
    let inputs = vec![mk("dup", 0), mk("unique", 1), mk("dup", 2)];

    let err = state.posts.create_posts(&inputs).await.unwrap_err();
    assert!(matches!(err, CreatePostError::SlugConflict));

    // Nothing persisted: the author's collection is empty.
    let collection = state
        .posts
        .list_collection_by_user(user_id, None, 50)
        .await
        .unwrap();
    assert!(collection.is_empty(), "expected full rollback, found {} rows", collection.len());
}
```

> Confirm during implementation that `CreatePostInput`, `PostFormat`,
> `AudienceTarget`, `ViewerIdentity`, `username`, `password`, `Utc`, `Backend`,
> `backends` are already imported at the top of `storage.rs` (the existing
> `create_rendered_post_*` tests use all of them); add any missing `use` only if
> the compiler asks.

**Run (expect the three new tests PASS on both `case_1_sqlite` and
`case_2_postgres`):**

```
devtool pg run -- cargo nextest run -p jaunder --test storage create_posts
```

**Commit:** `feat(storage): add PostStorage::create_posts batch insert` (tests
included in the same commit — behavior change requires tests).

---

## Task 3 — extract `render_post_input` + `seed_post_input`, delegate singles

**File:** `storage/src/post_service.rs`

**`render_post_input`** — pull the render-and-build half out of
`create_rendered_post` (`post_service.rs:28-52`):

```rust
/// Renders `body` per `format` and assembles the `CreatePostInput`, without
/// writing. Shared by `create_rendered_post` (write one) and the batch seeders
/// (collect many), so the render+assemble recipe lives in one place.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn render_post_input(
    user_id: i64,
    title: Option<String>,
    slug: Slug,
    body: String,
    format: PostFormat,
    published_at: Option<DateTime<Utc>>,
    summary: Option<String>,
    audiences: Vec<AudienceTarget>,
) -> CreatePostInput {
    let rendered_html = render(&body, &format);
    CreatePostInput { user_id, title, slug, body, format, rendered_html, published_at, summary, audiences }
}
```

`create_rendered_post` delegates (signature unchanged):

```rust
pub async fn create_rendered_post(
    storage: &dyn PostStorage,
    /* …same args… */
) -> Result<i64, CreatePostError> {
    let input = render_post_input(user_id, title, slug, body, format, published_at, summary, audiences);
    storage.create_post(&input).await
}
```

**`seed_post_input`** — recipe-as-data alongside `seed_rendered_post`
(`post_service.rs:67-86`), same `#[cfg(any(test, feature = "seed-posts"))]`
gate. It **supersedes** `seed_rendered_post`, which Task 4 deletes once both
callers batch; this task keeps `seed_rendered_post` only as a temporary
green-compile bridge:

```rust
/// The single definition of a timeline-visible seeded post, as data: Public
/// audience + Markdown render, published-now iff `published`. Returns the input
/// instead of writing it, so both seeders can batch a `Vec` of these.
#[cfg(any(test, feature = "seed-posts"))]
#[must_use]
pub fn seed_post_input(user_id: i64, slug: Slug, body: String, published: bool) -> CreatePostInput {
    render_post_input(
        user_id,
        None,
        slug,
        body,
        PostFormat::Markdown,
        published.then(Utc::now),
        None,
        vec![AudienceTarget::Public],
    )
}
```

`seed_rendered_post` delegates (temporary — **deleted in Task 4**; both its
callers move to `create_posts` there, and a kept zero-caller `pub` fn would fail
the stateless coverage gate):

```rust
pub async fn seed_rendered_post(
    storage: &dyn PostStorage,
    user_id: i64,
    slug: Slug,
    body: String,
    published: bool,
) -> Result<i64, CreatePostError> {
    storage.create_post(&seed_post_input(user_id, slug, body, published)).await
}
```

**Test:** none new — behavior-preserving. Existing `create_rendered_post_*`
tests and any `seed_rendered_post`/`seed_posts` users must still pass.

**Run (expect PASS, unchanged):**

```
devtool pg run -- cargo nextest run -p jaunder --test storage
```

**Commit:**
`refactor(storage): extract render_post_input/seed_post_input builders`.

---

## Task 4 — rewire both seeders to batch

**File A:** `storage/src/test_support.rs` — `seed_posts` (`:574-594`):

```rust
pub async fn seed_posts(state: &Arc<AppState>, user_id: i64, count: usize, published: bool) -> Vec<i64> {
    let inputs: Vec<_> = (0..count)
        .map(|i| {
            crate::seed_post_input(
                user_id,
                format!("seed-{i}").parse().expect("valid slug"),
                format!("# Post {i}\n\nbody"),
                published,
            )
        })
        .collect();
    state
        .posts
        .create_posts(&inputs)
        .await
        .expect("seed posts should be created")
}
```

**File B:** `test-support/src/lib.rs` — `seed_posts_for_user` (`:57-90`). Build
the `Vec<CreatePostInput>` (handling the slug-parse error while building), then
one `create_posts`:

```rust
use storage::{seed_post_input, AppState};   // was `{seed_rendered_post, AppState}` at lib.rs:16
                                            // (do NOT add CreatePostInput — the Vec element type is
                                            // inferred from create_posts's &[CreatePostInput]; an
                                            // unused import trips clippy -D warnings)

pub async fn seed_posts_for_user(
    state: &Arc<AppState>,
    username: &str,
    count: usize,
    published: bool,
    prefix: &str,
) -> anyhow::Result<Vec<i64>> {
    let uname = username.parse::<Username>().map_err(|_| anyhow::anyhow!("invalid username: {username}"))?;
    let user = state
        .users
        .get_user_by_username(&uname)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no such user: {username}"))?;

    let mut inputs = Vec::with_capacity(count);
    for i in 0..count {
        let slug = seed_slug(prefix, i)
            .parse()
            .map_err(|_| anyhow::anyhow!("generated slug invalid for prefix {prefix:?} index {i}"))?;
        inputs.push(seed_post_input(user.user_id, slug, seed_body(prefix, i), published));
    }

    state
        .posts
        .create_posts(&inputs)
        .await
        .map_err(|e| anyhow::anyhow!("batch seed of {count} posts failed: {e:?}"))
}
```

**File C:** `storage/src/post_service.rs` — **delete `seed_rendered_post`**
(`:67-86`). After File A + File B, it has zero callers (verify:
`git grep -n seed_rendered_post` should return only its definition and the stale
doc references below). Leaving it as a compiled, gated, zero-caller `pub` fn
would make its body uncovered and **fail the stateless coverage gate** that Task
4 runs. Its recipe now lives in `seed_post_input`.

**File D (stale references — non-compiling, fix in the same commit):** once
`seed_rendered_post` is gone, repoint the docs/manifests that name it:

- `test-support/src/lib.rs` module doc (`:5-11`) — "drives the shared
  `storage::seed_rendered_post` recipe" → `seed_post_input`.
- `storage/Cargo.toml:41` — the `seed-posts` feature comment naming
  `seed_rendered_post` → `seed_post_input`.
- `test-support/Cargo.toml` — any comment naming `seed_rendered_post`.
- `git grep -n seed_rendered_post` must return **nothing** after this task.

**Coverage check (blocking):** `seed_posts_for_user` lives in the `test-support`
crate, exercised only out-of-process by e2e, and is **not** measured by the host
coverage set (it carries no `cov:ignore` today yet main is green — proof it
isn't measured), so batching it is coverage-safe. The real coverage risk was the
orphaned `seed_rendered_post` (File C), now deleted. Confirm the gate is green:

```
devtool run -- cargo xtask validate --no-e2e
```

If it unexpectedly flags `test-support/src/lib.rs`, match the file's existing
treatment (`git grep -n 'cov:ignore' test-support/src/lib.rs`) rather than
inventing a new exemption.

**Run (expect PASS — the `web_posts` pagination tests that call `seed_posts`
still green, now via one transaction):**

```
devtool pg run -- cargo nextest run -p jaunder
```

**Commit:** `refactor(test-support): seed posts via one batched transaction`
(rewires both seeders, deletes the superseded `seed_rendered_post`, repoints its
stale doc/manifest references; `Refs: #9`).

---

## Task 5 — full gate

Run the pre-push gate; it must be green before ship:

```
devtool run -- cargo xtask validate --no-e2e
```

(e2e is unaffected — no web/UI surface changed — but `jaunder-ship` will run the
full `validate` including e2e as the final gate.)

Then hand off to `jaunder-ship`.

## Self-review

- [ ] No second post-INSERT body exists anywhere (grep `INSERT INTO posts` → one
      occurrence, in `write_post_in_tx`).
- [ ] `storage/src/sqlite/posts.rs` and `storage/src/postgres/posts.rs`
      unchanged; no new `PostDialect` const.
- [ ] Tasks 1 & 3 introduce no behavior change (no new tests, existing green).
- [ ] `create_posts` covered on both backends incl. empty-slice branch.
- [ ] Both seeders call `create_posts` exactly once; ids remain in order.
- [ ] `git grep -n seed_rendered_post` returns **nothing** (deleted, no stale
      doc/manifest reference left) — no orphaned zero-caller fn for the gate.
- [ ] `test-support/src/lib.rs` import is `{seed_post_input, AppState}` (no
      unused `CreatePostInput`).
- [ ] No `Co-Authored-By` trailer on any commit.
