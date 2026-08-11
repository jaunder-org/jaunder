# Issue #876 + #883 — post-tag write spelled once: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-11-issue-876-tag-reconcile.md` — the
_what_ and _why_, including the six-entry correction log and why the CTE was
rejected.

**Goal:** Attach a tag to a post in two statements spelled once, instead of
three statements spelled four times.

**Architecture:** Two shared `pub(crate)` consts in `storage/src/posts.rs`, used
by both dialect reconciles and both arms of `PostWriteLock::add_tag`.
Transaction structure untouched.

**Tech Stack:** Rust, sqlx 0.8.6 (SQLite + Postgres), rstest / rstest_reuse,
cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:**

- `storage/src/posts.rs` — two consts; delete `SELECT_TAG_ID_BY_SLUG` and
  `TaggingError::MissingRow`; stable slug ordering in `post_tag_diff`; doc
  fixes.
- `storage/src/{sqlite,postgres}/posts.rs` — the loop bodies and the
  `RequireRow` imports.
- `storage/src/test_support.rs` — both `add_tag` arms and its doc block.
- `storage/src/postgres/mod.rs` — one stale `reason:` comment.

**Scope — out:** the reconcile bodies, the removal loop, transaction
_enforcement_ (Task 1 files it).

**Separable concerns:** transaction enforcement — filed by Task 1.

**Tasks:**

1. File the transaction-enforcement follow-up issue.
2. Apply `to_add` in stable slug order — **before** the lock is introduced.
3. Two shared consts; convert all four call sites.
4. Remove the dead pieces; fix the stale docs.
5. Pin the conflict path; run the full gate.

**Key risks / decisions:**

- **Task 2 precedes Task 3 deliberately.** Task 3's `DO UPDATE` takes a `tags`
  row lock that `DO NOTHING` did not, opening a Postgres deadlock window; Task 2
  is the mitigation. Ordered this way, **no commit in the history carries the
  regression unmitigated**. The sort is behaviour-neutral before Task 3, so it
  costs nothing.
- The sort must be **stable**, or `[Nix, nix]` loses first-casing-wins.
- `RequireRow`/`MissingRow` themselves **stay** — `subscriptions.rs:263` still
  uses them. Only `TaggingError`'s variant and the two `posts.rs` imports go.
- Both consts are **shared**: `$n` and `ON CONFLICT … DO NOTHING` both work on
  SQLite.

## Global Constraints

- Dual-backend tests use `#[apply(backends)]`; a bare `#[tokio::test]` that
  should be parameterised fails the `test-backend-pattern` guard.
- Shared SQL lives in `storage/src/posts.rs`, not the ADR-0019 dialect files.
- No `Co-Authored-By` trailer.
- Run `cargo xtask check` before committing; the pre-commit hook runs it too.

---

### Task 1: File the transaction-enforcement follow-up

Filed first per `jaunder-plan`'s scope check — captured up front so it can be
picked up concurrently rather than deferred to ship. It produces **no in-tree
artifact** (issues live in GitHub only, `docs/agents/issue-tracker.md`), so if
GitHub is unavailable, proceed to Task 2 and file it before ship rather than
blocking.

- [ ] **Step 1: File it via `jaunder-issues`**

Type `Task`, label `type-safety`. Title along the lines of "storage: make the
post-tag write's transaction unforgeable rather than conventional".

Body must carry, from the spec's "Transaction enforcement" section:

- The gap: both reconciles run in a transaction, but the two SQL consts could be
  issued on a bare pool; atomicity is a convention.
- It is pre-existing, not introduced by #876 — all call sites are inside
  transactional bodies today.
- Why it is not small: `PostDialect` needs an associated `Write<'p>` GAT (SQLite
  holds a raw `PoolConnection` with a manual `BEGIN IMMEDIATE`, Postgres a
  `Transaction`; one generic struct cannot name both, and an enum over concrete
  backends cannot be produced from `Pool<DB>` in generic code), and
  commit/rollback/conn-access all diverge too.
