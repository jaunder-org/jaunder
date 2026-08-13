# Direct Author-Unpublished Permalink Lookup Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

## Review

**Goal:** Replace the paginated author-draft permalink fallback with one indexed
query and make future-scheduled Posts open at their canonical permalink.

**Scope:** In: the `PostStorage` direct lookup, shared request timestamp, web
fallback cutover, dual-backend regressions, and the existing scheduled-Post
Playwright flow. Out: publication timing, permalink construction, slug
allocation, schema migrations, and non-author access.

**Tasks:**

1. Add and prove the direct author-unpublished storage query.
2. Cut the web path over and prove scheduled permalink navigation.

**Key risks/decisions:** The new query must use each backend's exact canonical
UTC date expression so the existing partial unique index applies. The public and
unpublished queries must receive one request-scoped `now`; sampling twice
creates a transient 404 at go-live. Final code removes the old free helper and
its 200-page exhaustion test rather than preserving an alias.

**Spec:**
[`../specs/2026-08-12-issue-341-issue-340-author-unpublished-permalink.md`](../specs/2026-08-12-issue-341-issue-340-author-unpublished-permalink.md)

**Architecture:** Add one object-safe method to `PostStorage`, implemented once
by the generic `PostStore<DB>`. Reuse `PostDialect::PERMALINK_DATE_CLAUSE`,
broadening its expression from `published_at` to the canonical
`COALESCE(published_at, created_at)` date on both dialects. The web server
function captures one instant, tries the public visibility-filtered lookup, then
uses the new author-unpublished lookup only after authentication and namespace
ownership checks.

**Tech Stack:** Rust 2024, async-trait, sqlx SQLite/PostgreSQL dialects,
rstest/rstest_reuse dual-backend harness, Leptos server functions, Playwright
TypeScript.

## Global Constraints

- Preserve ADR-0019: shared query body in `storage/src/posts.rs`; only
  backend-specific date syntax in `storage/src/{sqlite,postgres}/posts.rs`.
- Preserve ADR-0020: an author always sees their own Post, while another user
  and an anonymous viewer cannot discover unpublished Posts.
- Backend-common storage and server regressions use `#[apply(backends)]`; no
  hand-rolled pool.
- Use the existing `posts_user_date_slug` index expression; add no migration or
  index.
- One request captures one `DateTime<Utc>` and passes it to both permalink
  lookups.
- Remove the paginated helper, 200-iteration bound, exhaustion mock, obsolete
  imports, and all call sites; no compatibility shim.
- Extend the existing Playwright page rather than adding a document boot; use
  `navigateInApp`.
- Run commands through `devtool run --`; invoke pinned tools directly.
- Before each commit, follow `jaunder-commit`, stage the exact checked tree, and
  add no `Co-Authored-By` trailer.

---

### Task 1: Direct author-unpublished storage lookup

**Files:**

- Modify: `storage/src/posts.rs` — trait contract, generic query, canonical-date
  docs, and dual-backend direct-lookup regression.
- Modify: `storage/src/sqlite/posts.rs` — canonical SQLite date clause.
- Modify: `storage/src/postgres/posts.rs` — canonical PostgreSQL UTC date
  clause.
- Modify: `server/tests/storage/mod.rs` — exact public scheduled/live boundary
  assertion.
- Include in commit:
  `docs/superpowers/specs/2026-08-12-issue-341-issue-340-author-unpublished-permalink.md`,
  this plan with Task 1 checked.

**Interfaces:**

- Consumes: `PostDialect::TAGS_SUBQUERY`, `PostDialect::PERMALINK_DATE_CLAUSE`,
  the existing `posts_user_date_slug` index, `UserId`, `PermalinkDate`, `Slug`,
  `DateTime<Utc>`.
- Produces:

```rust
async fn get_unpublished_post_by_permalink(
    &self,
    user_id: UserId,
    date: PermalinkDate,
    slug: &Slug,
    now: DateTime<Utc>,
) -> sqlx::Result<Option<PostRecord>>;
```

- Query invariant: one `fetch_optional` over `posts p JOIN users u`, selecting
  the normal hydrated `PostRow`, with `p.user_id = $1`, `p.slug = $2`, the
  dialect canonical-date clause bound at `$3`,
  `(p.published_at IS NULL OR p.published_at > $4)`, and `p.deleted_at IS NULL`.
  No list call, cursor, loop, or visibility-resolution fragment.

- [x] **Step 1: Write the failing dual-backend direct-lookup regression**

Replace the current helper-level
`find_draft_by_permalink_for_user_finds_draft_and_misses` test in
`storage/src/posts.rs` with a `#[apply(backends)]` test named
`get_unpublished_post_by_permalink_matches_canonical_date_and_scope`. Its
complete behavioral table is:

