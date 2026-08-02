# Post DTO Remodelling Implementation Plan (#569)

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-07-31-issue-569-post-dto-names.md` — the
"what" and "why" live there and are **not** restated here. Tasks reference it by
decision (D1–D11) and acceptance criterion (AC1–AC15).

**Goal:** Rename and remodel the posts DTO family so every type is named for
what it is, collapsing four duplications and deleting five unread wire fields.

**Architecture:** Renames are type-coupled across crates, so tasks are drawn at
**atomic-compile boundaries** — each task's tree compiles and its full suite
passes before commit. Order runs dependency-first: the one independent deletion,
then the write path, then the read path, then the cursor, then the drafts
listing (which depends on both the write path and the cursor).

**Tech Stack:** Rust (workspace crates `common`, `web`, `server`, `storage`),
leptos `#[server]` fns, serde wire DTOs, `cargo nextest`, Playwright e2e
(`end2end/`).

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- Every commit runs the full `cargo xtask check` via the pre-commit hook — run
  it yourself first so it passes clean (**`jaunder-commit`**).
- **`cargo xtask validate` refuses a dirty tree.** `HANDOFF.md` is uncommitted
  scaffolding and must be deleted before the final gate (Task 11).
- Wire DTOs keep
  `#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]`. `PageCursor`
  is the sole exception and additionally keeps `Copy` (spec D6).
