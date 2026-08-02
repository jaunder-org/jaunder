# Spec — post DTOs named for what they are (#569)

- Issue: jaunder-org/jaunder#569
- Milestone: Web: canonical Leptos CSR convergence
- Deferred from: #323 (posts vertical convergence)
- Supersedes the withdrawn 2026-07-31 draft of this file, whose D6/D7 were
  written from #747's issue body rather than from the tree, and which never
  addressed #569's two owner comments.

## Problem

The posts read/result DTOs and the `#[server]` arg wrappers are named for their
**role in the plumbing** (`*Result`, `*Response`, `args`) rather than for **what
they are**. Beneath the naming sit four structural defects, each verified
against the tree:

1. **Two result pairs are the same shape wearing two names**, and two of their
   fields ride the wire unread.
2. **The two read DTOs are near-duplicates that the code hand-converts between**
   — `component.rs:971-986` rebuilds a `TimelinePostSummary` from a
   `PostResponse` field by field, **fabricating**
   `published_at: fetched.published_at.unwrap_or(created_at)`.
3. **A field is named for the query that produced it, not for what it holds** —
   `DraftSummary.scheduled_at` is `draft.published_at.map(UtcInstant::from)`
   (`api.rs:469`), renamed because in _that listing_ a `Some` is necessarily
   future.
4. **A struct exists to carry a field nothing reads** — `DerivedPostMetadata`.

The family's real discriminator is **content weight**: carries authored source
(`body`), carries rendered form (`RenderedHtml`), or metadata only. No name
encodes it. The consequence is concrete — unrelated types read as redundant, and
#747 was filed proposing a merge that would have been a wire regression.

### The invariant the owner asked about does not exist

#569's 2026-07-24 comment names "the always-published `published_at: UtcInstant`
invariant on timeline rows" as the one real design question for merging the read
DTOs. That invariant is **already fabricated**:

- `web/src/posts/server.rs:13` upholds it on the _listing_ path
  (`let published_at = post.published_at?;` — the `?` bails on a draft).
- `web/src/posts/component.rs:979-981` **breaks it**: `PostPage` rebuilds a
  summary from a fetched detail with
  `published_at: fetched.published_at.unwrap_or(fetched.created_at)`, laundering
  a draft's `created_at` into a `published_at`.
- `render.rs:97` and `:116` are the same `PostView` construction twice,
  differing only in that same `unwrap_or(created_at)` fallback.

So the merged type does not need to _preserve_ an invariant — it gets to
**delete a lie**. `Option<UtcInstant>` is what the code already means.

## Decisions

Renamed types stay in the module they live in today, except the pagination
cursor (D6). All keep their existing derive set —
`#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]` for the wire
DTOs (`PartialEq` is load-bearing at `web/src/posts/api.rs:644` and `:665`,
which assert serde round-trip equality) — **with one exception, spelled out in
D6**: the cursor gains `Serialize`/`Deserialize` because it has none today.

### D1 — One `SavedPost` for every post-mutating operation

```rust
/// The saved post's identity, publication state, and where to find it.
pub struct SavedPost {
    pub post_id: PostId,
    pub slug: Slug,
    pub published_at: Option<UtcInstant>,
    pub permalink: RootRelativeUrl,
}
```

`CreateResult`, `UpdateResult`, and `PublishResult` are all replaced by it.
`created_at` and `summary` are **dropped** — no consumer reads either off a
result value (swept across `web/src/posts/**`, `server/tests/`, `end2end/`,
`elisp/`).

`published_at` is load-bearing and stays: `update` can unpublish (`CreateArgs`/
`UpdateArgs` carry `publish: bool` + `publish_at`, fed through
`publish.into_inputs()` at `post_service.rs:332`), and two consumers
discriminate on it — `component.rs:762,769` picks "Post published!" vs "Draft
saved!", and `page_state.rs:131` gates the publish redirect. Every use is
`.is_some()`/`.is_none()`; nothing reads the instant.

**Rejected:** modelling the state as `enum { Draft, Published(UtcInstant) }`. It
is _exactly isomorphic_ to `Option<UtcInstant>` — same inhabitants, same
information — so it buys ceremony and a wire change across ~42 deserialize
sites, and nothing else.

### D2 — All four mutations return `SavedPost`