```rust
let now = Utc::now();
let scheduled_at = now + chrono::Duration::days(30);
let author = SeedUser::new().seed(&env.state).await.user_id;
let other = SeedUser::new().seed(&env.state).await.user_id;

let draft = SeedRawPost::new(author).draft().seed(&env.state).await;
let scheduled = SeedRawPost::new(author)
    .published_at(scheduled_at)
    .seed(&env.state)
    .await;
let live_at_boundary = SeedRawPost::new(author)
    .published_at(now)
    .seed(&env.state)
    .await;
let deleted = SeedRawPost::new(author)
    .published_at(scheduled_at)
    .seed(&env.state)
    .await;
posts.soft_delete_post(deleted.post_id).await.unwrap();

let draft_record = posts
    .get_post_by_id(draft.post_id, &ViewerIdentity::Local { user_id: author })
    .await
    .unwrap()
    .expect("author can read seeded draft");
let draft_date = PermalinkDate::from(draft_record.created_at.date_naive());
let scheduled_date = PermalinkDate::from(scheduled_at.date_naive());

assert_eq!(
    posts
        .get_unpublished_post_by_permalink(author, draft_date, &draft.slug, now)
        .await
        .unwrap()
        .map(|post| post.post_id),
    Some(draft.post_id),
);
assert_eq!(
    posts
        .get_unpublished_post_by_permalink(author, scheduled_date, &scheduled.slug, now)
        .await
        .unwrap()
        .map(|post| post.post_id),
    Some(scheduled.post_id),
);

for (user_id, date, slug) in [
    (other, scheduled_date, &scheduled.slug),
    (author, scheduled_date, &parse_slug("missing")),
    (author, PermalinkDate::from(now.date_naive()), &live_at_boundary.slug),
    (author, scheduled_date, &deleted.slug),
] {
    assert!(
        posts
            .get_unpublished_post_by_permalink(user_id, date, slug, now)
            .await
            .unwrap()
            .is_none()
    );
}
```

Keep the date and slug values owned long enough for the table; if Rust rejects
the temporary `parse_slug("missing")` reference, bind
`let missing = parse_slug("missing");` before the array. Delete the old
mock-only `find_draft_by_permalink_returns_none_after_exhausting_pages` test
only in Task 2, when its production helper is removed.

- [x] **Step 2: Pin the exact public scheduled/live boundary**

In `permalink_hides_scheduled_until_due`, change the successful public lookup
from one second after `published_at` to exactly `published_at`. Assert the
scheduled Post is absent for an injected instant before publication and present
when `now == published_at`. This proves the `<= now` side that complements the
new method's strict `> now` predicate.

- [x] **Step 3: Run the new contract and verify red**

Run:
`devtool run -- cargo nextest run -p storage get_unpublished_post_by_permalink`

Expected: FAIL at compile time because
`PostStorage::get_unpublished_post_by_permalink` does not exist. The existing
public-boundary test remains green independently.

- [x] **Step 4: Implement the object-safe direct query**

Add the produced method signature and rustdoc to `PostStorage`. Implement it in
`impl<DB> PostStorage for PostStore<DB>` with one traced `fetch_optional`, using
the query invariant above and `post_record_from_row` for decode validation/tag
hydration. Bind `date.to_string()` as the third parameter and `now` as the
fourth.

Change the two dialect constants to the index-identical canonical expressions:

```rust
// SQLite
"date(COALESCE(p.published_at, p.created_at)) = $3"

// PostgreSQL
"date(COALESCE(p.published_at, p.created_at) AT TIME ZONE 'UTC') = $3::date"
```

Update `PostDialect` rustdoc to call this the canonical permalink date. The
existing public query remains behaviorally identical because it already requires
`published_at IS NOT NULL`.

- [x] **Step 5: Run targeted storage tests and verify green**