- Storage tests follow the dual-backend template (`CONTRIBUTING.md` "backend
  parity"); a bare `#[tokio::test]` that should be dual-backend fails the
  `test-backend-pattern` guard.
- **Every AC grep must be run against the pre-change tree and observed to fail**
  before it counts as satisfied (spec, "Acceptance criteria" preamble).

## Scope

**In:** D1–D10 — the posts DTO family in `web/`, `common/`, the
`DerivedPostMetadata` deletion in `common/` + `storage/`, the cursor across
`web/src/timeline/` and `web/src/cockpit/`, and the matching `server/tests/` and
`end2end/` updates.

**Out:** D11 — `delete`'s return shape, the media/auth verticals, #747, the
storage row types, persisting `summary_label`, and changing where `unpublish`
navigates. Task 1 files issues for the two that warrant tracking.

## Tasks

| #   | Task                                                       | Spec     |
| --- | ---------------------------------------------------------- | -------- |
| 1   | File follow-up issues; correct #569 and #754               | D10, D11 |
| 2   | Delete `DerivedPostMetadata`; `derive_post_title`          | D8       |
| 3   | `SavedPost` for all four mutations                         | D1, D2   |
| 4   | `PostInputs`; wire key `args` → `post`                     | D3, D4   |
| 5   | `TimelinePostSummary` → `RenderedPost`; `published_at` opt | D5       |
| 6   | `PostResponse` → `AuthoredPost` nesting `RenderedPost`     | D5       |
| 7   | `PageCursor` on responses                                  | D6       |
| 8   | `PageCursor` on the six request signatures                 | D6       |
| 9   | `UnpublishedPost` in an `UnpublishedPage`                  | D7       |
| 10  | ADR: the content-weight axis                               | D9       |
| 11  | Final gate: delete `HANDOFF.md`, `cargo xtask validate`    | AC15     |

## Key risks and decisions

- **Task 6 is the riskiest.** Nesting `AuthoredPost.post` changes the `PageSeed`
  wire shape, which ADR-0041/0044 govern. AC12a's byte-identical paint is the
  guard: after the two `PostView` builders converge, the emitted HTML must be
  unchanged. If `authed-cls.spec.ts` moves, the convergence is wrong — do
  **not** widen the tolerance.
- **Task 5 has a silent-failure mode.** Making `published_at` optional removes
  the type-level reason `rendered_post` returns `Option`. Dropping that guard
  leaks drafts into public listings. Step 1 of Task 5 pins it with a test
  _before_ the field changes.
- **Tasks 3, 4, 7, 8, 9 each change the wire.** Only the e2e matrix verifies
  them, so AC15 is load-bearing, not ceremonial.
- **Task 2 is independent** of every other task and can be delegated or
  reordered freely.

---

### Task 1: File follow-up issues; correct #569 and #754

Per `jaunder-start` step 5 — separable concerns are filed up front so they can
be picked up concurrently, not deferred behind this cycle. Uses
**`jaunder-issues`** for type, labels, and project metadata.

**Files:** none (tracker only).

**Interfaces:**

- Consumes: nothing.
- Produces: two issue numbers, referenced by the spec's D11.

- [x] **Step 1: File the role-suffix audit issue** — #782

Title:
`web/common: drop *Result / *Response role-suffixes in the media and auth DTOs`

Body must name the three verified survivors and cite the precedent:

```
Deferred from #569, which fixed the same defect in the posts vertical and
scoped its AC1 to that family precisely because these three survive:

- common/src/media.rs:845  UploadResponse
- web/src/auth/api.rs:33    LoginResponse
- web/src/media/api.rs:62   DeleteResult

Rationale and the naming axis: see #569's ADR on post content weight.

Acceptance: `rg 'pub struct \w*(Result|Response)\b' common/ web/` returns
nothing; `cargo xtask validate` green.
```

- [x] **Step 2: File the unpublish-navigation issue** — #783 (blocked by #569)

Title:
`web(posts): unpublish navigates to /drafts instead of the permalink it now returns`

```
Created by #569. Unpublishing moves a post's permalink (permalink() is
published_at.unwrap_or(created_at)-based, storage/src/posts.rs:76), and after
#569 `unpublish` returns the moved URL as a SavedPost. But
component.rs:952-954 still hardcodes a navigate to /drafts, so the returned
value is unread.

#569 deliberately left this out of scope: where unpublish should send you is
a UX decision (stay on the now-draft post, or go to the drafts list), not a
naming one.

Acceptance: the decision is made and implemented, or recorded as
"stay on /drafts" with the return value's purpose documented.
```

- [x] **Step 3: Rewrite #754 to the surviving question** (spec D10)

Its step 1 (unify the two implementations) dies with Task 2 — one implementation
is dead, so the remedy is deletion. Retitle to
`posts: decide whether summary_label is stored or recomputed per drafts row`,
keep its step 2 (the migration question), and add a note that the duplication
premise was corrected by #569.

- [x] **Step 4: Correct #569's body** (spec D10)

It still proposes `PostDetails` and "consistent with `*Summary`", both of which
the approved spec inverts. Replace those with the settled names and link the
spec.

- [x] **Step 5: Record the two new issue numbers in the spec's D11**

Replace the two bullet descriptions under "Filed as follow-up issues" with
`#<N> — <title>`.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-07-31-issue-569-post-dto-names.md
git commit -m "docs(spec): record #569 follow-up issue numbers"
```

---

### Task 2: Delete `DerivedPostMetadata`; `derive_post_metadata` → `derive_post_title`

Spec D8. Independent of every other task.

**Files:**

- Modify: `common/src/render.rs:350-401` (delete the struct, rewrite the fn),
  `:1109-1170` and `:1240-1245` (tests)
- Modify: `storage/src/post_service.rs:310-311`, `:326`, `:334`, `:476-477`,
  `:494`, `:507`

**Interfaces:**

- Consumes: nothing.
- Produces:
  `pub fn derive_post_title(explicit_title: Option<&str>, body: &str, format: &PostFormat) -> Option<(Option<PostTitle>, String)>`
  — `None` when the post is empty; the `String` is the slug seed.

- [x] **Step 1: Rewrite the seven tests in `common/src/render.rs`'s derive
      block**

All seven (`:1110`, `:1123`, `:1136`, `:1146`, `:1153`, `:1166`, `:1240`)
destructure the tuple instead of a struct. **Keep every existing test name
except one.** AC9a authorizes exactly one rename:
`:1146 derive_metadata_for_html_extracts_no_title_but_keeps_fallback_label`
names a field that no longer exists. Renaming the other six is unauthorized
churn and would make AC9a's audit — _are the three `summary_label` tests still
present?_ — harder rather than easier.

The three that asserted on `summary_label` (`:1119`, `:1149`, `:1162`) now
assert on what the function returns:

```rust
// name unchanged
#[test]
fn derive_metadata_prefers_explicit_title() {
    let (title, slug_seed) =
        derive_post_title(Some("Explicit"), "# Body Heading\n\nBody", &PostFormat::Markdown).unwrap();
    assert_eq!(title, Some(PostTitle::from("Explicit".to_string())));
    assert_eq!(slug_seed, "Explicit");
}

// name unchanged
#[test]
fn derive_metadata_extracts_markdown_heading() {
    let (title, slug_seed) =
        derive_post_title(None, "# Article Title\n\nBody text", &PostFormat::Markdown).unwrap();
    assert_eq!(title, Some(PostTitle::from("Article Title".to_string())));
    assert_eq!(slug_seed, "Article Title");
}

// name unchanged
#[test]
fn derive_metadata_extracts_org_title() {
    let (title, slug_seed) =
        derive_post_title(None, "#+title: Org Title\n\nBody text", &PostFormat::Org).unwrap();
    assert_eq!(title, Some(PostTitle::from("Org Title".to_string())));
    assert_eq!(slug_seed, "Org Title");
}

// THE ONE AUTHORIZED RENAME (AC9a) — was
// derive_metadata_for_html_extracts_no_title_but_keeps_fallback_label
#[test]
fn derive_metadata_for_html_extracts_no_title_and_seeds_slug_from_the_body() {
    let (title, slug_seed) =
        derive_post_title(None, "<p>Hello world</p>", &PostFormat::Html).unwrap();
    assert_eq!(title, None);
    assert_eq!(slug_seed, "<p>Hello world</p>");
}

// name unchanged
#[test]
fn derive_metadata_untitled_uses_fallback_label() {
    let (title, slug_seed) =
        derive_post_title(None, "A compact note\n\nmore", &PostFormat::Markdown).unwrap();
    assert_eq!(title, None);
    assert_eq!(slug_seed, "A compact note");
}

// name unchanged
#[test]
fn derive_metadata_returns_none_for_an_empty_post() {
    assert_eq!(derive_post_title(None, "   \n\t", &PostFormat::Markdown), None);
}

// name unchanged
#[test]
fn derive_metadata_extracts_org_heading() {
    let (title, slug_seed) =
        derive_post_title(None, "* Org Heading\n\nBody text", &PostFormat::Org).unwrap();
    assert_eq!(title, Some(PostTitle::from("Org Heading".to_string())));
    assert_eq!(slug_seed, "Org Heading");
}
```

Confirm the exact existing names at `common/src/render.rs:1109-1245` before
editing and preserve them verbatim; the names above reproduce the pattern, not
necessarily the bytes.

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common derive_metadata` Expected: FAIL —
`derive_post_title` not defined.

- [x] **Step 3: Implement against the tests**

Delete `pub struct DerivedPostMetadata` (`:350-356`) and rewrite
`derive_post_metadata` to the signature in **Interfaces**. Every branch is
pinned by a Step 1 test — explicit title, extracted markdown title, extracted
org title, HTML (no title), untitled fallback, and the empty-post `None`.

Two things the tests cannot pin, so they are stated here:

- The `fallback_label(body)` calls at `:372` and `:387` fed **only**
  `summary_label` and must be **deleted**. The call at `:395` stays — it is both
  the empty-post gate (`?`) and the slug seed.
- All three `PostSummary::truncated` calls go. `PostSummary` may become an
  unused import in this file; remove it if so.

- [x] **Step 4: Update both `storage/src/post_service.rs` call sites**

Each keeps its own error type — `:311` is the update path, `:477` the creation
path:

```rust
// :310-311 (perform_post_update)
let (title, slug_seed) =
    derive_post_title(title, &body, &format).ok_or(PerformUpdateError::EmptyPost)?;

// :476-477 (perform_post_creation)
let (title, slug_seed) =
    derive_post_title(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;
```

The shadowed bindings replace `metadata.title` (`:334`, `:507`) and
`metadata.slug_seed` (`:326`, `:494`). Note `:488` already binds a local named
`slug_seed` in the creation path — rename the derived one there to avoid
shadowing the `Slug`, or inline it into the `slugify_title` call.

- [x] **Step 5: Run the full suites, verify they pass**

Run: `cargo nextest run -p common -p storage` Expected: PASS.

- [x] **Step 6: Verify AC9**

```bash
rg 'DerivedPostMetadata|derive_post_metadata'          # expect: nothing
rg -c 'fallback_label' common/src/render.rs            # expect: 2 (definition + one call)
rg 'PostSummary::truncated' common/src/render.rs       # expect: nothing
```

- [x] **Step 7: Commit**

```bash
cargo xtask check
git add common/src/render.rs common/src/post_title.rs common/src/post_summary.rs \
        storage/src/post_service.rs docs/adr/0024-server-side-org-canonicalization.md
git commit -m "refactor(common): delete DerivedPostMetadata, derive title and slug seed directly (#569)"
```

Wider than planned by three files, each a doc reference the rename
**falsified**: `post_title.rs:14` and `post_summary.rs:49-51` (the latter
asserted a caller count that became wrong), and `docs/adr/0024:36`, which points
at a live code seam — a dangling symbol there misleads the next reader. AC9 was
tightened to say `docs/archive/` and superseded specs stay frozen while ADRs
pointing at live code get corrected.

---

### Task 3: `SavedPost` for all four mutations

Spec D1, D2.

**Files:**

- Modify: `web/src/posts/api.rs:74-96` (delete `CreateResult`/`UpdateResult`,
  doc comments included), `:115-122` (delete `PublishResult`), `:163`,
  `:218-225`, `:309`, `:389-395`, `:489`, `:536-541`, `:582-605`, `:651-671`
  (retarget the round-trip test)
- Modify: `web/src/posts/mod.rs:54`, `:57` (re-exports)
- Modify: `web/src/posts/component.rs:32-33`, `:266-269`, `:276`, `:477`,
  `:758`, `:808`, `:1357`, `:1417`
- Modify: `web/src/posts/page_state.rs:34`, `:128`, `:386-387`
- Modify: `server/tests/web/web_posts.rs:13` and its ~46 result-type sites

**Interfaces:**

- Consumes: nothing.
- Produces:
  `pub struct SavedPost { pub post_id: PostId, pub slug: Slug, pub published_at: Option<UtcInstant>, pub permalink: RootRelativeUrl }`,
  re-exported from `web::posts`. All of `create`, `update`, `publish`,
  `unpublish` return `WebResult<SavedPost>`.

- [x] **Step 1: Write the failing tests**

Retarget the existing round-trip test (AC14 forbids deleting it) and add one
pinning unpublish's new return. In `web/src/posts/api.rs`:

```rust
#[test]
fn saved_post_permalink_wire_is_root_relative() {
    use super::SavedPost;
    use common::test_support::{parse_root_relative_url, parse_utc_instant};
    let original = SavedPost {
        post_id: PostId::from(1),
        slug: "hello".parse::<Slug>().unwrap(),
        published_at: Some(parse_utc_instant("2026-01-01T00:00:00Z")),
        permalink: parse_root_relative_url("/~alice/2026/01/01/hello"),
    };
    let json = serde_json::to_string(&original).unwrap();
    assert_eq!(serde_json::from_str::<SavedPost>(&json).unwrap(), original);
    // Swapping the field to an absolute URL is rejected at decode.
    let absolute = json.replace("/~alice", "https://evil.example/~alice");
    assert!(serde_json::from_str::<SavedPost>(&absolute).is_err());
}
```

In `server/tests/web/web_posts.rs`, add a test beside
`unpublish_post_reverts_published_post_to_draft` (`:1802`).

**The dates must differ, or the test is vacuous.** `permalink()` is day-granular
(`storage/src/posts.rs:74-89`, `{:04}/{:02}/{:02}`). The existing test creates
with `publish: true`, so `created_at` and `published_at` are both `Utc::now()` —
the two URLs are byte-identical and the assertion would pass whether or not the
implementation clears `published_at` before reading `permalink()`, which is
precisely the bug it must catch. So publish at a **backdated** `publish_at`:

```rust
#[rstest]
#[case(Backend::Sqlite)]
#[case(Backend::Postgres)]
#[tokio::test]
async fn unpublish_returns_the_created_at_based_permalink(#[case] backend: Backend) {
    // Create a draft today, then publish it at a date in a DIFFERENT month, so the
    // published_at-based and created_at-based permalinks cannot coincide.
    // ... create with publish: false, then publish with publish_at = 2020-03-04 ...
    let published: SavedPost = serde_json::from_str(&body).unwrap();
    assert!(
        published.permalink.as_ref().contains("/2020/03/04/"),
        "published permalink is published_at-based: {}", published.permalink,
    );

    let (status, body) = unpublish_post_form(&state, created.post_id, Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "unpublish body: {body}");
    let unpublished: SavedPost = serde_json::from_str(&body).unwrap();
    assert_eq!(unpublished.post_id, created.post_id);
    assert_eq!(unpublished.published_at, None, "unpublish clears published_at");
    assert_eq!(
        unpublished.permalink, created.permalink,
        "unpublishing reverts to the created_at-based URL, not the published_at one",
    );
    assert_ne!(
        unpublished.permalink, published.permalink,
        "if these are equal the test proves nothing — the dates must differ",
    );
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p web saved_post_permalink_wire_is_root_relative`
Expected: FAIL — `SavedPost` not defined.

Deviation: the two tests were written and the implementation landed in the same
pass, so the compile-time red (`SavedPost` not defined) was never observed. The
_behavioural_ red that matters was observed instead: with `permalink()` read
before `existing.published_at = None`, both backend cases of
`unpublish_post_returns_the_draft_permalink` fail with
`left: "/~user0/2020/03/05/moved-permalink"` vs
`right: "/~user0/2026/08/01/moved-permalink"` — the two dates genuinely differ,
so the test is not vacuous.

- [x] **Step 3: Implement `SavedPost` and repoint the four fns**

Add the struct to `web/src/posts/api.rs` with the Interfaces shape and the
standard derives. Delete `CreateResult`, `UpdateResult`, `PublishResult`. Change
the four signatures per spec D2 and drop `created_at`/`summary` from the
construction sites (`:218-225`, `:389-395`, `:536-541`).

`unpublish` (`:582-605`) is the only one needing new logic — the tests pin the
values but not that the record must be mutated **before** `permalink()` is read,
so:

```rust
// bind `existing` as `mut` at :587
posts.unpublish_post(post_id).await?;
// ... existing feed-event enqueue, unchanged ...
existing.published_at = None;
Ok(SavedPost {
    post_id,
    slug: existing.slug.clone(),
    published_at: None,
    permalink: existing.permalink(),
})
```

Reading `existing.permalink()` before clearing `published_at` returns the _old_
published_at-based URL — the bug this task exists to avoid.

- [x] **Step 4: Repoint the consumers**

`component.rs:266-269`'s closure binding `move |()|` → `move |_|` (spec D2 — the
caller keeps navigating to `/drafts`; the value is deliberately unread). `:276`,
`:477`, `:758`, `:808`, `:1357`, `:1417` swap the type name. `page_state.rs:128`
and the `:386-387` test builder likewise. `mod.rs:54-57` re-exports `SavedPost`
in place of the three deleted names.