```rust
pub async fn create(post: PostInputs) -> WebResult<SavedPost>;
pub async fn update(post_id: PostId, post: PostInputs) -> WebResult<SavedPost>;
pub async fn publish(post_id: PostId) -> WebResult<SavedPost>;
pub async fn unpublish(post_id: PostId) -> WebResult<SavedPost>;  // was WebResult<()>
```

`PublishResult` is deleted rather than renamed. Its distinguishing feature — a
non-optional `published_at` — is read by **no consumer**: both live readers
touch only `.permalink` (`component.rs:284`, `:1422`), and one test reads
`.post_id` (`web_posts.rs:1272`). A type distinguished by an unread field is not
a refinement worth keeping.

`web/src/posts/api.rs:651-671 publish_result_permalink_wire_is_root_relative` is
built entirely around `PublishResult`, including its non-optional `published_at`
at `:659`. It is **retargeted to `SavedPost`**, not deleted — it covers the
root-relative-URL wire guard, which survives. Its name changes to match.

**`unpublish` gains a return value.** Not uniformity for its own sake:
`permalink()` is `published_at.unwrap_or(created_at)`-based
(`storage/src/posts.rs:76`), so **unpublishing moves the permalink** exactly as
publishing does. `publish` returns the moved URL for that reason
(`component.rs:277-284`); `unpublish` not returning it is an accident. It needs
no storage change — `unpublish` already holds the pre-update record, so binding
it `mut` (`api.rs:587`) suffices:

```rust
existing.published_at = None;
Ok(SavedPost { post_id, slug: existing.slug.clone(),
               published_at: None, permalink: existing.permalink() })
```

**Caller behaviour is unchanged.** `component.rs:952-954` keeps navigating to
`/drafts`, and `component.rs:266-269`'s closure changes only its binding
(`move |()|` → `move |_|`). The returned value is deliberately **unread for
now**: this decision puts the moved permalink on the wire so a caller _can_ use
it, but changing where unpublish navigates is behaviour, and #569 is a
naming/modelling issue. AC12's `posts.spec.ts:587` therefore still passes
unmodified.

### D3 — `CreateArgs` → `PostInputs`; `UpdateArgs` deleted

`UpdateArgs` is `CreateArgs` plus a leading `post_id` and nothing else
(`api.rs:128-137` vs `:142-152`). One struct for the authored inputs, with
`post_id` as its own parameter. No arity constraint blocks it — `list_by_user`
already takes four (`api/listing.rs:115-120`).

**Forced consequence.**
`api.rs:691 create_post_args_rejects_unknown_format_token` and
`:711 update_post_args_rejects_unknown_format_token` are the same assertion
against the two structs. With `UpdateArgs` gone, `:711` becomes byte-identical
to `:691` — **delete `:711`**, rename `:691` to
`post_inputs_rejects_unknown_format_token`. Deleting a covered region is
deliberate here: the region it covered no longer exists.

### D4 — Wire key `args` → `post`

In leptos `#[server]`, JSON body keys are the **parameter names**, not the type
names — the macro builds the request struct's fields from the fn's inputs
verbatim, with derived serde and no `rename`
(`server_fn_macro-0.8.10/src/lib.rs:395-419`; Cargo.lock pins 0.8.10). So:

- `create` — `{"args": {…}}` → `{"post": {…}}`
- `update` — `{"args": {…}}` → `{"post_id": …, "post": {…}}`

**Every raw-JSON producer must change — four files, and the two languages spell
the key differently:**

| File                                    | Sites | Note                                                  |
| --------------------------------------- | ----- | ----------------------------------------------------- |
| `server/tests/web/web_posts.rs`         | 12    | `"args": {` (quoted)                                  |
| `server/tests/feed/feed_events_hook.rs` | 6     | `:28, 69, 104, 147, 214, 281`                         |
| `end2end/tests/posts.ts`                | 1     | `createPostViaApi()` `:33` — imported by 5 spec files |
| `end2end/tests/feeds.spec.ts`           | 1     | `:272` — nests `post_id` **inside** `args`            |

`posts.ts:20`'s doc comment ("nested under an `args` wrapper (#299)") updates
too.

### D5 — The read-DTO collapse: `RenderedPost` and `AuthoredPost`

