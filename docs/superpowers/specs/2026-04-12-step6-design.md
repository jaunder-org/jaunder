# Milestone 5 Step 6 – Permalink Page Design

## 1. Goal & Scope
- Expose `/~:username/:year/:month/:day/:slug` so published posts render minimal HTML (title, author/date, rendered body).
- Drafts remain private to their author; others see a plain text "Post not found" message.
- This step only covers reading a single post; timelines, draft listings, edit links, etc., ship in later steps.

## 2. Backend / Server Function
- Reuse the `get_post` server function in `web/src/posts.rs` for public permalinks.
- Inputs: `username`, `year`, `month`, `day`, `slug` (strings/numbers from route params).
- Flow:
  1. Parse `username` → `Username`, `slug` → `Slug`, and validate the date via `NaiveDate::from_ymd_opt`. No auth requirement here—published posts are public.
  2. Call `AppState.posts.get_post_by_permalink(&username, year, month, day, &slug)` which now filters by `published_at` (not `created_at`). Drafts won’t match because they lack a publish timestamp.
  3. If storage returns `None` or the row has `deleted_at`, return `ServerFnError("Post not found")`.
  4. Return `PostResponse` with `post_id`, `username`, `title`, `slug`, raw `body`, `format`, `rendered_html`, `created_at`, and `published_at` (all RFC3339 strings where applicable).
- Draft preview server fn: add `get_post_preview(post_id: i64)` which requires auth, fetches the post by ID, ensures `post.user_id == auth.user_id`, and returns the same payload without checking `published_at`. This powers the author-only preview route.

## 3. Routing & UI
- Public permalink route (`/~:username/:year/:month/:day/:slug`) still lives in `web/src/pages/mod.rs`, but it now renders only published posts.
- Draft preview route: add `/draft/:post_id/preview` wired to the new `get_post_preview`. It shows the same `<article>` markup with a small "Draft preview" badge and requires auth (server fn already enforces ownership).
- `PostPage` continues using `use_params_map` to parse params, strip the leading `~`, and validate date parts before calling `get_post`.
- Rendering for public posts stays minimal: `<article>` with title, metadata linking to `/~username/` and showing `published_at`, the rendered HTML body, plus a "Home" link to avoid dead ends.
- After publishing, the create-post success UI shows:
  - Slug text (`<p data-test="slug-value" data-slug=...>`)
  - A "Draft preview" link pointing to `/draft/:post_id/preview` (always available to the author)
  - The canonical permalink link (only when `published_at` is set)
- Drafts no longer hit the public route, so the old inline draft badge there is removed.

## 4. Testing Strategy
- **Server tests (`server/tests/web_posts.rs`)**
  - Published post happy path: create post via helper, call server fn endpoint, assert rendered html/title match.
  - Draft gating: same user (authenticated) succeeds, different user/anonymous receives 404 message.
  - Soft-delete exclusion: ensure `deleted_at` posts return 404.
- **E2E (`end2end/tests/posts.spec.ts`)**
  - Extend the existing create-post flow: after publishing, navigate to the permalink and assert the rendered HTML shows.
  - Add a second test (when multi-user fixture available) verifying that a draft permalink is visible to the author but not when logged out; until we can create multiple accounts in one test, document that part as pending.
- Continue running the full verification suite (`cargo build`, `cargo nextest run`, `cargo clippy -- -D warnings`, `scripts/check-coverage`, `nix flake check`) after implementing.

## 5. Documentation & Milestone Tracking
- Update `docs/milestones/M5.md` Step 6 checkboxes once server, UI, and tests land.
- No schema/docs changes beyond referencing this spec; permalink URLs match the architecture doc already.
- Later steps (draft list, timelines) will reuse the same `PostResponse`, so keep it minimal yet complete.
