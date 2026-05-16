# Tags UI Design

## Scope

Wire the existing tag storage (M5 step 2) through to the user interface:
tag input during post create/edit, tag chips displayed on post cards,
site-wide and per-user tag-listing pages, and client-side autocomplete
suggestions from the existing tag corpus. No new storage features beyond
two read helpers (`list_tags` for autocomplete, `get_tags_for_posts` for
batch loading on timeline pages).

## Architecture Decisions

### Wire-format refactor (precondition)

`create_post` and `update_post` switch to JSON encoding
(`#[server(input = Json)]`). This lands as a self-contained refactor
**before** any tag work begins — typed `Vec<String>` is the cleanest
representation for the tag list, and the refactor sets a future-proof
precedent for any other complex param that lands on these server fns
later. The integration tests in `server/tests/web_posts.rs` (≈50 call
sites) are rewritten to send JSON bodies via a new `post_json` helper;
the existing `post_form` helper stays for other endpoints.

### Tag input affordance

A new `<TagInput>` component renders the current tag set as chips with
inline `×` remove buttons, plus a text field beneath. Pressing `Enter`
or `Tab` turns the text into a new chip; `Backspace` on an empty input
removes the last chip; clicking `×` removes a specific chip. Comma is
not a chip-creation key.
Autocomplete suggestions for existing tags appear in a dropdown beneath
the input, fetched from `list_tags(prefix)` with 150 ms debounce. Keyboard
navigation (`ArrowUp`/`ArrowDown` + `Enter`) selects a suggestion.

Underlying state: `RwSignal<Vec<TagSummary>>` where
`TagSummary = { slug: String, display: String }`. The component emits one
`<input type="hidden" name="tags" value=display>` per chip; with the
form's JSON encoding, those collect into a `Vec<String>` typed parameter
server-side.

### Tag display on post cards

`PostDisplay`'s currently-empty `<footer class="j-post-foot">` carries a
new `<TagList context=…>` component that renders one `<a class="j-tag">`
per tag. Visible everywhere a post is shown (timelines, permalinks, draft
previews, drafts list).

Each chip links to the site-wide listing at `/tags/:slug`. When
`context` is `TagContext::ForUser(username)`, each chip also carries a
small `· here` link pointing at `/~:username/tags/:slug` so the per-user
listing remains one click away from any user-rooted page. On site-wide
contexts (local timeline, site tag pages), the chip stands alone.

### Tag listing pages

Two new leptos routes:

| Path | Component | Backing query |
|---|---|---|
| `/tags/:tag` | `SiteTagPage` | `PostStorage::list_posts_by_tag` |
| `/~:username/tags/:tag` | `UserTagPage` | `PostStorage::list_user_posts_by_tag` |

Each uses `Topbar` (`title = "#<tag>"`, `sub` = "Posts on this instance"
or "Posts by ~<username>"), then the standard `j-scroll`+`j-page` chrome
with a cursor-paginated `PostCard` list. Reuses the timeline pagination
pattern from M5 step 9.

### Server functions and storage extensions

New on `PostStorage`:

```rust
async fn list_tags(
    &self,
    prefix: Option<&str>,
    limit: u32,
) -> sqlx::Result<Vec<TagRecord>>;

async fn get_tags_for_posts(
    &self,
    post_ids: &[i64],
) -> sqlx::Result<HashMap<i64, Vec<PostTag>>>;
```

New web server fn:

```rust
#[server(endpoint = "/list_tags", input = Json)]
pub async fn list_tags(
    prefix: Option<String>,
    limit: Option<u32>,
) -> WebResult<Vec<TagSummary>>;
```

`create_post` and `update_post` (now JSON-encoded) gain a
`tags: Vec<String>` typed parameter. Each token is validated through
`Tag::from_str`; invalid tokens fail the whole request with the offending
token in the error message. The update path computes the set-difference
against `get_tags_for_post(post_id)` and issues `tag_post` / `untag_post`
calls accordingly, all within the same transactional context as the post
mutation.

A hard cap of 25 tags per post is enforced server-side; the UI shows a
flash on overflow.

### `TimelinePostSummary` carries tags

Add `tags: Vec<TagSummary>` to `TimelinePostSummary`. All timeline
listing server fns (`list_user_posts`, `list_local_timeline`,
`list_home_feed`, the new tag-listing fns, plus `get_post`) batch-load
tags for the resulting page via `get_tags_for_posts` to avoid N+1.

### Crate layout