Per #569's 2026-07-24 comment. `PostResponse` is a strict superset of
`TimelinePostSummary` — 11 identical fields, `published_at` differing only in
optionality, `PostResponse` adding `body`+`format`, and `TimelinePostSummary`
having no unique field.

```rust
/// A post in rendered form: everything needed to paint it, without its source.
pub struct RenderedPost {          // was TimelinePostSummary
    pub post_id: PostId,
    pub username: Username,
    pub title: Option<PostTitle>,
    pub summary: Option<PostSummary>,
    pub slug: Slug,
    #[serde(deserialize_with = "deserialize_rendered_html")]
    pub rendered_html: RenderedHtml,
    pub created_at: UtcInstant,
    pub published_at: Option<UtcInstant>,   // was UtcInstant — the fabrication dies
    pub permalink: Option<RootRelativeUrl>,
    pub is_author: bool,
    pub is_draft: bool,
    pub tags: Vec<TagSummary>,
}

/// A post with the source it was authored from.
pub struct AuthoredPost {          // was PostResponse
    pub post: RenderedPost,
    pub body: PostBody,
    pub format: PostFormat,
}
```

**Nested, not `#[serde(flatten)]`.** Flatten would keep the permalink seed wire
flat, but it buffers through `Content` and re-drives `deserialize_with`, and
`RenderedPost` carries `deserialize_with = "deserialize_rendered_html"`. The
seed is server→client within one deploy, so its shape is free to change; taking
a serde sharp edge to preserve it is not worth it.

**The names encode the content-weight axis** (D9), in idiomatic adjective-noun
order matching `RenderedHtml` and `CreatePostInput` already in this tree.
Rejected: `PostDetails` and `EditablePost` — the first is a vague comparative
("detailed compared to what?"), the second names a purpose the type does not
primarily serve (the dominant consumer is the _anonymous_ permalink,
`PageSeed::Permalink` served with `is_author: false` by
`server/src/projector/mod.rs:192`).

**`rendered_post` stays fallible.** `web/src/posts/server.rs:12-13` returns
`Option<TimelinePostSummary>` today _only_ because `published_at` was
non-optional (`let published_at = post.published_at?;`), and `api/listing.rs:44`
`filter_map`s on that. Once `RenderedPost.published_at` is `Option`, nothing
type-level forces the bail — so the builder must **keep** its
`-> Option<RenderedPost>` return and its `post.published_at?` guard. Removing it
would leak drafts into public listings. Pinned by AC6c.

Other consequences:

- `component.rs:971-986`'s 16-line, 12-field hand rebuild becomes
  `fetched.post.clone()`.
- `render.rs:89 permalink_article` and `:107 render_posts` converge on one
  `PostView` construction: with `published_at` now `Option` on both, `:116`
  becomes the same
  `format_post_time(post.published_at.unwrap_or(post.created_at))` as `:97`.
- Builders rename: `server::timeline_post_summary` → `rendered_post`,
  `server::post_response` → `authored_post`.
- `PageSeed::Permalink(AuthoredPost)` — a tuple variant of a plain
  externally-tagged enum, so its JSON key stays `"Permalink"`.

**Blast radius (verified):** `TimelinePostSummary` 23 sites / 7 files, including
`web/src/timeline/{state.rs,mod.rs}`. `PostResponse` 19 sites / 7 files. The two
builders 19 code sites / 5 files, including `web/src/posts/api/listing.rs` and
`server/src/projector/mod.rs`.

### D6 — `PageCursor` on the wire

Per #569's 2026-07-21 comment. `TimelineCursor` moves from
`web/src/timeline/state.rs` into `common/src/seed.rs` and is **renamed
`PageCursor`**: D7 gives the drafts listing a page type that carries the same
cursor, so "Timeline" would be a role name for a type serving two listings — the
same defect this issue exists to fix. It does not collide with
`storage::PostCursor` (`storage/src/posts.rs:188`), the storage-layer keyset
cursor it is built from.

```rust
/// The `(created_at, post_id)` keyset pair a paginated listing hands back.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageCursor {
    pub created_at: UtcInstant,
    pub post_id: PostId,
}

pub struct TimelinePage {
    pub posts: Vec<RenderedPost>,
    pub next_cursor: Option<PageCursor>,   // was two flat Option fields
    pub has_more: bool,
}
```

