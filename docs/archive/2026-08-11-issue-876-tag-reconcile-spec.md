# Issue #876 + #883 — the post-tag write, spelled once, without a read-back

## Status

Spec — awaiting approval.

Supersedes the parked `2026-08-10-issue-876-tag-upsert-sql.md` draft; its
correction log is carried into this document's own.

## Background

Attaching one tag to a post is three statements — insert-or-ignore into `tags`,
`SELECT` the id back, insert the `post_tags` row — and the sequence is spelled
in four places:

- `storage/src/sqlite/posts.rs:183-207`
- `storage/src/postgres/posts.rs:170-195`
- `storage/src/test_support.rs:240-251` and `:259-270`
  (`PostWriteLock::add_tag`, one arm per backend)

Two open issues describe two halves:

- **#876** — the duplication: "a schema change needs three coordinated edits".
- **#883** — the read-back is a TOCTOU of the shape #343 fixed for `subscribe`.
  The comment #343 left in both dialect files says it exactly: the missing-row
  arm "is unreachable only because nothing deletes a tag today — a fact about
  the data, not the statement."

One change closes both.

## The precedent

#343 landed this shape for `subscriptions`
(`storage/src/{sqlite,postgres}/subscriptions.rs:9-14`), and its trait doc gives
the reasoning:

> … on the conflict, rewrites `subscriber_ref` to the value it already holds.
> That deliberate no-op write is what makes `RETURNING` emit the row on the
> conflict path too — so the statement returns the `subscription_id` on both
> paths, and no second `SELECT` (and no TOCTOU window) is needed.

It is genuinely exercised on **both** backends: `SubscriptionStore::subscribe`
(`storage/src/subscriptions.rs:173-180`) drives it with `fetch_one`, and
`server/tests/web/web_subscriptions.rs` runs
`subscribe_then_unsubscribe_round_trips` under `#[apply(backends)]`.

#883's pre-checks hold: `tags.tag_slug` is `TEXT NOT NULL UNIQUE` and
`post_tags` has `UNIQUE (post_id, tag_id)` on both backends
(`storage/migrations/{sqlite,postgres}/0009_create_tags.sql:3`, `:10`), and
there are **no triggers and no `updated_at`** anywhere in `storage/migrations`.

## Design

### Two shared constants replace the whole sequence

```rust
/// Get-or-create a tag by slug, returning its id in **one** statement.
///
/// The no-op `DO UPDATE` is load-bearing: `DO NOTHING` emits no row for `RETURNING`
/// on the conflict path, which is exactly why a second `SELECT` existed here (#883).
/// Rewriting `tag_slug` to the value it already holds makes the id come back on both
/// the insert and the conflict path. #343 landed the same shape for `subscriptions`.
///
/// Bind order: `tag_slug`.
pub(crate) const UPSERT_TAG_RETURNING_ID: &str = "INSERT INTO tags (tag_slug) VALUES ($1)
     ON CONFLICT (tag_slug) DO UPDATE SET tag_slug = excluded.tag_slug
     RETURNING tag_id";

/// Attaches a tag to a post, tolerating the row already being there.
///
/// `DO NOTHING`, not `DO UPDATE`: `desired` may carry two labels sharing a slug
/// (`post_tag_diff` does not dedupe) and the first occurrence's casing must win, so
/// the existing row is left exactly as it is. Nothing reads a value back, so there is
/// no reason to force a row out of the conflict path here.
///
/// Bind order: `post_id, tag_id, tag_display`.
pub(crate) const INSERT_POST_TAG: &str = "INSERT INTO post_tags
     (post_id, tag_id, tag_display) VALUES ($1, $2, $3)
     ON CONFLICT (post_id, tag_id) DO NOTHING";
```