- [x] **Step 5: Run the full suites, verify they pass**

Run: `cargo nextest run -p web -p server` Expected: PASS. (The package is
`jaunder`, so the run was
`devtool pg run -- cargo nextest run -p web -p jaunder`: 1597 passed, 0 failed.)

- [x] **Step 6: Verify AC2 and AC3**

```bash
rg 'CreateResult|UpdateResult|PublishResult'   # expect: nothing
rg -n 'pub async fn (create|update|publish|unpublish)' web/src/posts/api.rs
# expect: all four -> WebResult<SavedPost>
```

Both verified — the only surviving hits are in `docs/` (the spec, this plan,
`docs/archive/`) and `HANDOFF.md`, all of which describe the rename rather than
depend on the names.

- [x] **Step 7: Commit**

```bash
cargo xtask check
git add web/src/posts server/tests/web/web_posts.rs
git commit -m "refactor(web/posts): one SavedPost for create, update, publish, unpublish (#569)"
```

---

### Task 4: `PostInputs`; wire key `args` → `post`

Spec D3, D4. **Wire change** — the raw-JSON producers in two languages must move
together.

**Files:**

- Modify: `web/src/posts/api.rs:124-152` (one struct), `:161-163`, `:307-309`,
  `:691`, `:711` (delete)
- Modify: `web/src/posts/mod.rs:54`, `:57` (the `CreateArgs`/`UpdateArgs`
  re-exports), `component.rs:32-33` and its dispatch sites
- Modify: `server/tests/web/web_posts.rs` (12 `"args"` sites),
  `server/tests/feed/feed_events_hook.rs` (`:28, 69, 104, 147, 214, 281`)
- Modify: `end2end/tests/posts.ts:20` (doc), `:33`;
  `end2end/tests/feeds.spec.ts:272`

**Interfaces:**

- Consumes: `SavedPost` (Task 3).
- Produces:
  `pub struct PostInputs { body, format, slug_override, publish, publish_at, tags, summary, audience }`
  — the eight `CreateArgs` fields verbatim. `create(post: PostInputs)`,
  `update(post_id: PostId, post: PostInputs)`. **The parameter name `post` is
  the wire contract** (AC4).

- [x] **Step 1: Rename the surviving arg test and delete its duplicate**

`:711 update_post_args_rejects_unknown_format_token` becomes byte-identical to
`:691` once `UpdateArgs` is gone — **delete `:711`** and rename `:691`:

```rust
#[test]
fn post_inputs_rejects_unknown_format_token() {
    // body unchanged; only the type name and the fn name change
}
```