**This is the one type whose derive set changes.**
`web/src/timeline/state.rs:25` is `#[derive(Clone, Copy, Debug, PartialEq, Eq)]`
— **no serde at all**. Going on the wire requires adding
`Serialize`/`Deserialize`; `Copy` must be **kept**, since
`TimelineCursor::into_query` uses it by value.

`TimelineCursor::from_page` — whose whole job is reconciling a half-cursor the
server never emits — is **deleted**; the type makes the half-cursor
unrepresentable. `listing.rs:41-51` stops splitting `next_cursor` into two
mapped fields.

**The cursor is a `PageCursor` on the request side too, at all six paginated
endpoints.** ADR-0063 §4
(`docs/adr/0063-domain-value-newtype-convention.md:300`, accepted) requires
parsing into the newtype at the **outermost** boundary — "the `#[server]`
argument" — and carrying the newtype on every surface we define. Today five
timeline fns take a flat pair (`api/listing.rs:115`, `:139`, `:161`, `:262`,
`:286`) and `list_drafts` a sixth (`api.rs:441-444`). All six become:

```rust
#[server(endpoint = "/posts/list_by_user", input = Json)]
pub async fn list_by_user(username: Username, cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
pub async fn list_local_timeline(cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
pub async fn list_home_feed(cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
pub async fn list_by_tag(tag: Tag, cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
pub async fn list_by_user_and_tag(username: Username, tag: Tag, cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
pub async fn list_drafts(cursor: Option<PageCursor>, limit: Option<PageSize>) -> …;
```

**All six gain `input = Json`.** They use the default form-urlencoded codec
today, which cannot carry a nested struct. This is not a new convention:
`create` and `update` (`api.rs:161`, `:307`) are the **only** two `#[server]`
fns in the `web` crate that declare `input = Json`, and they are exactly the two
that take a struct parameter. Every other server fn takes flat scalars and
newtypes. So a struct argument already implies `Json` here; these six join that
rule rather than bending it. Consequence: the content type changes on six
endpoints, and all six `*_form` test helpers
(`server/tests/web/web_posts.rs:664`, `:699`, `:721`, `:736`, `:752`, `:773`)
move from hand-built urlencoded bodies to JSON.

`TimelineCursor::into_query` (`timeline/state.rs:50`) existed only to split the
newtype back into the flat pair. With no flat pair left it is **deleted**, along
with its test `cursor_into_query_splits_or_empties` (`:332`) — the region it
covered no longer exists, the same justification as D3's `:711`.
`state.rs:229`'s call site passes the cursor straight through, and the three
`spawn_load_more` closures (`component.rs:1048`, `:1563`, `:1633`) take one
cursor argument instead of two.

Leaving the five flat would put two conventions for one concept in the spec that
exists to remove exactly that.

**Blast radius (verified):** 62 sites / 9 files. Beyond
`web/src/timeline/state.rs` (28) and `server/tests/web/web_posts.rs` (12), the
flat pair is constructed in `server/src/projector/mod.rs` (2 pairs),
`web/src/posts/render.rs` (4 pairs), `web/src/posts/page_state.rs` (1), and
**`web/src/cockpit/state.rs`** (1) — a vertical the withdrawn draft declared out
of scope. `storage/src/posts.rs:485-497`'s `next_cursor` is a differently-named
local `PostCursor` and is **not** in scope.

The request-side change adds 60 further sites across 3 files —
`server/tests/web/web_posts.rs` (30), `web/src/posts/api/listing.rs` (27),
`web/src/posts/api.rs` (3) — counted as `\bcursor_created_at|\bcursor_post_id`,
i.e. excluding the `next_cursor_*` response fields already counted above.

### D7 — `DraftSummary` → `UnpublishedPost`, in an `UnpublishedPage`

`list_drafts_by_user` returns "drafts (`published_at` NULL) and scheduled posts
(`published_at` in the future)" (`api.rs:466-468`) — exactly the author's
**not-yet-public** posts. "Draft" is wrong for half that set.

```rust
pub struct UnpublishedPost {
    pub post: SavedPost,
    pub title: Option<PostTitle>,
    pub summary_label: PostSummary,
    pub edit_url: RootRelativeUrl,
}

pub struct UnpublishedPage {
    pub posts: Vec<UnpublishedPost>,
    pub next_cursor: Option<PageCursor>,
    pub has_more: bool,
}

pub async fn list_drafts(
    cursor: Option<PageCursor>,
    limit: Option<PageSize>,
) -> WebResult<UnpublishedPage>;   // was WebResult<Vec<DraftSummary>>
```