| Module | Change |
|---|---|
| `common/src/storage/posts.rs` | Two new trait methods. |
| `server/src/storage/{sqlite,postgres}.rs` | Concrete implementations. |
| `web/src/posts.rs` | `TagSummary` struct, `tags: Vec<TagSummary>` on `TimelinePostSummary`; `tags: Vec<String>` param on `create_post` and `update_post`. |
| `web/src/tags.rs` (new) | `list_tags` server fn. |
| `web/src/pages/ui.rs` | New `TagInput` and `TagList` components, plus a `TagContext` enum. |
| `web/src/pages/posts.rs` | Wire `TagInput` into the create- and edit-post forms; pass `TagContext` to `PostDisplay` based on route. |
| `web/src/pages/tags.rs` (new) | `SiteTagPage` and `UserTagPage`. |
| `web/src/pages/mod.rs` | Two new routes. |

No new CSS classes that aren't already needed — chips reuse the existing
`.j-chip` styling pattern (see `Chip` in `ui.rs`) with the addition of a
`.j-tag` class for tag-specific styling (color hint, hover state) and a
`.j-tag-chip` class for the editable chip used inside `TagInput`.

## Steps

After each step verify:

1. `cargo build` succeeds.
2. `cargo nextest run` passes.
3. `cargo clippy -- -D warnings` is clean.
4. `scripts/check-coverage` succeeds.
5. `nix flake check` passes.

Wait for user review, commit when the user says that the review is
complete and successful.

### Step 1: JSON-encode `create_post` and `update_post` (refactor only)

This step ships **no tag functionality** — it only changes the wire
format of the two existing server fns so that subsequent steps can add a
typed `Vec<String>` tags parameter cleanly.

1. [ ] Add `#[server(input = Json)]` to `create_post` and `update_post`.
2. [ ] Add a `post_json(state, path, body, cookie)` helper in
   `server/tests/web_posts.rs` mirroring the existing `post_form`
   helper but emitting `application/json`.
3. [ ] Rewrite every `/api/create_post` and `/api/update_post` call
   site in `server/tests/web_posts.rs` to use `post_json` with
   `serde_json::json!({...})` bodies.
4. [ ] Verify `scripts/verify` passes end-to-end (the change is
   transparent to ActionForm in the leptos pages, but the e2e
   suite exercises both server fns so this proves the switch
   is correct).
5. [ ] Commit as a standalone refactor before any tag work begins.

### Step 2: Tag storage extensions

1. [ ] Add `list_tags(prefix, limit)` to `PostStorage`.
2. [ ] Add `get_tags_for_posts(post_ids)` to `PostStorage`.
3. [ ] Implement both on `SqlitePostStorage`.
4. [ ] Implement both on `PostgresPostStorage`.
5. [ ] Integration tests on both backends: prefix matching with various
   cases (case-insensitive on slug), empty prefix returning alphabetical
   tags, limit clamping, batch lookup with mixed post_ids (some with
   tags, some without, some non-existent).

### Step 3: `list_tags` server fn and `TagSummary` type

1. [ ] Add `TagSummary { slug: String, display: String }` to
   `web/src/posts.rs` (or a new `web/src/tags.rs`).
2. [ ] Add `list_tags(prefix, limit) -> Vec<TagSummary>` server fn with
   `#[server(input = Json)]`.
3. [ ] Integration tests covering prefix filtering, limit defaults,
   `tags` table empty case.

### Step 4: `TimelinePostSummary` carries tags

1. [ ] Add `tags: Vec<TagSummary>` to `TimelinePostSummary`.
2. [ ] Update `list_user_posts`, `list_local_timeline`,
   `list_home_feed`, and `get_post` to batch-load tags via
   `get_tags_for_posts`. Single round-trip per page.
3. [ ] Integration tests confirming tags appear on each surface.

### Step 5: Tags param on `create_post` and `update_post`

1. [ ] Add `tags: Vec<String>` to both server fns.
2. [ ] Helper `parse_and_validate_tags(tokens) -> Result<Vec<String>, _>`:
   trims whitespace, rejects empty tokens, validates each via
   `Tag::from_str`, enforces the 25-tag cap.
3. [ ] In `create_post`: after the post is created, call `tag_post`
   for each validated display token.
4. [ ] In `update_post`: load existing tags via `get_tags_for_post`,
   compute the set-difference, call `tag_post` for newly-added,
   `untag_post` for removed.
5. [ ] Integration tests: create with tags, update adds + removes,
   editing only display casing keeps the slug stable, exceeds the cap.

### Step 6: `TagInput` component