- **It requires #874 first**: a shared `?`-propagating body drops the guard on
  error, and the SQLite arm has no `Drop` rollback, so an error mid-reconcile
  returns a connection to the pool with `BEGIN IMMEDIATE` still open.

Cross-reference #874 (prerequisite), #363 (extends the property to the server-fn
boundary), #876 (this cycle).

- [ ] **Step 2: Wire the dependency and project**

```bash
gh api repos/jaunder-org/jaunder/issues/874 --jq .id
```

then, with that node id:

```bash
gh api --method POST repos/jaunder-org/jaunder/issues/<new>/dependencies/blocked_by -F issue_id=<874-node-id>
gh project item-add 1 --owner jaunder-org --url <new-issue-url>
```

Read the issue back (`issue_read`) — GitHub strips angle-bracket markup, and
this body names generic types.

- [ ] **Step 3: No commit** — nothing changed in the tree.

---

### Task 2: Apply `to_add` in stable slug order

Lands **before** the lock exists, so the history never contains the unmitigated
regression.

**Files:** Modify `storage/src/posts.rs` — `post_tag_diff` at `:347-367`; its
unit test `post_tag_diff_adds_removes_keeps` at `:3098-3113`.

---

- [ ] **Step 1: Sort in `post_tag_diff`**

The binding is currently immutable and untyped
(`let to_add = desired.iter()…collect()`, its type inferred from the struct
field), so it needs `let mut` and an explicit type:

```rust
    let mut to_add: Vec<&'a TagLabel> = desired
        .iter()
        .filter(|label| !existing_slugs.contains(&label.slug()))
        .collect();
    // Slug order, so every transaction takes `tags` row locks in the same order. The
    // upsert this feeds (`UPSERT_TAG_RETURNING_ID`) holds a row lock on the tag until
    // commit, so two concurrent reconciles adding overlapping tags in caller-supplied
    // order could otherwise deadlock on Postgres (#876). SQLite is unaffected —
    // `BEGIN IMMEDIATE` is database-wide — but the ordering is free and applies to both.
    //
    // `sort_by_key`, not `sort_unstable_by_key`: `desired` may carry two labels sharing
    // a slug and the FIRST occurrence's casing must still win, which
    // `set_post_tags_is_idempotent_and_absorbs_duplicate_slugs` asserts.
    to_add.sort_by_key(|label| label.slug());
```

`Tag: Ord` comes from the shared derive trailer (`macros/src/lib.rs:552-563`),
and `slug()` returns an **owned** `Tag`, so `sort_by_key` compiles without a
borrow problem.

Sorting here rather than in each dialect gives both backends the property from
one place, and the pure function already has a unit test.

- [ ] **Step 2: Rebuild the diff unit test's fixture**

`post_tag_diff_adds_removes_keeps` (`:3098-3113`) currently yields a
**single-element** `to_add`, so it cannot observe ordering — this is a fixture
rewrite, not an added assertion. Give it deliberately unordered multi-slug input
plus a duplicate-slug pair, and assert both properties: `to_add` comes back
slug-ordered, and the two labels sharing a slug keep their **input** order
(first-casing-wins).

- [ ] **Step 3: Run and gate**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage post_tag_diff set_post_tags
```

Expected: **PASS**.

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-876-tag-upsert-sql -- cargo xtask check
```

Expected: exit 0.

- [ ] **Step 4: Commit**

```bash
git add storage/src/posts.rs
git commit -m "refactor(storage): apply tag additions in slug order (#876)"
```

---

### Task 3: Two shared consts, four call sites

The atomic core: deleting `SELECT_TAG_ID_BY_SLUG` breaks the build until all
four call sites move, so Steps 1-4 land together.

**Files:**

- Modify: `storage/src/posts.rs` — replace `:293-296` only (`:298-304` is
  `DELETE_POST_TAG_BY_SLUG`, out of scope).
- Modify: `storage/src/sqlite/posts.rs` — import `:5`, loop body `:183-207`.
- Modify: `storage/src/postgres/posts.rs` — import `:5`, loop body `:170-195`.
- Modify: `storage/src/test_support.rs` — SQLite arm `:240-256`, Postgres arm
  `:259-275`.