Four changes beyond the rename:

- **`scheduled_at` → `published_at`.** It _is_ `published_at` (`api.rs:469`),
  renamed for a property of the query that produced it, with the guarantee
  living only in a comment. `parse.rs:80` then spells the future-ness itself
  rather than reading it off a field name.
- **The listing gains a page type.** `list_drafts` is cursor-paginated on the
  wire (`api.rs:441-444`), and
  `web_posts.rs:1024 list_drafts_returns_current_user_drafts_with_cursor_pagination`
  exercises it, feeding `first_entry.created_at` back as the page-2 cursor
  (`:1087`). It returns a bare `Vec`, so a client must reconstruct the cursor
  from two row fields — exactly what D6 removes for timelines. The page carries
  the cursor instead.
- **`created_at` and `updated_at` leave the row.** `updated_at` was already
  unread. `created_at` was read _only_ as a cursor component, and the cursor now
  lives on the page — so with `UnpublishedPage` in place, neither field has a
  consumer. Every surviving row field is the label, the badge, or an action
  target. `web_posts.rs:1024` is rewritten to seed page 2 from
  `first_page.next_cursor` rather than from a row.
- **It nests `SavedPost`.** With `published_at` corrected, the reduced row is an
  exact superset of `SavedPost` — all four fields, same names, same types, from
  the same producers (`PostRecord::permalink()`, `PostRecord.published_at`).

**Rejected: one flat 7-field `SavedPost`.** It would put `title`,
`summary_label`, and `edit_url` on every create/update/publish/unpublish
response, where nothing reads them — removing two dead fields (D1) to add three.
Worse, `summary_label` would require calling `record.fallback_summary_label()`
on the write path, scanning the full `PostBody` to produce a value nobody reads.

**Blast radius (verified):** 16 sites / 5 files.

### D8 — Delete `DerivedPostMetadata`; `derive_post_metadata` → `derive_post_title`

`DerivedPostMetadata.summary_label` is read **only by `common/src/render.rs`'s
own tests** (`:1119`, `:1149`, `:1162`). Neither production consumer touches it
— `post_service.rs:310-334` and `:476-507` use `.title` and `.slug_seed` only.
The struct exists to carry a dead field, so it is deleted and the function is
**renamed**:

```rust
pub fn derive_post_title(
    explicit_title: Option<&str>,
    body: &str,
    format: &PostFormat,
) -> Option<(Option<PostTitle>, String)>   // (title, slug seed); None = empty post
```

Both call sites destructure into shadowed bindings, each keeping its own error:

```rust
// storage/src/post_service.rs:311  (update)
let (title, slug_seed) =
    derive_post_title(title, &body, &format).ok_or(PerformUpdateError::EmptyPost)?;
// storage/src/post_service.rs:477  (creation)
let (title, slug_seed) =
    derive_post_title(title, &body, &format).ok_or(PerformCreationError::EmptyPost)?;
```

This deletes real dead computation, not just a field: `fallback_label(body)` is
called three times (`render.rs:372`, `:387`, `:395`) and **two of those results
feed only `summary_label`**. Only `:395` survives, where it is both the
empty-post gate and the slug seed. All three `PostSummary::truncated` calls in
`render.rs` go.

**Three tests in `common/src/render.rs` assert on `summary_label`** (`:1119`,
`:1149`, `:1162`) and must be updated, not silently dropped.
`:1146 derive_metadata_for_html_extracts_no_title_but_keeps_fallback_label`
exists _for_ the label: it is renamed to state what it now covers (HTML bodies
extract no title) rather than left under a name describing a deleted field.
Pinned by AC9a.

The `(Some(t), t)` / `(None, label)` correlation is left unencoded — no consumer
relies on it, and an enum would cost a match at both call sites for an invariant
nothing reads.

### D9 — Record the content-weight axis in an ADR