**Both are shared, not per-dialect.** `$n` placeholders work on both backends
here — the existing shared `SELECT_POST_TAGS` and `DELETE_POST_TAG_BY_SLUG`
already rely on that, and `sqlite/posts.rs` already binds `$1..$3`. And SQLite
accepts `ON CONFLICT (…) DO NOTHING`, which is why `INSERT OR IGNORE` can go: it
was a spelling difference, not a capability one. (Unlike `SubscriptionDialect`,
whose consts are per-dialect only because they use `?`.)

That is the whole of #876: after this the tag-attach sequence exists **once**,
and the `post_tags` column list — the copy that made a schema change a
three-file edit — exists once too.

### The loop bodies

Each of the four sites collapses from three statements to two:

```rust
let tag_id = sqlx::query_scalar::<_, TagId>(UPSERT_TAG_RETURNING_ID)
    .bind(&slug)
    .fetch_one(&mut *conn)
    .await?;
sqlx::query(INSERT_POST_TAG)
    .bind(post_id)
    .bind(tag_id)
    .bind(label)
    .execute(&mut *conn)
    .await?;
```

`fetch_one`, not `fetch_optional` + `require_row`: `DO UPDATE` guarantees a row
on both paths, so the absence the `require_row` guard described can no longer
occur.

Everything else is untouched. **SQLite** keeps `BEGIN IMMEDIATE` + read + diff +
loop; **Postgres** keeps `pool.begin()` + `SELECT … FOR UPDATE` + read + diff +
loop (ADR-0021). The removal loop and `DELETE_POST_TAG_BY_SLUG` are untouched.

### The one behavioural regression: a Postgres deadlock window

`DO UPDATE` takes a **row lock** on the `tags` row, held to commit. `DO NOTHING`
took none. So two concurrent `set_post_tags` on **different** posts, adding
overlapping tags in different orders (`[x, y]` versus `[y, x]`), can now
deadlock (`40P01`) where today they never contend.

`post_tag_diff`'s `to_add` preserves the caller's `desired` order
(`storage/src/posts.rs:358-361`), which is user-supplied, so nothing imposes a
consistent order today.

**Mitigation: apply `to_add` in slug order**, so every transaction takes `tags`
row locks in the same order. The sort must be **stable** — `to_add` can hold two
entries with the same slug (`[Nix, nix]`) and the first occurrence's casing must
still win, which `set_post_tags_is_idempotent_and_absorbs_duplicate_slugs`
asserts.

SQLite is unaffected either way: `BEGIN IMMEDIATE` is a database-wide write
lock, so two writers never overlap.

### Consequences to clean up

- **`SELECT_TAG_ID_BY_SLUG` is deleted** — no callers remain.
- **`TaggingError::MissingRow` (`storage/src/posts.rs:375-386`) becomes
  unconstructible.** It was the read-back's named absence and its doc describes
  exactly that. Nothing else in the tagging path produces one, so the variant
  and its `#[from]` go with the statement they existed for.
- **`use crate::error::RequireRow;`** becomes unused in both `sqlite/posts.rs:5`
  and `postgres/posts.rs:5` — a hard failure under the repo's deny-warnings
  build.

### Not a hazard, but worth stating

`storage/src/sqlite/sessions.rs:19-20` avoids `RETURNING` because it "with a
correlated subquery causes `SQLITE_BUSY` under concurrency". That caveat is
scoped to a correlated subquery; `UPSERT_TAG_RETURNING_ID` has none, and
`INSERT_SUBSCRIPTION` already runs `RETURNING` on SQLite alongside a `VALUES`
subquery. Recorded because it is the one SQLite-specific `RETURNING` hazard this
repo has already documented, and a reviewer will otherwise raise it.

### Why not the single-statement CTE

An earlier draft had Postgres perform the whole reconcile in one data-modifying
CTE, on the reasoning that a single statement is atomic and so needs no
transaction. **That reasoning was wrong.** Atomicity is not serializability:
under READ COMMITTED the whole statement runs on one snapshot taken at statement
start, so the `DELETE` arm cannot see rows a concurrent transaction commits
after it. Two reconciles on the same post, A→`{x}` and B→`{y}`, would both
snapshot `{alpha}`, both delete it, and both insert — final state `{x, y}`,
neither writer's set. That is the lost update #339 exists to prevent, and
`set_post_tags_locks_before_snapshotting` fails on its regression assertion.
With `desired = []` the statement inserts nothing, so no FK check fires and it
does not block a `FOR UPDATE` holder at all.