**Interfaces:**

- Produces: `UPSERT_TAG_RETURNING_ID`, `INSERT_POST_TAG` (`pub(crate)`).
- Removes: `SELECT_TAG_ID_BY_SLUG`.

---

- [ ] **Step 1: Add the two consts, replacing `SELECT_TAG_ID_BY_SLUG`**

In `storage/src/posts.rs`, replace `:293-296` (the const and its doc) with:

```rust
/// Get-or-create a tag by slug, returning its id in **one** statement.
///
/// The no-op `DO UPDATE` is load-bearing: `DO NOTHING` emits no row for `RETURNING`
/// on the conflict path, which is exactly why a second `SELECT` used to follow this
/// (#883). Rewriting `tag_slug` to the value it already holds makes the id come back
/// on both the insert and the conflict path, so there is no window in which a
/// concurrently deleted tag yields `RowNotFound`. #343 landed the same shape for
/// `subscriptions`; both dialects run it.
///
/// Shared rather than per-dialect: SQLite accepts `$n` placeholders and
/// `ON CONFLICT … DO UPDATE … RETURNING`, so the old `INSERT OR IGNORE` was a spelling
/// difference, not a capability one.
///
/// **Takes a row lock on the tag until commit** (unlike the `DO NOTHING` it replaces),
/// so callers apply `to_add` in slug order — see `post_tag_diff`.
///
/// Bind order: `tag_slug`.
pub(crate) const UPSERT_TAG_RETURNING_ID: &str = "INSERT INTO tags (tag_slug) VALUES ($1)
     ON CONFLICT (tag_slug) DO UPDATE SET tag_slug = excluded.tag_slug
     RETURNING tag_id";

/// Attaches a tag to a post, tolerating the row already being there.
///
/// `DO NOTHING`, not `DO UPDATE`: `desired` may carry two labels sharing a slug
/// (`post_tag_diff` does not dedupe) and the first occurrence's casing must win, so the
/// existing row is left exactly as it is. Nothing reads a value back, so there is no
/// reason to force a row out of the conflict path here.
///
/// Bind order: `post_id, tag_id, tag_display`.
pub(crate) const INSERT_POST_TAG: &str = "INSERT INTO post_tags
     (post_id, tag_id, tag_display) VALUES ($1, $2, $3)
     ON CONFLICT (post_id, tag_id) DO NOTHING";
```

- [ ] **Step 2: Convert the SQLite loop body** (`sqlite/posts.rs:183-207`)

```rust
            for label in diff.to_add {
                let slug = label.slug();
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
            }
```

`fetch_one`, not `fetch_optional` + `require_row`: `DO UPDATE` guarantees a row
on both paths. Update the `use crate::posts::{…}` list (add both consts, drop
`SELECT_TAG_ID_BY_SLUG`) and delete `use crate::error::RequireRow;` at `:5` — it
has no other use in this file.

- [ ] **Step 3: Convert the Postgres loop body** (`postgres/posts.rs:170-195`)

Identical shape with `&mut *tx`. Same import edits, and delete
`use crate::error::RequireRow;` at `:5`.

- [ ] **Step 4: Convert both `add_tag` arms**

`storage/src/test_support.rs:240-256` (SQLite) and `:259-275` (Postgres) — note
both ranges run to the end of the `post_tags` insert; editing only the first
half leaves a stray statement. Both arms become the same two statements,
differing only in the executor (`&mut **conn` vs `&mut **tx`). Drop the
`SELECT_TAG_ID_BY_SLUG` import; add the two consts.

- [ ] **Step 5: Confirm the sequence is spelled once**

```bash
rg -n 'SELECT_TAG_ID_BY_SLUG' storage/
```

Expected: no output — this is AC1's own check.

```bash
rg -n 'INSERT OR IGNORE' storage/src/
```

Expected: no output.

```bash
rg -n 'INSERT INTO post_tags' storage/src/
```