Via `jaunder-adr` (numberless draft in `docs/adr/drafts/`, numbered at ship by
`cargo xtask adr promote`). It records: the tier axis (authored source /
rendered form / metadata only); that viable merges sit **within** a tier; that a
cross-tier union ships unread payload (a union of `RenderedPost` and
`AuthoredPost` would ship `body` on all 50 timeline rows — `PageSize::default()`
is 50, `common/src/pagination.rs:24-25`); and the `SavedPost`↔`UnpublishedPost`
overlap, so it is not re-litigated as duplication.

### D10 — Correct #569 and #754

- **#569's body** still proposes `PostDetails` and "consistent with `*Summary`",
  both of which this spec inverts. Update it rather than let the spec silently
  override the issue it cites.
- **#754's premise is false.** It proposes unifying two implementations of the
  summary-label fallback chain; D8 establishes one of them is dead, so the
  remedy is deletion, not unification. Rewrite it to the question that genuinely
  survives — whether the label should be _stored_ rather than recomputed per
  drafts row (its own step 2, which needs a migration and its own evidence) —
  and note the corrected premise.

### D11 — Out of scope

Not deferred silently: the plan's **first task** files the two new issues below
via `jaunder-issues`, together with D10's #754 rewrite, so all three can be
picked up concurrently rather than blocked behind this cycle.

**Filed as follow-up issues:**

- **#782** — _web/common: drop \*Result / \*Response role-suffixes in the media
  and auth DTOs._ `common/src/media.rs:845 UploadResponse`,
  `web/src/auth/api.rs:33 LoginResponse`,
  `web/src/media/api.rs:62 DeleteResult`. Verified to survive this change, which
  is precisely why AC1 is scoped to the posts family.
- **#783** — _web(posts): unpublish navigates to /drafts instead of the
  permalink it now returns._ Created by D2: unpublishing moves the permalink and
  the correct URL is now in the response, but `component.rs:952-954` still
  hardcodes `/drafts` and the value goes unread. Needs a UX decision (stay on
  the post, or go to the drafts list), which is why it is not settled here.
  Blocked by #569.

**Out of scope, already tracked:**

- The storage-layer merges (`RenderedPostContent` → `CreatePostInput`, carrying
  `PublishUpdate` to the binding point) — **#747**. Different crates; either
  order.
- Persisting `summary_label` — **#754**, after D10's rewrite.

**Out of scope, no issue warranted:**

- **`delete`** (`api.rs:548`) keeps its `WebResult<()>` return. It removes a
  post rather than saving one, so there is no `SavedPost` to hand back; D2
  covers the four save-shaped mutations only. The asymmetry is explained, not
  accidental.
- `storage::helpers::PostRow` and `storage::PostRecord` — the DB row and record
  are not wire DTOs, so the naming convention here does not reach them.

## Acceptance criteria

Each check below **must be run against the unchanged tree first and observed to
fail**; a grep-based criterion that passes before the change is a false green.

**AC1.**
`rg 'pub struct \w*(Result|Response)\b' web/src/posts common/src/seed.rs`
returns nothing.

**AC2.** `SavedPost` exists with exactly the four D1 fields and the stated
derives. `CreateResult`, `UpdateResult`, and `PublishResult` do not exist
anywhere.

**AC3.** `create`, `update`, `publish`, and `unpublish` all have return type
`WebResult<SavedPost>`. (`delete` keeps `WebResult<()>` per D11.)

**AC4.** `PostInputs` exists; `CreateArgs` and `UpdateArgs` do not. The
signatures are exactly `create(post: PostInputs)` and
`update(post_id: PostId, post: PostInputs)` — **the parameter names are the wire
contract**, so `post` is required, not incidental.

**AC5.** No `args` envelope survives, checked per language:

- `rg '"args"' server/` returns nothing (Rust, quoted key).
- `rg 'args:' end2end/tests/posts.ts end2end/tests/feeds.spec.ts` returns
  nothing (TS, bare key). Scoped to these two files: `playwright.config.ts:36`
  and `seed.ts:20,29,30` use `args` for unrelated purposes and must not be
  touched.

**AC6.** `RenderedPost` and `AuthoredPost` exist with the D5 shapes;
`TimelinePostSummary` and `PostResponse` do not exist anywhere. `AuthoredPost`
nests `RenderedPost` as a field named `post`, with no `#[serde(flatten)]`. The
builders are `rendered_post` and `authored_post`.

