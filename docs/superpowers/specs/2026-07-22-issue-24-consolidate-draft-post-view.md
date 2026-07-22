# Spec — #24: consolidate `DraftPreviewPage` and `PostPage` into a single view

**Status:** awaiting approval. **Milestone:** #11 (Web: canonical Leptos CSR
convergence). **Subsumes:** #23 (PostCard shows "Unpublish" on draft permalinks)
— closed on merge. **Vertical:** `posts` (already converged to the ADR-0070
four-file layout in #323); all UI lives in `web/src/posts/component.rs`.

## Problem

Two routed pages render the same post through the same inner component
(`PostDisplay`), differing only in their action column:

- **`DraftPreviewPage`** — route `/draft/:post_id/preview`
  (`component.rs:1512-1613`). Loads by `post_id` via the `get_post_preview`
  server fn, renders `PostDisplay` with a hand-built `[Publish, Delete]` column,
  a `"Draft preview – visible only to you"` banner, and a post-publish success
  message linking to the new permalink. Drafts only.
- **`PostPage`** — route `/~user/YYYY/MM/DD/slug` (`component.rs:1140-1245`).
  Loads by permalink via `get_post` (which already falls back to
  `find_draft_by_permalink_for_user`, so it renders **both** published posts and
  the author's own drafts), renders `PostCard`, and computes a
  `"Draft - visible only to you"` banner from `fetched.is_draft`.

The `DraftsPage` listing links each draft to **both** a "Preview"
(`DraftPreviewPage`) and a "Permalink" (`PostPage`), surfacing arbitrary chrome
and action differences for one draft. The same "preview" URL is also emitted as
a flash link after creating or editing a draft (`CreatePostResult.preview_url`,
`UpdatePostResult.preview_url`, and the inline-composer flash).

Because `PostPage`→`PostCard` renders a fixed `[Edit, Unpublish, Delete]` column
with **no draft branch** (`component.rs:227-257`), viewing a draft at its
permalink shows **"Unpublish"** on something already unpublished — the #23 bug.
`PostResponse.is_draft` is available at `PostPage` but is dropped when it builds
the `TimelinePostSummary` that feeds `PostCard`.

`PostPage` is already the strictly-more-capable view. Consolidation is therefore
mostly deletion plus one real fix: teach `PostCard`'s action column about draft
state, then delete `DraftPreviewPage` and every "preview" URL.

## Decisions (interview-resolved)

1. **`PostPage` (the permalink route) is the single canonical view.** It already
   renders drafts for the author. `DraftPreviewPage` is deleted.

2. **The `/draft/:post_id/preview` route is removed entirely** — not kept as a
   redirect or an id-load shim. Draft previews are author-only with no external
   inbound links; any stale bookmark 404s. The route entry
   (`web/src/pages/mod.rs:149-156`) and `DraftPreviewPage`
   (`component.rs:1512-1613`) are deleted.

3. **`PostCard`'s action column becomes draft-aware, driven by post state.**
   When the post is a draft it renders `[Edit, Publish, Delete]` (Publish
   dispatches `PublishPost`, guarded by a `confirm(...)`); when published it
   renders `[Edit, Unpublish, Delete]` (unchanged). `Edit` and `Delete` are
   identical in both arms. This *is* the #23 fix. The action column is an
   overlaid reactive `children` sibling, computed client-side from the post
   state — it is **never part of the SSR/projector render string** (which shares
   `render_post_inner`/`render_post_content` and has no author-action slot), so
   the HTML render path is unchanged.

4. **`is_draft` is threaded through `TimelinePostSummary` as a `bool` field**,
   not passed as a separate `PostCard` prop. The card is self-describing, which
   structurally prevents a future caller from re-introducing #23.
   `TimelinePostSummary` is a serialized wire type
   (`api/listing.rs:37-52`), so every construction site sets the new field:
   - `server.rs:27-39` `timeline_post_summary` → `false` (only ever built from a
     published post; it early-returns `None` unless `published_at` is `Some`).
   - `component.rs:1211-1225` `PostPage` → `fetched.is_draft`.
   - `render.rs:284-298` `sample_summary()` test helper → `false`.
   - `api.rs:644-656` round-trip test literal → set + asserted.
   - (`DraftPreviewPage`'s builder at `component.rs:1539-1553` is deleted.)

5. **On successful publish, `PostCard` navigates to the new canonical
   permalink.** Unlike the existing unpublish path (a hard-coded `/drafts`
   callback baked by `PostPage`), the publish destination is *dynamic* — it comes
   from `PublishPostResult.permalink` (a non-optional `String`, always populated
   server-side by `updated.permalink()`). So `PostCard` owns its own
   `publish_action` and a navigate effect that reads the result's permalink on
   success; it is not a pre-baked callback. This is required for correctness: a
   draft's permalink is `created_at`-based, and publishing can move it to a
   `published_at`-based URL, so staying on the current URL could land on a
   now-stale address.

6. **Every former "preview" URL is replaced by the canonical permalink.** The
   permalink view *is* the preview now. Concretely:
   - `DraftSummary` drops `preview_url`; `DraftsPage`
     (`render_draft_row`, `component.rs:2014-2067`) drops its "Preview" link and
     keeps only the existing "Permalink" link. The `DraftSummary` fixture in the
     `draft_row_display` test (`parse.rs:153`) drops `preview_url` accordingly
     (the pure `DraftRowDisplay` struct itself has no such field).
   - **`CreatePostResult`/`UpdatePostResult` already carry `permalink:
     Option<String>` (`api.rs:68`, `api.rs:79`), but it is populated only for
     published posts (`published_at.is_some().then(|| record.permalink())`,
     `api.rs:233`/`api.rs:409`) — i.e. `None` for a draft, which is exactly why
     the flash consumers fall back to `preview_url` today.** This population must
     change so a draft also gets a permalink (always `Some(record.permalink())`,
     the `created_at`-based URL). Then `preview_url` is dropped from both result
     types and the flash consumers link to `permalink` instead.
   - **The CreatePostPage draft flash currently renders *two* links** — a
     `preview-link` ("Preview draft") and a published-only `permalink-link`
     ("View permalink") (`component.rs:1081-1094`). With `permalink` now always
     populated, these collapse to a **single** permalink link. The
     inline-composer flash (`component.rs:764-767`) and `EditPostPage` flash
     (`component.rs:1924`) likewise switch their one link from `preview_url` to
     `permalink`.

7. **`get_post_preview` is retained — only its `DraftPreviewPage` usage is
   removed.** The server fn has a second caller: `EditPostPage`
   (`component.rs:1665-1670`) loads the post-to-edit by `post_id`, and its route
   `/posts/:post_id/edit` carries only a `PostId` — it cannot use `get_post`
   (which requires `username/year/month/day/slug`). So the endpoint, its
   integration test (`get_post_preview_shows_draft_to_author_only`) and helper
   (`get_post_preview_form`), and the `xtask` `pascal_case` sample all stay put.
   The draft-visibility guarantee at the *permalink* route (author sees; stranger
   and anon denied) is independently held by `get_post`'s draft path
   (`find_draft_by_permalink_for_user`, author-only), already covered by
   `get_post_returns_draft_to_author_only`.

## Acceptance criteria

Observable, so ship-time conformance can tell delivered from not:

- **AC1 (route gone).** There is no `/draft/:post_id/preview` route; requesting
  such a URL does not render `DraftPreviewPage` (the component no longer exists).
- **AC2 (draft actions).** Visiting a draft at its permalink as the author shows
  action buttons `[Edit, Publish, Delete]` — a **Publish** button, **not**
  Unpublish. (Directly resolves #23.)
- **AC3 (published actions unchanged).** Visiting a published post at its
  permalink as the author shows `[Edit, Unpublish, Delete]`, exactly as before.
- **AC4 (publish navigation).** Clicking Publish on a draft permalink (and
  confirming) publishes the post and navigates the browser to the returned
  canonical published permalink; the destination renders the now-published post.
- **AC5 (drafts listing).** Each row in `DraftsPage` links to exactly one view of
  the draft — its permalink. No "Preview" link and no `/draft/:id/preview` href
  appears.
- **AC6 (flashes).** After creating or editing a draft, the resulting flash
  message's link opens the draft's canonical permalink view; no
  `/draft/:id/preview` URL is emitted anywhere in the app.
- **AC7 (draft-flag round-trip).** `TimelinePostSummary` carries `is_draft`; it
  serializes and deserializes intact (extends the existing round-trip test), is
  `true` when `PostPage` renders a draft, and `false` for timeline/listing rows.
- **AC8 (visibility preserved).** At a draft's permalink: the author sees the
  draft; a different signed-in user and an anonymous visitor are denied (not
  shown the draft). Covered by `get_post` tests
  (`get_post_returns_draft_to_author_only`, extended to the stranger + anon
  cases if not already). The retained `get_post_preview` keeps its own
  visibility test.
- **AC9 (gate green).** `cargo xtask validate --no-e2e` passes (static, clippy,
  coverage); the CI e2e matrix passes with the updated Playwright specs. No new
  `cov:ignore`/`crap:allow` markers introduced by this change.

## Test impact (in-scope; enumerated so the plan can size it)

**Playwright — `end2end/tests/posts.spec.ts`** — rewire every preview-anchored
assertion to the permalink:

- `L570` "inline composer: draft flash is a link to the draft preview URL" —
  retitled/rewired to assert the flash links to the permalink.
- `L226-265` "draft lifecycle: create, view, edit, and publish" — navigate to the
  draft via permalink; publish and assert landing on the published permalink
  (AC4).
- `L81-94`, `L127-145`, `L160-189`, `L672-686` — replace `preview-link` /
  `/draft/(\d+)/preview` extraction and the "Preview draft" flash-label assertion
  with the permalink equivalent.

**Rust integration — `server/tests/web/web_posts.rs`:**

- `get_post_preview_shows_draft_to_author_only` (`L670-731`) and the
  `get_post_preview_form` helper (`L88-94`) **stay** (endpoint retained for
  `EditPostPage`). Ensure `get_post_returns_draft_to_author_only` (`L558`) covers
  the stranger + anon denial cases (extend if it only asserts the author) —
  satisfies AC8.
- `create_post_persists_rendered_published_post` (`L136`) and
  `create_post_accepts_slug_override_and_saves_draft` (`L321`) — replace
  `created.preview_url` assertions with `created.permalink` (now `Some` for
  drafts).

**Unit — `web/src/posts/`:** update the `TimelinePostSummary` round-trip test
(`api.rs:644-656`) for `is_draft` (AC7); update `render.rs` `sample_summary`
(`L284-298`); drop `preview_url` from the `DraftSummary` fixture in the
`draft_row_display` test (`parse.rs:153`).

**xtask:** no change (`get_post_preview` retained, so the `pascal_case` sample
stays valid).

> Local heavy e2e is reaped in this environment; gate locally with
> `cargo xtask validate --no-e2e` and let CI's `{backend}×{browser}` matrix run
> the Playwright suite (see project memory).

## Out of scope

- No change to `get_post`'s resolution logic, `PostDisplay`, the SSR projector,
  or the server-side HTML render path (the draft action branch is client-only).
- No change to the timeline/listing surface beyond the mechanical `is_draft:
  false` at its one construction site.
- No new draft/preview capability; this is a pure consolidation + the #23 fix.

## Decision record

No new ADR: this is an application of the existing `posts` convergence
(ADR-0070, #323) and a bug fix, introducing no cross-cutting architectural
decision a future reader would reverse-engineer. If the plan's investigation
surfaces one, record it then.