Expected: exactly one hit, the const. (**Not** `rg 'INSERT INTO tags'` — that
also matches the seed loop at `storage/src/postgres/mod.rs:344`, an unrelated
test, so it would fail on a correct implementation.)

- [ ] **Step 6: Run the tag suites**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags
```

Expected: **PASS**, both backends, all nine tests named in spec AC6 — including
`set_post_tags_locks_before_snapshotting` (`:2859`), which is what actually pins
AC3 (the transactions still serialize) now that code inside both transactions
has changed.

- [ ] **Step 7: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-876-tag-upsert-sql -- cargo xtask check
```

```bash
git add storage/src/posts.rs storage/src/sqlite/posts.rs storage/src/postgres/posts.rs storage/src/test_support.rs
git commit -m "refactor(storage): resolve a tag id in one statement, spelled once (#876, #883)"
```

---

### Task 4: Remove the dead pieces, fix the stale docs

Separately committable — Task 3's build is green without this.

**Files:** `storage/src/posts.rs` (`TaggingError` at `:369-390`, docs at
`:866-873`, `:912-915`), `storage/src/test_support.rs` (`:226-234`),
`storage/src/postgres/mod.rs` (`:329-334`).

---

- [ ] **Step 1: Remove `TaggingError::MissingRow`**

Delete the variant and its `#[from]` (`:375-386`). Its doc describes the
read-back that no longer exists, and nothing constructs one after Task 3.

Verified safe: no match arm on it anywhere — `server/src/atompub/mod.rs` uses
only a blanket `From<TaggingError>` and `TaggingError::PostNotFound`.

**Do not** remove `crate::error::MissingRow` or `RequireRow` themselves —
`storage/src/subscriptions.rs:263` still uses them and `lib.rs:55` re-exports
them.

- [ ] **Step 2: Fix the stale docs**

- `storage/src/posts.rs:866-873` and `:912-915` — say the upsert diverges
  `INSERT OR IGNORE` vs `ON CONFLICT DO NOTHING`. It no longer diverges at all;
  what still diverges in `set_post_tags` is the **transaction shape**
  (`BEGIN IMMEDIATE` vs `FOR UPDATE`), which is why the method stays on
  `PostDialect`.
- `storage/src/test_support.rs:226-234` — `add_tag`'s doc says "**Three
  statements**, not one" and that the conflict spelling "diverges per dialect".
  Both false now: two statements, one spelling. What remains true is that the FK
  forces the tag row to exist before the join row, which is why it is still two.
- `storage/src/postgres/mod.rs:329-334` — the `reason:` comment on #891's array
  test says the bind is "which #876's single-statement tag reconcile depends
  on". That dependency was never real and that design was abandoned; re-word to
  the capability the test actually pins (a `StrNewtype` slice binds as `TEXT[]`
  without a strip).

- [ ] **Step 3: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-876-tag-upsert-sql -- cargo xtask check
```

```bash
git add storage/src/posts.rs storage/src/test_support.rs storage/src/postgres/mod.rs
git commit -m "docs(storage): drop the read-back error and its stale narration (#883)"
```

---

### Task 5: Pin the conflict path, then the full gate

**Files:** `storage/src/posts.rs` — `mod tests`.

---

- [ ] **Step 1: Write the test**

Nothing currently names the conflict path. `…absorbs_duplicate_slugs` exercises
it incidentally (a `DO NOTHING` regression surfaces there as an opaque
`RowNotFound`), which is not a test whose failure explains itself.

```rust
    /// #883: the upsert returns the tag id on its **conflict** path, not just when it
    /// inserts. A `DO UPDATE` → `DO NOTHING` regression makes `RETURNING` emit no row,
    /// so `fetch_one` fails — and this is the test that says why.
    ///
    /// Cross-post deliberately: the second post's tag already exists in `tags`, so the
    /// upsert can only take the conflict path.
    #[apply(backends)]
    #[tokio::test]
    async fn set_post_tags_reuses_an_existing_tag_across_posts(#[case] backend: Backend) {
        let env = backend.setup().await;
        let user = SeedUser::new().seed(&env.state).await.user_id;
        let first = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let second = SeedRawPost::new(user).seed(&env.state).await.post_id;
        let posts = &*env.state.posts;

        posts
            .set_post_tags(first, &[parse_tag_label("rust")])
            .await
            .expect("first post takes the insert path");
        posts
            .set_post_tags(second, &[parse_tag_label("rust")])
            .await
            .expect("second post takes the conflict path");

        assert_eq!(slugs_of(posts, first).await, vec!["rust"]);
        assert_eq!(slugs_of(posts, second).await, vec!["rust"]);
    }