**AC6a.** `RenderedPost.published_at` is `Option<UtcInstant>`, and `PostPage` no
longer hand-builds a post row from a fetched detail:
`rg 'RenderedPost\s*\{|TimelinePostSummary\s*\{' web/src/posts/component.rs`
returns nothing — the field-by-field rebuild is replaced by a clone of the
nested value.

**Corrected 2026-08-01.** This criterion originally read
`rg 'unwrap_or\(.*created_at\)' web/src/posts/component.rs` returns nothing.
That was wrong in **both** directions, and each error is instructive:

- **False positive after the change.** `PostDisplay` legitimately computes its
  time label as `format_post_time(post.published_at.unwrap_or(post.created_at))`
  — the identical expression `render.rs`'s two converged builders use, and it
  **must stay** or the projector and CSR paints stop coinciding (ADR-0044). The
  original grep cannot tell that expression from the fabricated field it was
  aimed at.
- **False green before the change.** The obvious narrowing,
  `rg 'published_at:\s*.*unwrap_or'`, matches nothing on the pre-change tree
  either — rustfmt had wrapped the fabricated assignment across three lines, so
  a single-line pattern never saw it. It would have "passed" without the change
  ever being made.

The replacement targets the structural fact the criterion is actually about —
that no post row is constructed by hand in that file — and was verified to match
`TimelinePostSummary {` on the pre-change tree and nothing after.

**AC6b.** The `PostCard` **component** still exists under that exact name and is
still re-exported from `web/src/posts/mod.rs` — the rename must not have taken
its name.

**AC6c.** `rendered_post` still returns `Option<RenderedPost>` and still bails
on a draft via `post.published_at?`; `api/listing.rs` still `filter_map`s it. A
test asserts that a draft `PostRecord` yields `None`.

**AC7.** `common::seed::PageCursor` exists, derives
`Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq`, and both
`TimelinePage` and `UnpublishedPage` carry `next_cursor: Option<PageCursor>`.
`rg 'next_cursor_created_at|next_cursor_post_id|TimelineCursor'` returns nothing
anywhere in the repo. `TimelineCursor::from_page` does not exist.

**AC7b.** All six paginated `#[server]` fns declare `input = Json`, and
`server/tests/web/web_posts.rs`'s six `*_form` helpers post JSON bodies rather
than urlencoded ones. A test pins the on-the-wire request shape — a nested
`cursor` object, not two flat keys — so the codec change is verified rather than
assumed.

**AC7a.** All six paginated `#[server]` fns take `cursor: Option<PageCursor>` —
`list_by_user`, `list_local_timeline`, `list_home_feed`, `list_by_tag`,
`list_by_user_and_tag`, `list_drafts`. `rg 'cursor_created_at|cursor_post_id'`
returns nothing in `web/`. Neither `into_query` nor
`cursor_into_query_splits_or_empties` exists.

**AC8.** `UnpublishedPost` exists with the D7 shape, nesting `SavedPost` as
`post`; `UnpublishedPage` exists; `list_drafts` returns
`WebResult<UnpublishedPage>`. `DraftSummary` does not exist anywhere, and
`rg 'scheduled_at' web/ common/` returns nothing.

**AC8a.** `web_posts.rs:1024`'s cursor-pagination test still asserts two
distinct pages, seeding page 2 from `first_page.next_cursor` rather than from a
row field. The capability is preserved, not deleted.

**AC9.** `DerivedPostMetadata` and `derive_post_metadata` do not exist in
**source or live docs** — `rg 'DerivedPostMetadata|derive_post_metadata'`
returns hits only under `docs/archive/` and in superseded spec/plan documents,
which are frozen records of what was true when they were written and must
**not** be rewritten. `docs/adr/` is _not_ frozen in this respect: an ADR that
points at a live code seam is corrected, since a dangling symbol misleads the
next reader (ADR-0024 `:36` is the one such case). `derive_post_title` returns
`Option<(Option<PostTitle>, String)>`. `fallback_label` has exactly **one call
site** in `common/src/render.rs`, and `PostSummary::truncated` has none in that
file.

**AC9a.** The three `common/src/render.rs` tests that asserted on
`summary_label` (`:1119`, `:1149`, `:1162`) still exist and assert on what the
function now returns; `:1146` is renamed so no test name references a deleted
field. No test in `common/src/render.rs` is deleted.

