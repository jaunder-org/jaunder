# Post UI Consistency Design

Date: 2026-04-26

## Overview

Six related inconsistencies in the post authoring and viewing UI, addressed as a unified design:

1. No way to edit a published post from the timeline
2. Titleless posts have no permalink link in the timeline
3. The permalink page offers Delete but not Edit for the author
4. The draft list mixes link elements and form buttons inconsistently
5. The Create and Edit forms diverge visually from each other and from InlineComposer
6. The edit form breaks the title out of the post body, obscuring what the user originally typed

The design also adds Unpublish (published → draft) as a natural complement to Publish.

---

## Section 1 — Body storage and title caching

### Principle

Post body is stored verbatim — exactly as the user typed it. No stripping, no transformation before storage or rendering. This is consistent with the DESIGN.md principle that the original content is the source of truth.

### What changes

- `body` is stored verbatim. A markdown post beginning with `# My Title` stores that heading. An org post beginning with `#+TITLE: My Title` or `* My Title` stores it as typed.
- `rendered_html` is generated from the verbatim body. For markdown, `# Title` renders to `<h1>` naturally. For org, `* Title` also renders as a heading element, but `#+TITLE:` is a metadata keyword that orgize skips — no `<h1>` in the output for that form.
- The `title` column is retained as a **derived cache**, populated on every create/update by `derive_post_metadata`. It is never accepted from the UI as a separate input.
- `DerivedPostMetadata.body` field is removed. `derive_post_metadata` no longer mutates the body — callers use the original body for storage and rendering. Only `title`, `slug_seed`, and `summary_label` are returned.
- The separate title input fields are removed from `CreatePostPage` and `EditPostPage`. Users set the title by writing `# Heading` (markdown) or `#+TITLE: Title` / `* Title` (org) in the body.

### Org-mode title extraction algorithm

The updated `extract_org_title` applies the following algorithm in order:

1. Scan from the top, skipping blank lines and KV lines (`#+key: value` pattern).
2. If a `#+TITLE: value` KV line is encountered, use its value as the title.
3. Otherwise, if the first non-blank, non-KV line is a top-level heading (`* Heading`), use its text as the title. This is analogous to a `# Title` at the top of a markdown file — both `#+key: value` lines and `* Heading` can serve as the title, and neither renders as body content in the same way running text does.
4. Otherwise, no title — derive `slug_seed` and `summary_label` from the body text as normal.

The body is never modified. The algorithm only determines the cached `title` value.

### Rendering the title on the permalink page

Because `#+TITLE:` does not produce a heading in `rendered_html` but `* Title` and `# Title` do, a simple format check is insufficient to decide whether to inject a template `<h1>`. Instead, `render_post_article` inspects `rendered_html` directly: if it opens with an `<h1>` element, no template heading is added; if it does not but `title` is `Some`, a template `<h1>` is injected from the cached value.

**Note**: this is a mild encapsulation violation — `render_post_article` is making decisions based on assumed knowledge of what the HTML renderer will produce. It is the most practical approach given that the rendering and display layers are otherwise decoupled, and is accepted as a known trade-off until a cleaner boundary presents itself.

### What does not change

The `title` field in `DraftSummary`, `TimelinePostSummary`, and `PostResponse` continues to be read from the cache for efficiency. List views do not need to fetch or parse the full body to display a title.

---

## Section 2 — Shared `ComposerFields` component

### New component: `ComposerFields`

Location: `web/src/pages/ui.rs`

```rust
#[component]
pub fn ComposerFields(
    body: RwSignal<String>,
    format: RwSignal<String>,
    #[prop(default = "markdown".to_string())] default_format: String,
    #[prop(optional)] show_slug: bool,
    #[prop(optional)] slug: Option<RwSignal<String>>,
    #[prop(optional)] slug_editable: bool,
) -> impl IntoView
```

`default_format` is a forward-compatible hook for user-configurable format preference. Callers pass `"markdown"` for now; when user config is added, they pass the user's stored preference instead.

### Layout

`ComposerFields` renders the full two-panel layout:

- **Left panel**: body textarea (bound to `body` signal), hidden `<input name="format">` driven by the `format` signal
- **Right panel (aside)**: format segmented control, optional slug input (when `show_slug`), and a slot for the caller's action buttons and any extras (char count, etc.)

Each caller wraps `ComposerFields` in its own `ActionForm` and provides:
- Avatar + username header above the grid
- Action buttons inside the aside (below the options)
- Result/flash messages below the form

### Format segmented control