```

- [ ] **Step 2: Run it, and confirm it is not vacuous**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage set_post_tags_reuses_an_existing_tag
```

Expected: **PASS**, both backends.

Then temporarily change `UPSERT_TAG_RETURNING_ID`'s
`DO UPDATE SET tag_slug = excluded.tag_slug` to `DO NOTHING` and re-run:
expected **FAIL** on the second `set_post_tags` with a `RowNotFound`-shaped
error. Revert.

- [ ] **Step 3: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-876-tag-upsert-sql -- cargo xtask check
```

```bash
git add storage/src/posts.rs
git commit -m "test(storage): pin that the tag upsert returns an id on its conflict path (#883)"
```

- [ ] **Step 4: Full local gate (AC10)**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-876-tag-upsert-sql -- cargo xtask validate --no-e2e
```

Expected: exit 0. `--no-e2e` is right: the diff is `storage/`-only, no web or
server-fn surface. Use Bash background mode. **If killed at the harness's
10-minute cap, retry once before concluding anything** — this session
established that a killed run leaves contending processes and the retry
succeeds. Do not invent an explanation for a failure that has not been
diagnosed.

- [ ] **Step 5: Note for ship (AC9)**

The PR body must say **`Closes #876`** and **`Closes #883`**. #883's acceptance
("resolves a tag id in one statement per dialect, backend parity preserved,
suites green") is met by Task 3. Recorded here because no earlier step owns it
and #883 would otherwise stay open.

Also record in the PR: `DO UPDATE` is not only a deadlock risk (mitigated by
Task 2) but a **contention** change — every `set_post_tags` now holds an
exclusive lock on each tag row it adds until commit, so concurrent tagging with
a hot tag serializes, and each reuse writes a dead tuple into `tags`. Accepted,
matching #343's precedent, but stated.

---

## Self-review

**Spec coverage:**

| Spec AC | Task                                                                  |
| ------- | --------------------------------------------------------------------- |
| AC1     | T3 S1 (const replaced), T3 S5 (`rg SELECT_TAG_ID_BY_SLUG`)            |
| AC2     | T3 S1-S4 (four call sites), T3 S5 (greps, correctly scoped)           |
| AC3     | **T3 S6** — `set_post_tags_locks_before_snapshotting` is what pins it |
| AC4     | T2 (stable sort, both reasons in the comment, fixture rebuilt)        |
| AC5     | T5 S1-S2 (named test plus falsification)                              |
| AC6     | T3 S6 (all nine run)                                                  |
| AC7     | T4 S1 (`MissingRow`; `RequireRow` itself kept)                        |
| AC8     | T3 S2-S4 and T4 S2 (four stale doc sites)                             |
| AC9     | **T5 S5** (`Closes #876` / `Closes #883` in the PR body)              |
| AC10    | T5 S4 (`validate --no-e2e`)                                           |

**Placeholders:** none — every step carries real Rust or a real command. Task
1's issue body is specified by content, which is the `jaunder-issues` seam.

**Type consistency:** `UPSERT_TAG_RETURNING_ID` and `INSERT_POST_TAG` are
spelled identically in the definitions, all four call sites, and the greps.
`query_scalar::<_, TagId>` with `.bind(&slug)` matches the shipped idiom at
`sqlite/posts.rs:192-193` and needs no `sqlx-newtype-decode` allowlist entry.