**AC10.** An ADR draft exists in `docs/adr/drafts/` recording D9's
content-weight axis, the within-tier merge rule, and the
`SavedPost`↔`UnpublishedPost` overlap. (Numbering is a ship-time step, not a
state this criterion checks.)

**AC11.** #569's body no longer proposes `PostDetails`, and #754 has been
rewritten per D10.

**AC12.** No behaviour change. These existing e2e tests pass unmodified
**except** `posts.ts`/`feeds.spec.ts`'s raw-JSON envelope (AC5):

- `posts.spec.ts:26` create a post through the UI
- `posts.spec.ts:42` create a post with a summary
- `posts.spec.ts:69` over-long summary gates submit
- `posts.spec.ts:87` clearing a summary on edit persists as empty
- `posts.spec.ts:125` save a draft
- `posts.spec.ts:140` published post renders at permalink
- `posts.spec.ts:168` edit a draft post
- `posts.spec.ts:323` draft lifecycle: create, view, edit, and publish
- `posts.spec.ts:587` unpublishing from a permalink navigates to /drafts without
  a reload

**AC12a.** The projector↔CSR byte-identical paint is intact (ADR-0041
§"byte-identical per URL", ADR-0044). `PostView<'a>` (`render.rs:127`) is the
extracted render core these changes flow through; after D5's convergence both
call sites build it identically, and the emitted HTML is unchanged —
`authed-cls.spec.ts` and the `layout-shift.ts` helpers stay green with no
tolerance change.

**AC13.** The Rust integration tests pass with only mechanical edits — type
renames, the `"args"` → `"post"` key, `post_id` hoisted out of the envelope,
field accesses moved under `.post`, drafts rows read off `.posts`, and removal
of assertions on dropped fields. Scope is **both**
`server/tests/web/web_posts.rs` and `server/tests/feed/feed_events_hook.rs`. No
assertion is weakened or deleted beyond what the dropped fields force.

**AC14.** `api.rs:711` is deleted and `:691` renamed per D3; `api.rs:651`'s
round-trip test is retargeted to `SavedPost` per D2. No other `#[test]` in
`web/src/posts/` is removed.

**AC15.** `cargo xtask validate` green, including the four-combo e2e matrix.

## Known wire changes

All intended; all covered by AC12/AC13.

| Endpoint / payload                     | Change                                                                                                                                                                                                                            |
| -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create` response                      | loses `created_at` and `summary`                                                                                                                                                                                                  |
| `update` response                      | loses `summary` (it never carried `created_at`)                                                                                                                                                                                   |
| `create`, `update` requests            | envelope key `args` → `post`; `update` hoists `post_id` to a sibling key                                                                                                                                                          |
| `publish` response                     | unchanged in JSON (`Option<UtcInstant>` serializes identically when `Some`)                                                                                                                                                       |
| `unpublish` response                   | `null` → a `SavedPost` object                                                                                                                                                                                                     |
| permalink seed (`PageSeed::Permalink`) | flat → `{"post": {…}, "body": …, "format": …}`                                                                                                                                                                                    |
| `TimelinePage`                         | `next_cursor_created_at` + `next_cursor_post_id` → `next_cursor: {created_at, post_id} \| null`                                                                                                                                   |
| timeline rows                          | `published_at` becomes nullable; values unchanged (always `Some` there)                                                                                                                                                           |
| `list_drafts` response                 | array → `{"posts": […], "next_cursor": …, "has_more": …}`                                                                                                                                                                         |
| all six paginated requests             | `cursor_created_at` + `cursor_post_id` → one `cursor` key, **and the codec changes from form-urlencoded to JSON** (`list_by_user`, `list_local_timeline`, `list_home_feed`, `list_by_tag`, `list_by_user_and_tag`, `list_drafts`) |
| drafts rows                            | lose `created_at`/`updated_at`; `scheduled_at` → `published_at`; identity fields nest under `post`                                                                                                                                |

No rendered output changes; no user-visible behaviour changes.

## Notes on cited decision records

ADR-0041 and ADR-0044 are both **accepted** and both state the
byte-identical-per-URL requirement AC12a relies on. **ADR-0065 is `proposed`,
and its decision is scoped to `#[server]` _args_, not response fields** — D6
cites it as a directional analogy for newtyping the cursor, not as a governing
rule; the operative authority for parsing at the outermost boundary is ADR-0063
§4.