1. [ ] Add `TagInput` to `web/src/pages/ui.rs` (or a new file). Props:
   `tags: RwSignal<Vec<TagSummary>>`, `name: &'static str` (default
   `"tags"`).
2. [ ] Chip rendering with `×` remove buttons.
3. [ ] Text input with keydown handling for `Enter` / `Tab` (create
   chip), `Backspace` on empty (remove last). Comma is **not** a
   chip-creation key.
4. [ ] Client-side validation against `[a-z0-9][a-z0-9-]*` after
   lowercasing (mirrors `Tag::from_str`); invalid input shows a brief
   inline error.
5. [ ] Autocomplete dropdown: 150 ms-debounced `list_tags(prefix)` fetch,
   keyboard navigation, click-to-add.
6. [ ] Emit one `<input type="hidden" name="tags" value=display>` per
   chip.
7. [ ] Unit tests for the pure-function helpers (token splitting, case
   normalization).

### Step 7: Wire `TagInput` into create- and edit-post forms

1. [ ] `PostCreateForm` (compact branch): place `TagInput` in the
   composer bar's tag slot, beneath the textarea but above the action
   buttons.
2. [ ] `PostCreateForm` (full branch): place `TagInput` in the
   right-side aside, between the slug and the format toggle.
3. [ ] `EditPostPage`: place `TagInput` in the matching position,
   pre-populated with the post's current tags loaded via
   `get_post_preview`.
4. [ ] E2E (Playwright): create a post with three tags via the UI,
   submit, refetch the post, assert tags applied.

### Step 8: Tag display in `PostDisplay` footer

1. [ ] Add `TagContext` enum (`SiteWide`, `ForUser(username)`) and
   thread it through `PostCard` and `PostDisplay` as a new prop.
2. [ ] Add `<TagList post_tags=… context=…>` component that renders one
   `<a class="j-tag" href="/tags/:slug">#:display</a>` per tag, plus a
   `· here` link pointing at `/~user/tags/:slug` when the context is
   `ForUser`.
3. [ ] Place `<TagList>` at the start of `<footer class="j-post-foot">`
   inside `PostDisplay`.
4. [ ] Pass `TagContext::ForUser(username)` from `UserTimelinePage`
   and `PostPage` (when the post's `permalink` is user-rooted, which
   it always is); pass `TagContext::SiteWide` from `HomePage` Local
   mode and from the site-wide tag listing.
5. [ ] Update CSS: `.j-tag` (clickable chip in display contexts) and
   `.j-tag-chip` (editable chip in `TagInput`).

### Step 9: Tag listing pages and routes

1. [ ] Add `SiteTagPage` at `/tags/:tag` to `web/src/pages/tags.rs`.
   Uses `Topbar` (`title = format!("#{tag}")`, `sub = "Posts on this
   instance"`), `j-scroll` + `j-page` chrome, cursor-paginated
   `PostCard` list driven by a new `list_posts_by_tag` server fn that
   wraps the existing storage call.
2. [ ] Add `UserTagPage` at `/~:username/tags/:tag`. Same chrome, sub =
   `format!("Posts by ~{username}")`. New server fn wraps
   `list_user_posts_by_tag`.
3. [ ] Wire both routes into `web/src/pages/mod.rs`.
4. [ ] Integration tests for both new server fns.
5. [ ] E2E: visit `/tags/:tag` from a tag chip on a post, confirm only
   matching posts; use the `· here` link to navigate to
   `/~user/tags/:tag`, confirm scoping.

### Step 10: End-to-end exercise

1. [ ] E2E: register a user, create a post with three tags via the UI,
   land on the permalink, click a tag chip, verify navigation to
   `/tags/:tag`.
2. [ ] E2E: from `/tags/:tag`, navigate back to the post via the
   permalink, edit it to remove one tag and add a different one,
   verify the chip set updates in the footer and that the removed tag's
   listing page no longer contains the post while the added tag's
   listing does.
3. [ ] E2E autocomplete: in the create form, type a partial tag, verify
   the autocomplete dropdown appears with suggestions from existing
   tags, click one, verify a chip is added.

## Out of Scope

- Tag cloud / trending tags / tag discovery (roadmap: "Not yet
  scheduled").
- Renaming or merging tags site-wide (operator tooling).
- Per-user tag preferences (mute, follow).
- Bulk edits across many posts at once.
- Hashtag autodetection in post body text (must be entered in the
  TagInput).
- Reverting the JSON encoding switch — once Step 1 lands, the two
  server fns stay on JSON.