There is no in-statement repair — a `FOR UPDATE` inside the CTE does not refresh
the statement snapshot for other tables. Serialization needs the lock in a
separate, earlier statement, which is what the code already does. Hence this
spec leaves the transaction structure alone.

## Acceptance criteria

1. **AC1 — no read-back.** `SELECT_TAG_ID_BY_SLUG` is deleted;
   `rg 'SELECT_TAG_ID_BY_SLUG' storage/` returns nothing. No code resolves a tag
   id in a statement separate from the one that ensures the tag exists.

2. **AC2 — the sequence is spelled once.** `UPSERT_TAG_RETURNING_ID` and
   `INSERT_POST_TAG` are defined once in `storage/src/posts.rs` and used at all
   four sites (both dialect reconciles, both arms of `add_tag`). Mechanically:
   `rg 'INSERT OR IGNORE' storage/src/` returns nothing, and
   `rg 'INSERT INTO post_tags' storage/src/` returns exactly one hit. (**Not**
   `rg 'INSERT INTO tags'` — that also matches the seed loop in
   `storage/src/postgres/mod.rs:344`, an unrelated test, so it would fail on a
   correct implementation.)

3. **AC3 — the transaction structure is untouched.** SQLite still drives
   `BEGIN IMMEDIATE` with explicit `COMMIT`/`ROLLBACK`; Postgres still opens a
   transaction and takes `SELECT … FOR UPDATE`. Stated as a criterion because a
   previous draft removed them and was wrong to.

4. **AC4 — `to_add` is applied in stable slug order**, with a comment at the
   sort site giving both reasons: consistent `tags` lock-acquisition order
   across concurrent reconciles, and first-casing-wins preservation. A bare
   `sort` reads as cosmetic and invites removal.

5. **AC5 — the conflict path is pinned by name.** A dual-backend test attaches
   an **existing** tag to a **different** post, so `UPSERT_TAG_RETURNING_ID`
   takes its conflict path and must still return the id. A
   `DO UPDATE`→`DO NOTHING` regression would otherwise surface only
   incidentally, as a `RowNotFound` from an unrelated test.

6. **AC6 — every pinned behaviour survives, on both backends.** These
   dual-backend tests pass unchanged: `set_post_tags_adds_removes_and_clears`
   (`posts.rs:2745`), `…_preserves_existing_display_casing` (`:2772`),
   `…_is_idempotent_and_absorbs_duplicate_slugs` (`:2800`),
   `…_rejects_missing_post_but_allows_soft_deleted` (`:2829`),
   `…_locks_before_snapshotting` (`:2859`),
   `…_with_unchanged_set_writes_nothing` (`:2929`),
   `…_insert_error_returns_internal` (`:3772`), `…_round_trips_slug_and_label`
   (`:4010`), and `post_round_trips_slug_title_body_username_and_tag` (`:4040`).

7. **AC7 — the dead pieces go.** `TaggingError::MissingRow` and its `#[from]`
   are removed, and the `RequireRow` imports in both dialect files with them.
   Nothing in `storage/` still refers to a tag-row read-back.

8. **AC8 — stale comments corrected, and only the ones that are stale.**
   - The `#883` read-back comments in both dialect files — removed with their
     code.
   - `PostWriteLock::add_tag`'s doc (`test_support.rs:226-234`) — says "Three
     statements" and that the conflict spelling "diverges per dialect". **Both**
     become false: two statements, one spelling.
   - The dialect-divergence notes at `storage/src/posts.rs:293-296` and
     `:866-873`, and the `set_post_tags` trait doc at `:912-915` — these say the
     upsert is `INSERT OR IGNORE` vs `ON CONFLICT DO NOTHING`, which stops being
     true.
   - `storage/src/postgres/mod.rs:329-334` — the `reason:` comment on #891's
     array test claims the bind is "which #876's single-statement tag reconcile
     depends on". That dependency was never real (correction log 4) and the
     design it names is abandoned; re-word to the capability the test actually
     pins.