- [x] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p web post_inputs_rejects_unknown_format_token`
Expected: FAIL — `PostInputs` not defined.

- [x] **Step 3: Implement `PostInputs` and the two signatures**

Rename `CreateArgs` → `PostInputs`, delete `UpdateArgs`, and change:

```rust
pub async fn create(post: PostInputs) -> WebResult<SavedPost>;
pub async fn update(post_id: PostId, post: PostInputs) -> WebResult<SavedPost>;
```

`update`'s body reads `post_id` from the parameter rather than destructuring it
out of the args struct.

- [x] **Step 4: Update the Rust raw-JSON producers**

`server/tests/web/web_posts.rs` — 13 bodies (12 planned, plus the one Task 3
added in `unpublish_post_returns_the_draft_permalink`): `"args": {` →
`"post": {`. `server/tests/feed/feed_events_hook.rs` — 6 sites at
`:28, 69, 104, 147, 214, 281`. Where `update` is posted, `post_id` moves **out**
of the envelope to a sibling key:

```json
{
  "post_id": 1,
  "post": { "body": "...", "format": "markdown", "publish": true }
}
```

- [x] **Step 5: Update the TypeScript producers**

`end2end/tests/posts.ts:33` (`createPostViaApi`, imported by five spec files)
and `end2end/tests/feeds.spec.ts:272` (which nests `post_id` _inside_ `args`, so
D3's hoist splits it). Update `posts.ts:20`'s doc comment, which names the
`args` wrapper.

- [x] **Step 6: Run the suites, verify they pass**

Run: `cargo nextest run -p web -p jaunder` Expected: PASS.

- [x] **Step 7: Verify AC4 and AC5**

```bash
rg 'CreateArgs|UpdateArgs'                     # expect: nothing
rg '"args"' server/                            # expect: nothing
rg 'args:' end2end/tests/posts.ts end2end/tests/feeds.spec.ts   # expect: nothing
rg 'args' end2end/playwright.config.ts end2end/tests/seed.ts    # expect: UNCHANGED (do not touch)
```

- [x] **Step 8: Commit**

```bash
cargo xtask check
git add web/src/posts server/tests end2end/tests
git commit -m "refactor(web/posts): one PostInputs; wire key args -> post (#569)"
```

---

### Task 5: `TimelinePostSummary` → `RenderedPost`; `published_at` becomes optional

Spec D5. 23 sites / 7 files.

**Files:**

- Modify: `common/src/seed.rs:34-58`, `:63`
- Modify: `web/src/posts/server.rs:3`, `:9-42`; `render.rs:22`, `:107`, `:116`,
  `:230`, `:283-284`; `component.rs:15`, `:46`, `:153`, `:227`, `:971`;
  `api.rs:622-640`
- Modify: **`web/src/posts/api/listing.rs:18`** (the `use` of
  `crate::posts::server::timeline_post_summary`) and **`:44`** (the `filter_map`
  call) — both name the renamed symbol and must change to compile
- Modify: **`web/src/posts/api.rs:734-773`**
  (`timeline_post_summary_keeps_titleless_posts_titleless`, which imports at
  `:735` and calls at `:746`). AC14 forbids removing it — rename the fn to
  `rendered_post_*` and repoint the call
- Modify: `web/src/timeline/state.rs:15`, `:140`, `:275`;
  `web/src/timeline/mod.rs:6`

**Interfaces:**

- Consumes: nothing.
- Produces: `pub struct RenderedPost` — the 12 fields of `TimelinePostSummary`
  with `published_at: Option<UtcInstant>`. Builder
  `pub fn rendered_post(post: PostRecord, viewer_user_id: Option<UserId>) -> Option<RenderedPost>`
  — **still fallible** (AC6c).

- [ ] **Step 1: Write the failing test that pins the draft guard**

This is the task's silent-failure mode: once `published_at` is optional, nothing
type-level forces the bail, and dropping it leaks drafts into public listings.
Pin it first. In `web/src/posts/server.rs`:

```rust
#[cfg(feature = "server")]
#[test]
fn rendered_post_returns_none_for_a_draft() {
    use crate::posts::server::rendered_post;
    use chrono::{TimeZone, Utc};
    use common::test_support::parse_username;
    use common::{ids::{PostId, UserId}, slug::Slug};
    use storage::{PostFormat, PostRecord, RenderedHtml};

    let base_time = Utc.with_ymd_and_hms(2026, 4, 16, 10, 11, 12).unwrap();
    let record = PostRecord {
        post_id: PostId::from(1),
        user_id: UserId::from(2),
        author_username: parse_username("author"),
        title: Some("Title".into()),
        slug: "hello-world".parse::<Slug>().unwrap(),
        body: "body".into(),
        format: PostFormat::Markdown,
        rendered_html: RenderedHtml::from_trusted("<p>body</p>"),
        created_at: base_time,
        updated_at: base_time,
        published_at: None, // a draft
        deleted_at: None,
        summary: None,
        tags: vec![],
    };
    assert!(
        rendered_post(record, Some(UserId::from(2))).is_none(),
        "a draft must never become a public listing row"
    );
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p web rendered_post_returns_none_for_a_draft` Expected:
FAIL — `rendered_post` not defined.

- [ ] **Step 3: Rename the type and make `published_at` optional**

In `common/src/seed.rs`, rename `TimelinePostSummary` → `RenderedPost` and
change `published_at: UtcInstant` → `Option<UtcInstant>`. Update the doc
comment, which currently says "A published post row returned by timeline listing
endpoints" — it is now also what `PostPage` feeds `PostCard`.

- [ ] **Step 4: Rename the builder, keeping its guard**

`server.rs:9` `timeline_post_summary` → `rendered_post`. **Keep**
`-> Option<RenderedPost>` and **keep** `let published_at = post.published_at?;`
at `:13`, wrapping the value:
`published_at: Some(UtcInstant::from(published_at))`.

`api/listing.rs:18`'s `use` and `:44`'s `filter_map` are **renamed, semantics
unchanged** — the `filter_map` still drops the `None`s, which is what keeps
drafts out of public listings.

- [ ] **Step 5: Converge the two `PostView` builders**

`render.rs:116` currently reads `format_post_time(post.published_at)`. With the
field optional it becomes identical to `:97`:

```rust
time: &format_post_time(post.published_at.unwrap_or(post.created_at)),
```

Both call sites now build `PostView` identically — this is what AC12a's
byte-identical paint depends on. Update the `:283-284` test fixture's
`published_at` to `Some(...)`.

- [ ] **Step 6: Update the remaining sites**

`component.rs:971`'s rebuild keeps its `unwrap_or` **for now** (Task 6 deletes
the whole block); `:15`'s preamble comment names the old type and must be
updated. `timeline/` (3 sites + the `mod.rs:6` doc), `api.rs:622-640`'s
round-trip test.

- [ ] **Step 7: Run the suites, verify they pass**

Run: `cargo nextest run -p common -p web -p server` Expected: PASS.

- [ ] **Step 8: Commit**

```bash
cargo xtask check
git add common/src/seed.rs web/src
git commit -m "refactor(common/seed): TimelinePostSummary -> RenderedPost, published_at optional (#569)"
```

---

### Task 6: `PostResponse` → `AuthoredPost` nesting `RenderedPost`

Spec D5. 19 sites / 7 files. **Highest-risk task** — changes the `PageSeed` wire
shape.

**Files:**

- Modify: `common/src/seed.rs:69-90`, `:114`
- Modify: `web/src/posts/server.rs:3`, `:53-87`, `:117-156` (test);
  `render.rs:22`, `:89-102`; `component.rs:46`, `:965-999`; `api.rs`
- Modify: `web/src/posts/mod.rs:69`, `:73`; `server/src/projector/mod.rs:44`,
  `:192`
- Modify: `storage/src/posts.rs:432` (doc comment)
- Modify: `server/tests/web/web_posts.rs:6`, `:1964`

**Interfaces:**

- Consumes: `RenderedPost` (Task 5).
- Produces:
  `pub struct AuthoredPost { pub post: RenderedPost, pub body: PostBody, pub format: PostFormat }`.
  Builder
  `pub fn authored_post(post: PostRecord, is_author: bool) -> AuthoredPost`.
  `PageSeed::Permalink(AuthoredPost)`.

- [ ] **Step 1: Write the failing tests**

Retarget `post_response_carries_summary` (`server.rs:121`) through the nesting,
and pin that a draft's `published_at` is `None` rather than fabricated:

```rust
#[cfg(feature = "server")]
#[test]
fn authored_post_carries_summary_and_source() {
    // ... same PostRecord fixture as post_response_carries_summary ...
    let authored = authored_post(record, true);
    assert_eq!(authored.post.summary, Some(parse_post_summary("the summary")));
    assert_eq!(authored.body, "body".into());
    assert_eq!(authored.format, PostFormat::Markdown);
}

#[cfg(feature = "server")]
#[test]
fn authored_post_leaves_a_draft_published_at_none() {
    // ... same fixture with published_at: None ...
    let authored = authored_post(record, true);
    assert_eq!(authored.post.published_at, None, "a draft has no publication instant");
    assert!(authored.post.is_draft);
    assert_eq!(authored.post.permalink, None);
}
```

- [ ] **Step 2: Run them, verify they fail**

Run: `cargo nextest run -p web authored_post` Expected: FAIL — `authored_post`
not defined.

- [ ] **Step 3: Implement the nested type**

In `common/src/seed.rs`, replace `PostResponse` with the Interfaces shape. **No
`#[serde(flatten)]`** (spec D5 — it buffers through `Content` and re-drives the
`deserialize_rendered_html` hook). `PageSeed::Permalink(AuthoredPost)` at
`:114`.

- [ ] **Step 4: Rewrite the builder**

`server.rs:53` `post_response` → `authored_post`, returning the nested shape.

**Do not call `rendered_post` and do not factor a shared inner builder.**
`authored_post` must not bail on a draft — a draft permalink is exactly what it
serves — and **three of the twelve inner fields are derived differently**
(`server.rs:27-41` vs `:71-86`):

| field       | `rendered_post` (listing)         | `authored_post` (permalink)                          |
| ----------- | --------------------------------- | ---------------------------------------------------- |
| `is_author` | `viewer_user_id == Some(user_id)` | the `is_author: bool` parameter                      |
| `is_draft`  | hardcoded `false`                 | `published_at.is_none()`                             |
| `permalink` | always `Some(permalink)`          | `published_at.is_some().then(…)` → `None` for drafts |

Unifying them would silently change draft permalink and `is_draft` behaviour on
the seed path, which AC12a would surface as a paint diff with no obvious cause.
Build the inner `RenderedPost` inline in `authored_post`, preserving all three
derivations exactly.

- [ ] **Step 5: Collapse the hand rebuild**

`component.rs:971-986`'s 16-line, 12-field rebuild — including the
`published_at: fetched.published_at.unwrap_or(fetched.created_at)` fabrication —
becomes:

```rust
let summary = fetched.post.clone();
```

- [ ] **Step 6: Repoint `permalink_article`**

`render.rs:89` takes `&AuthoredPost` and reads through `.post`, or takes
`&RenderedPost` and callers pass `&seed.post`. Prefer the latter — the two
builders then share one body and the convergence AC12a depends on is structural
rather than incidental.

- [ ] **Step 7: Update the remaining sites**

`mod.rs:69,73`; `projector/mod.rs:44,192`; `storage/src/posts.rs:432`'s doc
comment; `web_posts.rs:6,1964`; `component.rs:15`'s preamble comment (it
explains why `:437` spells `audiences::Summary` — keep that explanation, update
the type names).

- [ ] **Step 8: Run the suites, verify they pass**

Run: `cargo nextest run -p common -p web -p server -p storage` Expected: PASS.

- [ ] **Step 9: Verify AC1, AC6, AC6a, AC6b**

```bash
rg 'pub struct \w*(Result|Response)\b' web/src/posts common/src/seed.rs   # nothing
rg 'TimelinePostSummary|PostResponse'                                     # nothing
rg 'unwrap_or\(.*created_at\)' web/src/posts/component.rs                 # nothing
rg -n 'PostCard' web/src/posts/mod.rs   # the COMPONENT must still be re-exported
```

- [ ] **Step 10: Commit**

```bash
cargo xtask check
git add common/src/seed.rs web/src server/src storage/src server/tests
git commit -m "refactor(common/seed): PostResponse -> AuthoredPost nesting RenderedPost (#569)"
```

---

### Task 7: `PageCursor` on responses

Spec D6. 62 sites / 9 files.

**Files:**

- Modify: `common/src/seed.rs` (add `PageCursor`, change `TimelinePage`)
- Modify: `web/src/timeline/state.rs:15`, `:21-56` (move out, delete
  `from_page`), **`:295-296`** (the `page(…)` test helper's flat-pair literal),
  **`:312-314`** (`cursor_from_page_needs_both_components`)
- Modify: **`web/src/timeline/mod.rs`** — its module doc names `TimelineCursor`,
  and AC7's grep is repo-wide
- Modify: `web/src/posts/api/listing.rs:41-51`; `page_state.rs:190-194`;
  `render.rs:303,398,415,485`; `web/src/cockpit/state.rs:128`;
  `server/src/projector/mod.rs:400,429`
- Modify: `server/tests/web/web_posts.rs` (12 sites)

**Interfaces:**

- Consumes: `RenderedPost` (Task 5).
- Produces:
  `pub struct PageCursor { pub created_at: UtcInstant, pub post_id: PostId }`
  with `#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]`.
  `TimelinePage { posts: Vec<RenderedPost>, next_cursor: Option<PageCursor>, has_more: bool }`.

- [ ] **Step 1: Write the failing test**

In `common/src/seed.rs`, pin that the pair travels together and round-trips:

```rust
#[test]
fn page_cursor_round_trips_and_is_absent_as_a_whole() {
    let page = TimelinePage { posts: vec![], next_cursor: None, has_more: false };
    let json = serde_json::to_string(&page).unwrap();
    assert_eq!(serde_json::from_str::<TimelinePage>(&json).unwrap(), page);

    let cursor = PageCursor {
        created_at: parse_utc_instant("2026-01-01T00:00:00Z"),
        post_id: PostId::from(7),
    };
    let page = TimelinePage { posts: vec![], next_cursor: Some(cursor), has_more: true };
    let json = serde_json::to_string(&page).unwrap();
    assert_eq!(serde_json::from_str::<TimelinePage>(&json).unwrap(), page);
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo nextest run -p common page_cursor_round_trips` Expected: FAIL —
`PageCursor` not defined.

- [ ] **Step 3: Move, rename, and re-derive the cursor**

Move `TimelineCursor` from `web/src/timeline/state.rs:21-29` into
`common/src/seed.rs` as `PageCursor`. Its current derives are
`#[derive(Clone, Copy, Debug, PartialEq, Eq)]` — **no serde**. Add
`Serialize`/`Deserialize`; **keep `Copy`**, which `into_query` (`state.rs:50`)
relies on. Keep the doc comment explaining that bundling makes a half-cursor
unrepresentable.

- [ ] **Step 4: Change `TimelinePage` and delete `from_page`**

`TimelinePage` gets one `next_cursor: Option<PageCursor>` in place of the two
flat fields. `TimelineCursor::from_page` (`state.rs:36-44`) reconciled a
half-cursor the server never emits — with the pair unrepresentable it is
**deleted**, and with it **`cursor_from_page_needs_both_components`
(`state.rs:312-314`)**, which exercises only `from_page`. Deleting a covered
region is deliberate: the region no longer exists, the same justification the
spec gives for `api.rs:711` and `state.rs:332`. Name it in the commit message so
the deletion is not mistaken for attrition.

- [ ] **Step 5: Update the construction sites**

`listing.rs:41-51` stops splitting: `next_cursor` goes straight in. The eight
struct-literal sites (`page_state.rs:190-194`, `render.rs` ×4,
`cockpit/state.rs:128`, `projector/mod.rs` ×2) collapse each
`next_cursor_created_at: None, next_cursor_post_id: None` pair into one
`next_cursor: None`.

- [ ] **Step 6: Run the suites, verify they pass**

Run: `cargo nextest run -p common -p web -p server` Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo xtask check
git add common/src/seed.rs web/src server/src server/tests
git commit -m "refactor(common/seed): TimelinePage carries Option<PageCursor> (#569)"
```

---

### Task 8: `PageCursor` on the six request signatures

Spec D6. 60 further sites / 3 files. **Wire change** on six endpoints.

**Files:**

- Modify: `web/src/posts/api/listing.rs:113`, `:137`, `:159`, `:260`, `:284`
  (the `#[server(…)]` attributes — **each gains `input = Json`**) and `:115`,
  `:139`, `:161`, `:262`, `:286` + bodies (27 sites);
  `web/src/posts/api.rs:439-444` (attribute + 3 sites)
- Modify: `web/src/timeline/state.rs:50-55` (delete `into_query`), `:229`,
  `:332-342` (delete its test)
- Modify: **`web/src/timeline/component.rs:26`** — `spawn_load_more`'s `F` bound
  is `FnOnce(Option<UtcInstant>, Option<PostId>, …)` and drops to one cursor
  param. The arity lives here, not in `state.rs`
- Modify: **every call site**, across three files the earlier draft of this plan
  missed:
  - `web/src/posts/component.rs:1029` (`list_by_user`), `:1048`, `:1393`
    (`list_drafts`), `:1545` (`list_by_tag`), `:1563`, `:1614`
    (`list_by_user_and_tag`), `:1633`
  - **`web/src/home/component.rs:44`** (`list_local_timeline`)
  - **`web/src/cockpit/component.rs:37`** (`list_home_feed`)
- Modify: `server/tests/web/web_posts.rs` (30 sites) and **all six `*_form`
  helpers** at `:664`, `:699`, `:721`, `:736`, `:752`, `:773`

**Interfaces:**

- Consumes: `PageCursor` (Task 7).
- Produces: all six paginated `#[server]` fns take `cursor: Option<PageCursor>`
  — `list_by_user`, `list_local_timeline`, `list_home_feed`, `list_by_tag`,
  `list_by_user_and_tag`, `list_drafts`.

- [ ] **Step 1: Write the failing tests**

Two contracts: the cursor round-trips end to end, **and** the request is
JSON-shaped. The second is the one that pins the codec change — without it,
nothing verifies the wire. Helper is `list_user_posts_form`
(`server/tests/web/web_posts.rs:699`), not `list_by_user_form`. Dual-backend per
`CONTRIBUTING.md`:

```rust
#[rstest]
#[case(Backend::Sqlite)]
#[case(Backend::Postgres)]
#[tokio::test]
async fn timeline_page_two_uses_the_cursor_the_first_page_returned(#[case] backend: Backend) {
    // ... seed two published posts, request page 1 with limit 1 ...
    let first: TimelinePage = serde_json::from_str(&body).unwrap();
    let cursor = first.next_cursor.expect("page 1 has more, so it carries a cursor");
    // page 2 is requested with the cursor as ONE value, not two
    let (status, body) = list_user_posts_form(&state, &username, Some(cursor), 10, None).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let second: TimelinePage = serde_json::from_str(&body).unwrap();
    assert_eq!(second.posts.len(), 1);
    assert_ne!(second.posts[0].post_id, first.posts[0].post_id);
}

#[rstest]
#[case(Backend::Sqlite)]
#[case(Backend::Postgres)]
#[tokio::test]
async fn paginated_listing_accepts_a_nested_json_cursor(#[case] backend: Backend) {
    // Post the body by hand so the ON-THE-WIRE shape is asserted, not just behaviour.
    let body = serde_json::json!({
        "username": "alice",
        "cursor": { "created_at": "2026-01-01T00:00:00Z", "post_id": 7 },
        "limit": 10
    });
    let (status, out) = post_json(&state, "/posts/list_by_user", &body, None).await;
    assert_eq!(status, StatusCode::OK, "body: {out}");
    // and the OLD flat urlencoded form is no longer accepted
    let (status, _) = post_urlencoded(
        &state, "/posts/list_by_user",
        "username=alice&cursor_created_at=2026-01-01T00:00:00Z&cursor_post_id=7&limit=10",
        None,
    ).await;
    assert_ne!(status, StatusCode::OK, "the flat urlencoded form must no longer decode");
}
```

- [ ] **Step 2: Run them, verify they fail**

Run:
`cargo nextest run -p server 'timeline_page_two_uses_the_cursor|nested_json_cursor'`
Expected: FAIL — `list_user_posts_form` still takes two cursor arguments; the
JSON body is rejected by the urlencoded codec.

- [ ] **Step 3: Switch the six endpoints to `input = Json`**

They use the **default form-urlencoded codec** today, which cannot carry a
nested struct. This is the repo's existing rule, not a new one: `create`
(`api.rs:161`) and `update` (`:307`) are the only two `#[server]` fns in the
`web` crate declaring `input = Json`, and they are exactly the two taking a
struct parameter.

```rust
#[server(endpoint = "/posts/list_by_user", input = Json)]
```

Same for `/posts/list_local_timeline`, `/posts/list_home_feed`,
`/posts/list_by_tag`, `/posts/list_by_user_and_tag`, `/posts/list_drafts`. Then
rewrite all six `*_form` helpers (`web_posts.rs:664`, `:699`, `:721`, `:736`,
`:752`, `:773`) — they hand-build urlencoded bodies (e.g. `:709-710`'s
`parts.push(format!("cursor_created_at={created_at}"))`) and must post JSON
instead.

- [ ] **Step 4: Change the six signatures**

```rust
pub async fn list_by_user(username: Username, cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<TimelinePage>;
pub async fn list_local_timeline(cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<TimelinePage>;
pub async fn list_home_feed(cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<TimelinePage>;
pub async fn list_by_tag(tag: Tag, cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<TimelinePage>;
pub async fn list_by_user_and_tag(username: Username, tag: Tag, cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<TimelinePage>;
pub async fn list_drafts(cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<Vec<DraftSummary>>;  // return type changes in Task 9
```

Each body feeds `parse_post_cursor` from the one value instead of two `Option`s.

- [ ] **Step 5: Delete `into_query` and repoint every caller**

`TimelineCursor::into_query` (`state.rs:50-55`) existed only to split the
newtype back into the flat pair. With no flat pair left it is **deleted**, along
with `cursor_into_query_splits_or_empties` (`:332`) — the region it covered no
longer exists, the same justification as Task 4's `:711`. `state.rs:229` passes
the cursor through.

`spawn_load_more`'s generic bound at **`web/src/timeline/component.rs:26`** —
`F: FnOnce(Option<UtcInstant>, Option<PostId>, …)` — drops to a single cursor
parameter, and its three closures (`posts/component.rs:1048`, `:1563`, `:1633`)
follow.

- [ ] **Step 6: Repoint the five non-paginating call sites**

These pass `None, None` today and are easy to miss because they do not paginate
— but they will not compile:

- `web/src/posts/component.rs:1029` —
  `list_by_user(user_query(username)?, None, …)`
- `web/src/posts/component.rs:1393` —
  `list_drafts(None, Some(PageSize::default()))`
- `web/src/posts/component.rs:1545` — `list_by_tag(tag_query(tag)?, None, …)`
- `web/src/posts/component.rs:1614` —
  `list_by_user_and_tag(username, tag, None, …)`
- `web/src/home/component.rs:44` — `list_local_timeline(None, None, …)`
- `web/src/cockpit/component.rs:37` — `list_home_feed(None, None, …)`

And two **by-name function passes**, which break on the `F` bound rather than at
a call:

- `web/src/home/component.rs:50` — `spawn_load_more(state, list_local_timeline)`
- `web/src/cockpit/component.rs:56` —
  `spawn_load_more(state.timeline, list_home_feed)`

- [ ] **Step 7: Run the suites, verify they pass**

Run: `cargo nextest run -p web -p server` Expected: PASS.

- [ ] **Step 8: Verify AC7, AC7a, AC7b**

```bash
rg 'next_cursor_created_at|next_cursor_post_id|TimelineCursor'   # expect: nothing
rg 'cursor_created_at|cursor_post_id' web/                        # expect: nothing
rg 'into_query'                                                    # expect: nothing
rg -c 'input = Json' web/src/posts/api/listing.rs                 # expect: 5
```

- [ ] **Step 9: Commit**

```bash
cargo xtask check
git add web/src server/tests
git commit -m "refactor(web/posts): paginated server fns take Option<PageCursor> (#569)"
```

---

### Task 9: `UnpublishedPost` in an `UnpublishedPage`

Spec D7. 16 sites / 5 files. Depends on Tasks 3, 7, and 8.

**Files:**

- Modify: `web/src/posts/api.rs:98-113` (the struct), `:440-484`
  (`list_drafts`), **`:457`** (`exact_limit()` → `fetch_limit()`)
- Modify: `web/src/posts/mod.rs:55`; `parse.rs:9`, `:62-87`, `:183-191`, `:205`
- Modify: `web/src/posts/api/listing.rs:404-451`
  (`every_paginated_fetcher_asks_storage_for_the_probing_row`)
- Modify: `web/src/posts/component.rs:32`, `:1452`, `:1477`
- Modify: `server/tests/web/web_posts.rs` (4 sites, incl. `:1024`'s pagination
  test)

**Interfaces:**

- Consumes: `SavedPost` (Task 3), `PageCursor` (Tasks 7–8).
- Produces:
  `pub struct UnpublishedPost { pub post: SavedPost, pub title: Option<PostTitle>, pub summary_label: PostSummary, pub edit_url: RootRelativeUrl }`;
  `pub struct UnpublishedPage { pub posts: Vec<UnpublishedPost>, pub next_cursor: Option<PageCursor>, pub has_more: bool }`;
  `list_drafts(cursor: Option<PageCursor>, limit: Option<PageSize>) -> WebResult<UnpublishedPage>`.

- [ ] **Step 1: Rewrite the pagination test to seed from the page, not a row**

`web_posts.rs:1024` currently feeds `first_entry.created_at` back as the cursor
(`:1087`). With `created_at` off the row, it seeds from the page (AC8a) — the
capability is preserved, not deleted:

```rust
let first_page: UnpublishedPage = serde_json::from_str(&body).unwrap();
assert_eq!(first_page.posts.len(), 1, "body: {body}");
let first_entry = &first_page.posts[0];
let cursor = first_page.next_cursor.expect("page 1 has more, so it carries a cursor");

let (status, body) = list_drafts_form(&state, Some(cursor), 10, Some(&author_cookie)).await;
// NB: list_drafts_form (web_posts.rs:664) currently takes 5 params and builds an
// urlencoded body; Task 8 Step 3 already rewrote it to (state, cursor, limit, cookie)
// posting JSON. If Task 8 is not yet landed, this test will not compile.
assert_eq!(status, StatusCode::OK, "body: {body}");
let second_page: UnpublishedPage = serde_json::from_str(&body).unwrap();
assert_eq!(second_page.posts.len(), 1, "body: {body}");
assert_ne!(first_entry.post.post_id, second_page.posts[0].post.post_id);
```

- [ ] **Step 2: Rewrite `draft_row_display`'s tests for the renamed field**

`parse.rs:183-191`'s builder loses `created_at`/`updated_at`, nests `post`, and
spells `published_at`. The badge behaviour is unchanged — a `Some` still means
scheduled:

```rust
fn unpublished(title: Option<&str>, scheduled: Option<&str>) -> UnpublishedPost {
    UnpublishedPost {
        post: SavedPost {
            post_id: PostId::from(1),
            slug: parse_slug("a-slug"),
            published_at: scheduled.map(parse_utc_instant),
            permalink: parse_root_relative_url("/~alice/2026/01/01/a-slug"),
        },
        title: title.map(|t| PostTitle::from(t.to_string())),
        summary_label: parse_post_summary("fallback label"),
        edit_url: parse_root_relative_url("/posts/1/edit"),
    }
}
```

- [ ] **Step 3: Run them, verify they fail**

Run: `cargo nextest run -p web -p server drafts` Expected: FAIL —
`UnpublishedPost` / `UnpublishedPage` not defined.

- [ ] **Step 4: Implement both types**

Replace `DraftSummary` with the two Interfaces shapes in `web/src/posts/api.rs`.
`published_at` replaces `scheduled_at` and moves into the nested `SavedPost`;
`created_at` and `updated_at` are gone.

- [ ] **Step 5: Switch `list_drafts` to the probing row — without this the page
      is born broken**

`api.rs:457` passes `page_size.exact_limit()`. With no probing row there is
nothing to detect "more" from, so `has_more` would be permanently `false` and
`next_cursor` permanently `None` — an `UnpublishedPage` that can never paginate,
and Step 1's test would fail at `.expect("page 1 has more…")`. Change it to
`page_size.fetch_limit()`. The storage trait already supports it:
`list_drafts_by_user` takes `limit: RowLimit` (`storage/src/posts.rs:634-640`).

- [ ] **Step 6: Extend the probing-row guard to cover `list_drafts`**

`web/src/posts/api/listing.rs:404-451`
(`every_paginated_fetcher_asks_storage_for_the_probing_row`) exists because
`fetch_posts_by_tag` once shipped with `exact_limit()` and load-more silently
died. That guard does not currently reach `list_drafts`. Extend it — or add a
mock-store twin beside it — so the same regression cannot recur on this path.
This is the test that makes Step 5 stick.

- [ ] **Step 7: Rewrite `list_drafts` itself**

It builds `UnpublishedPost { post: SavedPost { .. }, .. }` per row and returns
an `UnpublishedPage`. **`page_from_rows` (`listing.rs:32-52`) is not reusable**
— it returns `TimelinePage` of `RenderedPost` — so this is a second has-more
site derived the same way: over-fetch `page_size + 1`, pop the probe, take the
last kept row's cursor when there was one. Keep the two derivations visibly
parallel so the drift #696 removed does not return.

The comment at `:466-468` explaining that a `Some` here is necessarily future
**moves to `draft_row_display`**, which is now where that fact is used.

- [ ] **Step 8: Update `draft_row_display` and the component**

`parse.rs:75` takes `&UnpublishedPost`; `:80-82` reads `.post.published_at` and
spells the future-ness itself:

```rust
let scheduled_badge = row
    .post
    .published_at
    .map(|when| format!("Scheduled for {when}"));
```

`component.rs:1452` takes `Result<UnpublishedPage, WebError>` and maps over
`.posts`; `:1477`'s `render_draft_row` takes `UnpublishedPost` and reads
`draft.post.post_id`, `draft.post.permalink`.

- [ ] **Step 9: Run the suites, verify they pass**

Run: `cargo nextest run -p web -p server` Expected: PASS.

- [ ] **Step 10: Verify AC8 and AC8a**

```bash
rg 'DraftSummary'                       # expect: nothing
rg 'scheduled_at' web/ common/          # expect: nothing
rg -n 'exact_limit' web/src/posts/      # expect: no paginated fetcher uses it
```

- [ ] **Step 11: Commit**

```bash
cargo xtask check
git add web/src server/tests
git commit -m "refactor(web/posts): DraftSummary -> UnpublishedPost in an UnpublishedPage (#569)"
```

---

### Task 10: ADR — the content-weight axis

Spec D9. Uses **`jaunder-adr`** (numberless draft in `docs/adr/drafts/`;
`cargo xtask adr promote` numbers it at ship).

**Files:**

- Create: `docs/adr/drafts/post-dto-content-weight-axis.md`

**Interfaces:**

- Consumes: the final type names from Tasks 3, 5, 6, 9.
- Produces: the ADR draft AC10 checks for.

- [ ] **Step 1: Write the draft**

It must record, per D9:

- The tier axis — carries authored source / carries rendered form / metadata
  only — and that no name encoded it before #569.
- That viable merges sit **within** a tier, and a cross-tier union ships unread
  payload: a `RenderedPost`/`AuthoredPost` union would put `body` on all 50
  timeline rows (`PageSize::default()` is 50, `common/src/pagination.rs:24-25`).
- The `SavedPost`↔`UnpublishedPost` four-field overlap and **why it is not
  folded**: nothing converts between them, unlike the read DTOs, where
  `component.rs:971` was rebuilding one from the other by hand. Recording this
  is the point — otherwise the overlap gets re-filed as duplication, which is
  exactly how #747 came to be written.

- [ ] **Step 2: Verify the format gate**

Run: `cargo xtask check` Expected: `[ ok ] adr-format`,
`[ ok ] adr-readme-parity`.

- [ ] **Step 3: Commit**

```bash
git add docs/adr/drafts/post-dto-content-weight-axis.md
git commit -m "docs(adr): record the post DTO content-weight axis (#569)"
```

---

### Task 11: Final gate

- [ ] **Step 1: Delete the handoff scaffolding**

`HANDOFF.md` is uncommitted scaffolding, not a deliverable, and
`cargo xtask validate` refuses a dirty tree.

```bash
rm HANDOFF.md
```

It was staged (`A `) at the cycle's start but became untracked partway through,
so `git rm --cached` now fails with "did not match any files" — a plain `rm` is
all it needs. Confirm with `git status --short` that nothing else is left
untracked before the gate, since `validate` refuses a dirty tree.

- [ ] **Step 2: Run every AC grep against the final tree**

Each must now return nothing (they were each observed to match before the
change):

```bash
rg 'pub struct \w*(Result|Response)\b' web/src/posts common/src/seed.rs
rg 'CreateResult|UpdateResult|PublishResult|CreateArgs|UpdateArgs'
rg 'TimelinePostSummary|PostResponse|DraftSummary|DerivedPostMetadata'
rg 'TimelineCursor|next_cursor_created_at|next_cursor_post_id|into_query'
rg 'cursor_created_at|cursor_post_id' web/
rg 'scheduled_at' web/ common/
rg '"args"' server/
rg 'args:' end2end/tests/posts.ts end2end/tests/feeds.spec.ts
rg 'unwrap_or\(.*created_at\)' web/src/posts/component.rs
```

- [ ] **Step 3: Full validation including the e2e matrix** (AC15)

Run: `cargo xtask validate` Expected: PASS, including the four-combo e2e matrix.
**This is the only thing that verifies the ten wire changes** — treat a failure
here as a real defect, not flake, and root-cause before re-running.

- [ ] **Step 4: Review the test diff against AC13** — a human/agent read, not a
      gate

AC13's substantive clause is "**No assertion is weakened or deleted beyond what
the dropped fields force**," which no command can check. Read the whole test
diff:

```bash
git diff wt-base-issue-569..HEAD -- server/tests/ web/src/ common/src/ end2end/
```

Every deleted or relaxed assertion must trace to a specific authorized deletion.
The plan authorizes exactly four:

| Deleted                                           | Authorized by   |
| ------------------------------------------------- | --------------- |
| `api.rs:711 update_post_args_rejects_…`           | spec D3, AC14   |
| `state.rs:332 cursor_into_query_splits_or_…`      | spec D6, Task 8 |
| `state.rs:312 cursor_from_page_needs_both_…`      | spec D6, Task 7 |
| assertions on `created_at`/`summary`/`updated_at` | spec D1, D7     |

Anything else is attrition — restore it.

- [ ] **Step 5: Hand off to `jaunder-ship`**

---

## Self-review

**Spec coverage.** D1→T3 · D2→T3 · D3→T4 · D4→T4 · D5→T5,T6 · D6→T7,T8 · D7→T9 ·
D8→T2 · D9→T10 · D10→T1 · D11→T1 (issues) + no code. AC1→T6 · AC2,AC3→T3 ·
AC4,AC5→T4 · AC6,AC6a,AC6b→T6 · AC6c→T5 · AC7→T7 · AC7a,AC7b→T8 · AC8,AC8a→T9 ·
AC9,AC9a→T2 · AC10→T10 · AC11→T1 · AC12,AC12a→T11 · **AC13→T11 Step 4** (a diff
review, since its "no assertion weakened" clause is not machine-checkable) ·
AC14→T3,T4,T7,T8 · AC15→T11. No gaps.

**Task 8 is the one to delegate carefully.** It is the largest (six signatures,
a codec change, six test helpers, eight caller sites across five files, one
generic bound) and the only one whose failure mode is a silent wire break rather
than a compile error. Its Step 1 JSON-shape test is what makes that visible.

**Type consistency.** `SavedPost` (T3) is consumed by name in T4 and T9;
`RenderedPost` (T5) by T6 and T7; `PageCursor` (T7) by T8 and T9; `PostInputs`
(T4) is terminal. `list_drafts`'s return type is deliberately shown unchanged in
T8's signature block and changed in T9 — flagged inline so it doesn't read as a
contradiction.

**Ordering.** T2 is independent. T3 precedes T4 and T9. T5 precedes T6 and T7.
T7 precedes T8, which precedes T9. T10 needs the final names, so it follows T9.
T11 is last.