Run:

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage get_unpublished_post_by_permalink
devtool run -- devtool pg run -- cargo nextest run -p jaunder permalink_hides_scheduled_until_due
```

Expected: PASS on SQLite and PostgreSQL for both names.

- [x] **Step 6: Gate and commit Task 1**

Follow `jaunder-commit`. Run `devtool run -- cargo xtask check`; inspect its
JSON result and require `ok: true`. Check Task 1 in this plan, stage the spec,
plan, three storage files, and `server/tests/storage/mod.rs`, then commit:

```text
feat(storage): add direct unpublished permalink lookup (#340)
```

### Task 2: Web cutover and scheduled permalink navigation

**Files:**

- Modify: `storage/src/posts.rs` — inject time into `fetch_post_record`; remove
  `find_draft_by_permalink_for_user` and its exhaustion mock.
- Modify: `web/src/posts/api.rs` — one request timestamp; call direct lookup;
  remove obsolete import.
- Modify: `server/src/projector/mod.rs` — pass the projector's current instant
  to the public helper.
- Modify: `server/tests/web/web_posts.rs` — replace the pagination-shaped
  regression with scheduled canonical-permalink behavior.
- Modify: `end2end/tests/posts.spec.ts` — follow the scheduled row's permalink
  and assert the Post renders.
- Modify in commit: this plan with Task 2 checked.

**Interfaces:**

- Consumes: Task 1's `PostStorage::get_unpublished_post_by_permalink`.
- Changes:

```rust
pub async fn fetch_post_record(
    posts: &dyn PostStorage,
    viewer: &ViewerIdentity,
    username: &Username,
    date: PermalinkDate,
    slug: &Slug,
    now: DateTime<Utc>,
) -> InternalResult<Option<PostRecord>>;
```

- Produces: `web::posts::get` captures one `let now = Utc::now();`, passes it to
  `fetch_post_record`, and passes the unchanged value to
  `get_unpublished_post_by_permalink` after authentication and username
  ownership checks.

- [x] **Step 1: Replace the obsolete scan-shaped server regression with the bug
      reproduction**

Replace `get_post_finds_author_draft_across_multiple_pages` in
`server/tests/web/web_posts.rs` with
`get_post_returns_scheduled_post_at_canonical_permalink_to_author`. Use the
shared backend harness and this contract:

```rust
let TestEnv { state, base: _base } = backend.setup().await;
let author = create_user_and_session(&state).await;
let scheduled_at = chrono::Utc::now() + chrono::Duration::days(30);
let scheduled = SeedRawPost::new(author.user_id)
    .published_at(scheduled_at)
    .seed(&state)
    .await;

let (status, body) = get_post_form(
    &state,
    &author.username,
    scheduled_at.year(),
    scheduled_at.month(),
    scheduled_at.day(),
    scheduled.slug.as_ref(),
    Some(&author.cookie()),
)
.await;

assert_eq!(status, StatusCode::OK, "body: {body}");
let returned: AuthoredPost = serde_json::from_str(&body).unwrap();
assert_eq!(returned.post.post_id, scheduled.post_id);
assert!(returned.post.is_author);
```

The existing `get_post_returns_draft_to_author_only` continues to pin true-draft
resolution and non-author denial.

- [x] **Step 2: Extend the scheduled-Post Playwright scenario**

In `scheduling a post shows a Scheduled-for badge on the drafts page`, retain
the badge assertions, then add:

```typescript
const permalinkLink = scheduledRow.locator('a:has-text("Permalink")');
const permalinkHref = await permalinkLink.getAttribute("href");
expect(permalinkHref).toBeTruthy();
expect(permalinkHref).toMatch(/\/2999\/01\/01\//);
await navigateInApp(page, () => permalinkLink.click(), {
  url: permalinkHref!,
  ready: "article.j-post",
});
await expect(page.locator("article.j-post")).toContainText("Scheduled Draft");
```

This is an in-app route transition from the page's sole initial boot; do not
call `goto` or `allowSecondBoot`.

- [x] **Step 3: Run the server reproduction and verify red**

Run:
`devtool run -- devtool pg run -- cargo nextest run -p jaunder get_post_returns_scheduled_post_at_canonical_permalink_to_author`

Expected: FAIL with the current not-found response because the old fallback
compares the requested publication date with `created_at`.

- [x] **Step 4: Cut every caller over with one timestamp**

Before modifying the exported helper, use LSP references on `fetch_post_record`
and update every result.

In `storage/src/posts.rs`, add the `now` parameter to `fetch_post_record` and
forward it to `get_post_by_permalink` instead of sampling internally. Remove
`find_draft_by_permalink_for_user` completely, including its paging rustdoc and
200-page loop. Remove the mock exhaustion test and any imports used only by it.

In `web/src/posts/api.rs`, remove the free helper import. At the start of `get`,
after obtaining the store, capture `let now = Utc::now();`. Pass `now` to the
public helper. After the existing authentication and `auth.username == username`
check, call:

```rust
let post = posts
    .get_unpublished_post_by_permalink(auth.user_id, date, &slug, now)
    .await?
    .ok_or_else(not_found_error)?;
Ok(authored_post(post, true))
```

In `server/src/projector/mod.rs`, pass `chrono::Utc::now()` to
`fetch_post_record`; the anonymous projector has only the public lookup and
therefore no cross-query boundary to share.

- [x] **Step 5: Run the focused Rust regressions and verify green**

Run:

```bash
devtool run -- devtool pg run -- cargo nextest run -p jaunder get_post_returns_scheduled_post_at_canonical_permalink_to_author
devtool run -- devtool pg run -- cargo nextest run -p jaunder get_post_returns_draft_to_author_only
devtool run -- devtool pg run -- cargo nextest run -p storage get_unpublished_post_by_permalink
```

Expected: PASS on SQLite and PostgreSQL. Also verify by structural inspection
that `find_draft_by_permalink_for_user`, its 200-iteration bound, and all
references are absent.

- [x] **Step 6: Run the user-facing scheduled permalink scenario**

Run: `devtool run -- cargo xtask e2e-local posts.spec.ts`

Expected: PASS; the scheduled row links to `/~<author>/2999/01/01/<slug>`,
in-app navigation reaches `article.j-post`, and the article contains
`Scheduled Draft`.

- [x] **Step 7: Gate and commit Task 2**

Follow `jaunder-commit`. Run `devtool run -- cargo xtask check`; inspect its
JSON result and require `ok: true`. Check Task 2 in this plan, stage all five
source/test files plus the plan, then commit:

```text
fix(posts): resolve scheduled author permalinks directly (#341, #340)
```

The ship phase runs the final `cargo xtask validate`, Standards/Spec review,
push/PR monitoring, and merge approval gate.