9. **AC9 — #883 closed.** The PR references both #876 and #883; #883's
   acceptance ("resolves a tag id in one statement per dialect, backend parity
   preserved, suites green") is met.

10. **AC10 — the gate is green.** `cargo xtask validate --no-e2e` passes,
    including coverage. `sqlx-newtype-decode` needs no new entry — it approves
    any type whose declaration carries a bridge macro
    (`sqlx_newtype_decode_check.rs:27-30`), and `TagId` does. The diff is
    `storage/`-only; CI runs the full matrix.

## Out of scope

- The per-dialect reconcile bodies, the removal loop, `DELETE_POST_TAG_BY_SLUG`.

### Transaction _enforcement_ — deferred deliberately, to its own cycle

Both reconciles run inside a transaction and AC3 pins that. What this spec does
**not** do is make that unforgeable: the two constants are SQL strings, and
nothing stops a future caller issuing `UPSERT_TAG_RETURNING_ID` on a bare pool.
The atomicity of the tag write remains a convention.

That gap is **pre-existing, not introduced** — all four call sites are inside
existing transactional bodies, so this change adds no unenforced surface. But it
is a real gap, and it is the thing an earlier round of this design set out to
close with a `PostWriteTx` guard that `insert_post_tag` would take by reference.

Deferred because closing it properly is a much larger change than #876 asks for:

- `PostDialect` needs an associated `Write<'p>` type (a GAT), because the SQLite
  arm holds a raw `PoolConnection` with a manual `BEGIN IMMEDIATE` while
  Postgres holds a `Transaction` — one generic struct cannot name both, and an
  enum over concrete backends cannot be produced from `Pool<DB>` in generic
  code.
- Commit, rollback and connection-access all diverge too, so it is not "one
  `begin` method".
- It **requires #874 first**: a shared `?`-propagating body drops the guard on
  any error, and the SQLite arm has no `Drop` rollback, so an error
  mid-reconcile would return a connection to the pool with `BEGIN IMMEDIATE`
  still open — a database-wide write lock held by an idle pooled connection.

The plan's first task files this as an issue, cross-referencing #874 (the
prerequisite) and #363 (which extends the same property outward to the server-fn
boundary).

## Correction log

1. A draft claimed `INSERT OR IGNORE` swallows FK violations, making the change
   a safety fix. **False** — SQLite's conflict resolution never covered FOREIGN
   KEY, FKs are enforced here, and the schema has no CHECK constraints.
2. A draft proposed a shared helper taking `&mut DB::Connection` with "must be
   called inside a transaction" as a documented precondition. An unenforced
   precondition with an authoritative home is worse than the duplication it
   replaces.
3. A draft claimed `begin` would be the only dialect-divergent operation of a
   `PostWriteTx` guard. Commit, rollback and connection access diverge too.
4. **#891 was filed and landed as a prerequisite for #876** on the grounds that
   the fix needed typed array binds. True only of the CTE reconcile; **this fix
   needs no arrays**, and #883 was open the whole time naming the same two
   statements. #891 stands on its own merits but the dependency was asserted
   before the existing issue was found.
5. **A draft specified the whole reconcile as a single Postgres CTE**, arguing
   that single-statement atomicity removed the need for a transaction.
   **Atomicity is not serializability** — see "Why not the single-statement
   CTE".
6. **A draft re-scoped to the tag-id statement alone and called #876 closed.**
   It was not: the `post_tags` insert — the copy carrying the column list, and
   the reason a schema change is a three-file edit — was still spelled four
   times and still dialect-divergent. Unifying it is what actually closes #876.