Replaces both the current `<select>` dropdowns (CreatePostPage, EditPostPage) and the toggle buttons (InlineComposer) with a consistent segmented control:

- Two adjacent buttons with no gap between them
- Shared border; inner corners square, outer corners rounded
- Selected state: filled background (e.g. `var(--ink)` background, `var(--bg)` text)
- Unselected state: outline only
- Drives the hidden `<input name="format">` via the `format` signal

### Updated callers

| Caller | Action | Signals owned | Extras provided by caller |
|---|---|---|---|
| `InlineComposer` | `CreatePost` | `body`, `format` | Avatar + username header; Publish / Save draft buttons; flash message |
| `CreatePostPage` | `CreatePost` | `body`, `format`, `slug` | Avatar + username header; char count; Draft / Publish buttons; result message |
| `EditPostPage` | `UpdatePost` | `body` (pre-filled), `format` (pre-filled), `slug` (pre-filled, locked if published) | Save / Publish buttons; result message |

### Slug field

- Present in `CreatePostPage` and `EditPostPage`; absent from `InlineComposer`
- Editable only when the post is a draft (`slug_editable = !is_published`)
- Published posts: slug rendered as read-only text (not an input)

---

## Section 3 — Author actions and permalink links

### Consistent layout across all post views

All post views use a layout with a dedicated right-hand **actions column**. Content is on the left; actions are pinned to the right, stacked vertically, top-anchored.

### Timeline cards (`PostCard`, `render_timeline_post_row`)

**Layout**: three columns — avatar | content (username, title, body) | actions

**Actions column**:
- Timestamp as a permalink link with a small arrow icon (`→`) — shown to all viewers
- Edit, Unpublish, Delete buttons — shown only when `is_author`
- Unpublish and Delete show a confirmation dialog before acting

**Data change**: add `is_author: bool` to `TimelinePostSummary`, computed server-side by comparing the authenticated user's `user_id` to the post's `user_id`. Unauthenticated requests always produce `is_author: false`.

**Title display**: titled posts display the title as a non-linked heading in the content column. The title is no longer the permalink link — the timestamp serves that role for all posts.

### Draft list (`render_draft_row`)

**Layout**: two columns — content | actions (no avatar; always the author's own drafts)

**Content column**: title/summary label + slug

**Actions column** (stacked vertically):
- Preview (anchor styled as `j-btn`)
- Edit (anchor styled as `j-btn`)
- Permalink (anchor styled as `j-btn`)
- Publish (action button)
- Delete (action button, with confirmation)

### Permalink page (`PostPage`)

**Layout**: two columns — article content | actions

**Actions column** (stacked vertically, `is_author` only):
- Edit (links to `/posts/{post_id}/edit`)
- Unpublish (with confirmation)
- Delete (with confirmation)

### New server function: `unpublish_post`

```rust
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: i64) -> Result<(), ServerFnError>
```

- Requires authentication; verifies the authenticated user owns the post
- Sets `published_at = NULL` on the post record
- Returns an error if the post is not found or not owned by the caller
- No AP/AT federation side effects at this stage (future work)

---

## Testing

### Unit tests

- `derive_post_metadata` no longer returns a `body` field — update all existing tests
- Add: `derive_post_metadata` with markdown `# Title` body — verify `title` extracted, body unchanged
- Add: `derive_post_metadata` with org `#+TITLE:` body — verify `title` extracted, body unchanged
- Add: `derive_post_metadata` with org `* Title` heading (preceded only by KV lines) — verify `title` extracted, body unchanged
- Add: `derive_post_metadata` with org `* Title` heading preceded by body text — verify no title extracted
- Add: `extract_org_title` with `#+TITLE:` present alongside a `* Heading` — verify `#+TITLE:` takes precedence
- Add: `unpublish_post` storage layer — verifies `published_at` set to `NULL`

### Integration tests

- `POST /api/unpublish_post` — requires auth, ownership check, returns 200 on success, 401/403/404 on failure

### End-to-end tests

- Timeline card for author shows Edit, Unpublish, Delete; non-author sees only timestamp link
- Clicking timestamp navigates to the post permalink
- Draft list actions column: all six actions present and functional
- Permalink page shows Edit, Unpublish, Delete for author; none for visitor
- Unpublish moves post to draft; it disappears from public timeline
- Edit from timeline navigates to edit page with verbatim body (heading intact in textarea)
- Format segmented control: selecting Org switches format; saved post reflects the choice
- Slug field locked on published post edit; editable on draft edit
