# Post UI Consistency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make post authoring/viewing UI consistent: verbatim body storage, shared composer component, author action columns, and timestamp-as-permalink on all timeline views.

**Architecture:** Three sequential sections: (1) data model — verbatim storage + org title extraction; (2) shared `ComposerFields` Leptos component with segmented format control; (3) author action columns on all post views plus `unpublish_post`.

**Tech Stack:** Rust, Leptos (SSR via cargo-leptos), SQLite + PostgreSQL (sqlx), TypeScript/Playwright (e2e tests).

---

## File Map

| File | Change |
|---|---|
| `common/src/render.rs` | Update `extract_org_title`, remove `body` from `DerivedPostMetadata` |
| `common/src/storage.rs` | Add `unpublish_post` to `PostStorage` trait |
| `server/src/storage/sqlite.rs` | Implement `unpublish_post` for SQLite |
| `server/src/storage/postgres.rs` | Implement `unpublish_post` for PostgreSQL |
| `web/src/posts.rs` | Remove `title` param from `create_post`/`update_post`; add `unpublish_post` server fn; add `is_author` to `TimelinePostSummary` |
| `web/src/pages/ui.rs` | New `ComposerFields` component; update `InlineComposer` + `PostCard` |
| `web/src/pages/posts.rs` | Update `render_post_article`, `CreatePostPage`, `EditPostPage`, `render_draft_row`, `render_timeline_post_row`, `PostPage` |
| `server/assets/jaunder.css` | Segmented control, post actions column, draft/permalink layouts |
| `end2end/tests/posts.spec.ts` | Update tests to use body headings instead of title field; use button clicks for format |

---

## Section 1 — Body storage and title caching

### Task 1: Update `extract_org_title` to also handle `* Heading`

**Files:**
- Modify: `common/src/render.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `common/src/render.rs`:

```rust
#[test]
fn extract_org_title_handles_level1_heading() {
    let result = extract_org_title("* My Title\n\nBody text");
    assert_eq!(result, Some(("My Title".to_string(), "Body text".to_string())));
}

#[test]
fn extract_org_title_heading_after_kv_lines() {
    let result = extract_org_title("#+AUTHOR: Me\n* My Title\n\nBody");
    assert_eq!(result, Some(("My Title".to_string(), "Body".to_string())));
}

#[test]
fn extract_org_title_title_takes_precedence_over_heading() {
    let result = extract_org_title("#+TITLE: Meta\n* Heading\n\nBody");
    assert_eq!(result, Some(("Meta".to_string(), "* Heading\n\nBody".to_string())));
}

#[test]
fn extract_org_title_heading_not_top_level_ignored() {
    // ** is a level-2 heading, not a title
    let result = extract_org_title("** Sub\n\nBody");
    assert_eq!(result, None);
}

#[test]
fn extract_org_title_heading_after_body_text_ignored() {
    // A heading preceded by prose is not a title
    let result = extract_org_title("Some intro text.\n* Heading\n\nBody");
    assert_eq!(result, None);
}

#[test]
fn derive_metadata_extracts_org_level1_heading() {
    let metadata =
        derive_post_metadata(None, "* Org Heading\n\nBody text", &PostFormat::Org).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Org Heading"));
    assert_eq!(metadata.slug_seed, "Org Heading");
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo nextest run -E 'test(extract_org_title_handles_level1_heading) | test(extract_org_title_heading_after_kv_lines) | test(extract_org_title_title_takes_precedence_over_heading) | test(extract_org_title_heading_not_top_level_ignored) | test(extract_org_title_heading_after_body_text_ignored) | test(derive_metadata_extracts_org_level1_heading)'
```

Expected: all fail.

- [ ] **Step 3: Update `extract_org_title` in `common/src/render.rs`**

Replace the entire `extract_org_title` function body with:

```rust
fn extract_org_title(body: &str) -> Option<(String, String)> {
    let mut output = Vec::new();
    let mut found = None;
    let mut past_kv_block = false;

    for line in body.lines() {
        if found.is_some() {
            output.push(line);
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Blank lines are allowed inside the KV block
            if !past_kv_block {
                continue;
            }
            // Once we're past the KV block, a blank line without a title means no title
            return None;
        }

        // #+TITLE: value — standard org metadata title
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.eq_ignore_ascii_case("#+title") {
                let title = value.trim();
                if !title.is_empty() {
                    found = Some(title.to_owned());
                    continue;
                }
            }
            // Any other #+key: value KV line is skipped (part of the header block)
            if key.starts_with("#+") || key.starts_with("#+ ") {
                continue;
            }
        }

        // * Top-level heading (exactly one asterisk followed by space)
        if let Some(heading) = trimmed.strip_prefix("* ") {
            let title = heading.trim();
            if !title.is_empty() {
                found = Some(title.to_owned());
                past_kv_block = true;
                continue;
            }
        }

        // Any other non-blank, non-KV, non-heading content means no title
        return None;
    }

    found.map(|title| (title, output.join("\n").trim().to_owned()))
}
```

- [ ] **Step 4: Run tests to confirm they pass**

```bash
cargo nextest run -E 'test(extract_org_title) | test(derive_metadata_extracts_org)'
```

Expected: all pass (including the existing `derive_metadata_extracts_org_title` test).

- [ ] **Step 5: Commit**

```bash
git add common/src/render.rs
git commit -m "feat: extract_org_title now recognises top-level * Heading as title"
```

---

### Task 2: Store body verbatim — remove `body` from `DerivedPostMetadata`

**Files:**
- Modify: `common/src/render.rs`
- Modify: `web/src/posts.rs`

- [ ] **Step 1: Update the existing body-unchanged tests**

The tests `derive_metadata_extracts_markdown_h1` and `derive_metadata_extracts_org_title` currently assert `metadata.body == "Body text"` (stripped). They will need to assert the field is gone. Update them (they will fail after the struct change) as follows — note the `body` field is replaced by checking the original input is unchanged:

These tests will naturally break once the field is removed in Step 2; fix them then.

- [ ] **Step 2: Remove `body` from `DerivedPostMetadata` and stop stripping in `derive_post_metadata`**

In `common/src/render.rs`, change `DerivedPostMetadata`:

```rust
/// Metadata derived from a post body used for slug generation and display.
pub struct DerivedPostMetadata {
    pub title: Option<String>,
    pub slug_seed: String,
    pub summary_label: String,
}
```

Replace the entire `derive_post_metadata` function body:

```rust
pub fn derive_post_metadata(
    explicit_title: Option<&str>,
    body: &str,
    format: &PostFormat,
) -> Option<DerivedPostMetadata> {
    let explicit_title = explicit_title
        .map(str::trim)
        .filter(|title| !title.is_empty());
    let body = body.trim();

    if let Some(title) = explicit_title {
        let title = title.to_owned();
        let summary_label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            slug_seed: title,
            summary_label,
        });
    }

    let extracted_title = match format {
        PostFormat::Markdown => extract_markdown_title(body).map(|(title, _)| title),
        PostFormat::Org => extract_org_title(body).map(|(title, _)| title),
    };

    if let Some(title) = extracted_title {
        let summary_label = fallback_label(body).unwrap_or_else(|| title.clone());
        return Some(DerivedPostMetadata {
            title: Some(title.clone()),
            slug_seed: title,
            summary_label,
        });
    }

    let summary_label = fallback_label(body)?;
    Some(DerivedPostMetadata {
        title: None,
        slug_seed: summary_label.clone(),
        summary_label,
    })
}
```

- [ ] **Step 3: Update `perform_post_update` in `common/src/render.rs` to use original `body`**

Replace the current `perform_post_update` function:

```rust
pub async fn perform_post_update(
    storage: &dyn PostStorage,
    post_id: i64,
    editor_user_id: i64,
    body: String,
    format: PostFormat,
    slug_override: Option<&str>,
    publish: bool,
) -> Result<PostRecord, PerformUpdateError> {
    let metadata = derive_post_metadata(None, &body, &format)
        .ok_or(PerformUpdateError::EmptyPost)?;

    let slug = match slug_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => raw
            .to_ascii_lowercase()
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::InvalidSlug)?,
        None => slugify_title(&metadata.slug_seed)
            .ok_or(PerformUpdateError::NoSlugFromPost)?
            .parse::<Slug>()
            .map_err(|_| PerformUpdateError::NoSlugFromPost)?,
    };

    let rendered_html = render(&body, &format)?;
    let input = UpdatePostInput {
        title: metadata.title,
        slug,
        body,
        format,
        rendered_html,
        publish,
    };
    storage
        .update_post(post_id, editor_user_id, &input)
        .await
        .map_err(PerformUpdateError::from)
}
```

Note: `title: Option<String>` parameter is removed (always derived from body now).

- [ ] **Step 4: Fix compile errors caused by removing `title` from `perform_post_update`**

In `web/src/posts.rs`, update the `update_post` server function to remove `title` from both the function signature and the `perform_post_update` call:

```rust
#[server(endpoint = "/update_post")]
pub async fn update_post(
    post_id: i64,
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> Result<UpdatePostResult, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let format = format
        .parse::<PostFormat>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let record = perform_post_update(
        state.posts.as_ref(),
        post_id,
        auth.user_id,
        body,
        format,
        slug_override.as_deref(),
        publish,
    )
    .await
    .map_err(|e| match e {
        PerformUpdateError::NotFound | PerformUpdateError::Unauthorized => {
            ServerFnError::new("Post not found")
        }
        other => ServerFnError::new(other.to_string()),
    })?;

    let published_at_str = record.published_at.map(|t| t.to_rfc3339());
    let permalink = record
        .published_at
        .map(|ts| build_permalink(&auth.username, ts, &record.slug));

    Ok(UpdatePostResult {
        post_id,
        slug: record.slug.to_string(),
        published_at: published_at_str,
        preview_url: format!("/draft/{post_id}/preview"),
        permalink,
    })
}
```

- [ ] **Step 5: Update `create_post` in `web/src/posts.rs` to remove `title` and use verbatim body**

```rust
#[server(endpoint = "/create_post")]
pub async fn create_post(
    body: String,
    format: String,
    slug_override: Option<String>,
    publish: bool,
) -> Result<CreatePostResult, ServerFnError> {
    let auth = require_auth().await?;
    let state = expect_context::<Arc<AppState>>();

    let format = format
        .parse::<PostFormat>()
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let metadata = derive_post_metadata(None, &body, &format)
        .ok_or_else(|| ServerFnError::new("post body is required"))?;
    let published_at = publish.then(Utc::now);
    let slug_seed = slug_override
        .as_deref()
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
        .map(|slug| slug.to_ascii_lowercase())
        .map(|slug| slug.parse::<Slug>())
        .transpose()
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .map(|slug| slug.to_string())
        .or_else(|| slugify_title(&metadata.slug_seed))
        .ok_or_else(|| {
            ServerFnError::new(
                "post must contain at least one ASCII letter or digit for its slug",
            )
        })?;

    let created = create_post_with_unique_slug(
        state.as_ref(),
        auth.user_id,
        &auth.username,
        metadata.title,
        body,          // verbatim — no longer metadata.body
        format,
        slug_seed,
        published_at,
    )
    .await?;

    Ok(created)
}
```

- [ ] **Step 6: Fix unit tests that referenced `metadata.body`**

In `common/src/render.rs` tests, update tests that used `metadata.body`:

```rust
#[test]
fn derive_metadata_extracts_markdown_h1() {
    let metadata =
        derive_post_metadata(None, "# Markdown Title\n\nBody text", &PostFormat::Markdown)
            .unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Markdown Title"));
    assert_eq!(metadata.slug_seed, "Markdown Title");
    // body is not a field of DerivedPostMetadata — the caller retains the original
}

#[test]
fn derive_metadata_extracts_org_title() {
    let metadata =
        derive_post_metadata(None, "#+title: Org Title\n\nBody text", &PostFormat::Org)
            .unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Org Title"));
    assert_eq!(metadata.slug_seed, "Org Title");
}
```

Also update the `perform_update_error_*` tests if they reference the `title` parameter of `perform_post_update` — the function signature changed.

- [ ] **Step 7: Run all unit tests**

```bash
cargo nextest run
```

Expected: all pass. Fix any remaining compile errors.

- [ ] **Step 8: Commit**

```bash
git add common/src/render.rs web/src/posts.rs
git commit -m "feat: store post body verbatim; derive title cache from content only"
```

---

### Task 3: Update `render_post_article` for smart `<h1>` injection

**Files:**
- Modify: `web/src/pages/posts.rs`

Context: for markdown posts, `# Title` renders as `<h1>` in `rendered_html`. For org posts with `#+TITLE:`, orgize skips it — no `<h1>` in `rendered_html`. We check `rendered_html` to decide whether to inject a template `<h1>`.

- [ ] **Step 1: Replace `render_post_article` in `web/src/pages/posts.rs`**

Find the function starting at line 782. Replace it with:

```rust
fn render_post_article(post: PostResponse, banner: Option<&'static str>) -> AnyView {
    let PostResponse {
        title,
        username,
        rendered_html,
        created_at,
        published_at,
        ..
    } = post;
    let profile_href = format!("/~{}/", username);
    let username_display = username.clone();
    let display_time = published_at
        .as_deref()
        .map(format_post_time)
        .unwrap_or_else(|| format_post_time(&created_at));

    // If rendered_html already opens with <h1> (e.g. from a markdown `# Title` or
    // org `* Title`), don't inject a second one. Only inject for cases like org
    // `#+TITLE:` where the renderer doesn't produce a heading element.
    // Note: this is a mild encapsulation violation — we're inferring renderer
    // behaviour from the output. Accepted as a known trade-off (see design spec).
    let has_rendered_h1 = rendered_html.trim_start().starts_with("<h1");
    let template_h1 = (!has_rendered_h1).then(|| title.map(|t| view! { <h1>{t}</h1> }));

    view! {
        <article>
            {template_h1}
            <p class="metadata">
                "By " <a href=profile_href>{username_display}</a> " on " {display_time}
            </p>
            {banner.map(|text| view! { <p class="draft-banner">{text}</p> })}
            <div class="content" inner_html=rendered_html></div>
        </article>
    }
    .into_any()
}
```

- [ ] **Step 2: Update the e2e test that checks for `article h1`**

In `end2end/tests/posts.spec.ts`, the test `"published post renders at permalink"` currently:
1. Creates a post with `input[name="title"]` = `"Permalink Story"` and body `"**hello permalink**"`
2. Checks `page.locator("article h1")` has text `"Permalink Story"`

Update to embed the title in the body as a markdown heading, and update the `createPublishedPostViaApi` helper to omit `title` and embed it in `body`:

```typescript
async function createPublishedPostViaApi(
  page: Page,
  title: string,
): Promise<void> {
  const response = await withTimedAction(page, "api.create_post", () =>
    page.request.post(`${BASE_URL}/api/create_post`, {
      form: {
        body: `# ${title}\n\nBody for ${title}`,
        format: "markdown",
        publish: "true",
      },
    }),
  );
  expect(response.ok()).toBeTruthy();
}
```

Also update the UI-based post creation tests in `posts.spec.ts` to stop filling `input[name="title"]` and use `# Title` in the body instead. Update format selection from `page.selectOption('select[name="format"]', ...)` to clicking the appropriate button:

```typescript
// Old: await page.fill('input[name="title"]', "Playwright Post");
// Old: await page.fill('textarea[name="body"]', "**browser**");
// Old: await page.selectOption('select[name="format"]', "markdown");
// New:
await page.fill('textarea[name="body"]', "# Playwright Post\n\n**browser**");
// Markdown is the default; click if needed:
await page.click('button[data-format="markdown"]');
```

Similarly update: "authenticated user can save a draft through the UI", "published post renders at permalink", "authenticated user can edit a draft post", "editing a published post freezes the slug".

For the edit tests, the body will now contain the heading. When editing, the full body (with `# Title`) is pre-filled in the textarea, so fill accordingly:

```typescript
// Old: await page.fill('input[name="title"]', "Edited Draft");
// Old: await page.fill('textarea[name="body"]', "**edited content**");
// New:
await page.fill('textarea[name="body"]', "# Edited Draft\n\n**edited content**");
```

- [ ] **Step 3: Run the full test suite**

```bash
cargo nextest run
```

Expected: all unit tests pass. E2e tests will pass after CSS/component changes in later tasks.

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/posts.rs end2end/tests/posts.spec.ts
git commit -m "feat: render_post_article injects <h1> only when rendered_html lacks one; update e2e tests"
```

---

## Section 2 — Shared `ComposerFields` component

### Task 4: Add CSS for segmented control, post actions, and new layouts

**Files:**
- Modify: `server/assets/jaunder.css`

- [ ] **Step 1: Add segmented control styles**

After the `.j-btn.is-active` rule (around line 282), add:

```css
/* ─── segmented control ─── */
.j-seg { display: inline-flex; }
.j-seg .j-btn { border-radius: 0; border-right-width: 0; }
.j-seg .j-btn:first-child { border-radius: var(--radius-sm) 0 0 var(--radius-sm); }
.j-seg .j-btn:last-child  {
  border-radius: 0 var(--radius-sm) var(--radius-sm) 0;
  border-right-width: 1px;
}
.j-seg .j-btn.is-selected {
  background: var(--ink);
  color: var(--surface);
  font-weight: 500;
}
```

- [ ] **Step 2: Add post actions column styles**

After the `.j-post-foot` rule block (around line 331), add:

```css
/* ─── post actions column ─── */
.j-post-acts {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
  flex-shrink: 0;
  padding-left: 8px;
}
.j-post-plink {
  font-size: 12px;
  color: var(--muted-soft);
  text-decoration: none;
  white-space: nowrap;
}
.j-post-plink:hover { color: var(--ink); }
.j-post-acts .j-btn { font-size: 12px; padding: 3px 8px; }
```

- [ ] **Step 3: Add permalink page two-column layout**

After the `/* ─── drafts list ─── */` block (around line 557), add:

```css
/* ─── permalink page ─── */
.j-post-page {
  display: flex;
  gap: 24px;
  align-items: flex-start;
  padding: 28px 32px;
}
.j-post-page article { flex: 1; min-width: 0; }

/* ─── draft row ─── */
.j-draft-row {
  display: flex;
  gap: 16px;
  padding: 14px 0;
  border-bottom: 1px var(--border-style) var(--line-soft);
  align-items: flex-start;
}
.j-draft-row:first-child { border-top: 1px var(--border-style) var(--line-soft); }
.j-draft-row-content { flex: 1; min-width: 0; }
```

- [ ] **Step 4: Update `.j-draft-actions`**

Find `.j-draft-actions { display: flex; gap: 8px; }` and replace:

```css
.j-draft-actions {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  flex-shrink: 0;
}
```

- [ ] **Step 5: Add composer header style**

Before `/* ─── compose page ─── */` (around line 333), add:

```css
/* ─── composer header (avatar + username above compose grid) ─── */
.j-compose-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 20px 32px 0;
  background: var(--surface);
  font-size: 13px;
  color: var(--muted);
  font-family: var(--font-meta);
}
```

- [ ] **Step 6: Build to check for CSS errors**

```bash
cargo build 2>&1 | head -30
```

Expected: clean build.

- [ ] **Step 7: Commit**

```bash
git add server/assets/jaunder.css
git commit -m "feat: add CSS for segmented control, post actions column, and new layouts"
```

---

### Task 5: Add `ComposerFields` component

**Files:**
- Modify: `web/src/pages/ui.rs`

`ComposerFields` renders the two-panel grid: textarea in the left panel, format segmented control + optional slug in the aside. Caller provides action buttons as `children`.

- [ ] **Step 1: Add `ComposerFields` to `web/src/pages/ui.rs`**

Add before the `InlineComposer` function:

```rust
/// Shared two-panel composer layout.
/// Left: body textarea. Right aside: format segmented control, optional slug, then children (action buttons).
/// Wrap this in an `ActionForm` and provide a composer header outside it.
#[component]
pub fn ComposerFields(
    body: RwSignal<String>,
    format: RwSignal<String>,
    #[prop(default = "markdown".to_string())] default_format: String,
    #[prop(optional)] show_slug: bool,
    #[prop(optional)] slug: Option<RwSignal<String>>,
    #[prop(default = true)] slug_editable: bool,
    children: Children,
) -> impl IntoView {
    // Initialise format signal from default if not already set.
    if format.get_untracked().is_empty() {
        format.set(default_format);
    }

    view! {
        <div class="j-compose-grid">
            <div class="j-compose-body">
                <textarea
                    class="j-compose-editor"
                    name="body"
                    rows="20"
                    prop:value=move || body.get()
                    on:input=move |ev| body.set(event_target_value(&ev))
                ></textarea>
                <input type="hidden" name="format" prop:value=move || format.get() />
            </div>
            <aside class="j-compose-aside">
                <div>
                    <div class="j-sb-head" style="padding:0 0 10px">"Options"</div>
                    <div class="j-field-row" style="grid-template-columns:auto 1fr">
                        <label class="j-field-label">"Format"</label>
                        <div class="j-seg">
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "markdown" {
                                        "j-btn is-selected"
                                    } else {
                                        "j-btn"
                                    }
                                }
                                data-format="markdown"
                                on:click=move |_| format.set("markdown")
                            >
                                "Markdown"
                            </button>
                            <button
                                type="button"
                                class=move || {
                                    if format.get() == "org" { "j-btn is-selected" } else { "j-btn" }
                                }
                                data-format="org"
                                on:click=move |_| format.set("org")
                            >
                                "Org"
                            </button>
                        </div>
                    </div>
                    {show_slug
                        .then(|| {
                            let slug_sig = slug.unwrap_or_else(|| RwSignal::new(String::new()));
                            if slug_editable {
                                view! {
                                    <div class="j-field-row" style="grid-template-columns:auto 1fr">
                                        <label class="j-field-label" for="composer-slug">
                                            "Slug"
                                        </label>
                                        <input
                                            id="composer-slug"
                                            type="text"
                                            name="slug_override"
                                            class="j-field-val"
                                            placeholder="auto"
                                            prop:value=move || slug_sig.get()
                                            on:input=move |ev| {
                                                slug_sig.set(event_target_value(&ev))
                                            }
                                        />
                                    </div>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <div class="j-field-row" style="grid-template-columns:auto 1fr">
                                        <label class="j-field-label">"Slug"</label>
                                        <span class="j-field-val" style="background:var(--surface-alt);color:var(--muted)">
                                            {move || slug_sig.get()}
                                        </span>
                                        <input
                                            type="hidden"
                                            name="slug_override"
                                            prop:value=move || slug_sig.get()
                                        />
                                    </div>
                                }
                                    .into_any()
                            }
                        })}
                </div>
                <div class="j-edit-form-actions">{children()}</div>
            </aside>
        </div>
    }
}
```

- [ ] **Step 2: Build to confirm it compiles**

```bash
cargo build 2>&1 | head -40
```

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add web/src/pages/ui.rs
git commit -m "feat: add ComposerFields shared two-panel composer component"
```

---

### Task 6: Update `InlineComposer` to use `ComposerFields`

**Files:**
- Modify: `web/src/pages/ui.rs`

- [ ] **Step 1: Replace `InlineComposer` body**

Replace the entire `InlineComposer` function in `web/src/pages/ui.rs`:

```rust
#[component]
pub fn InlineComposer(username: String, on_publish: WriteSignal<u32>) -> impl IntoView {
    let create_action = ServerAction::<CreatePost>::new();
    let body = RwSignal::new(String::new());
    let format = RwSignal::new(String::new());
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    #[cfg(target_arch = "wasm32")]
    {
        use leptos_dom::helpers::set_timeout;
        use std::time::Duration;
        Effect::new(move |_| {
            if let Some(Ok(ref created)) = create_action.value().get() {
                body.set(String::new());
                let url = created
                    .permalink
                    .clone()
                    .unwrap_or_else(|| created.preview_url.clone());
                let msg = if created.published_at.is_some() {
                    "Post published!".to_string()
                } else {
                    "Draft saved!".to_string()
                };
                flash.set(Some((url, msg)));
                set_timeout(move || flash.set(None), Duration::from_secs(30));
                if created.published_at.is_some() {
                    on_publish.update(|v| *v += 1);
                }
            }
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = on_publish;

    view! {
        <div class="j-composer">
            <div class="j-compose-header">
                <Avatar name=username.clone() size=36 />
                <span>{username}</span>
            </div>
            <ActionForm action=create_action>
                <input type="hidden" name="slug_override" value="" />
                <ComposerFields body=body format=format default_format="markdown".to_string()>
                    <button
                        class="j-btn"
                        type="submit"
                        name="publish"
                        value="false"
                        disabled=move || body.get().trim().is_empty()
                    >
                        "Save draft"
                    </button>
                    <button
                        class="j-btn is-primary"
                        type="submit"
                        name="publish"
                        value="true"
                        disabled=move || body.get().trim().is_empty()
                    >
                        "Publish"
                    </button>
                </ComposerFields>
            </ActionForm>
            {move || {
                if let Some(e) = create_action.value().get().and_then(|r| r.err()) {
                    return view! { <p class="error">{e.to_string()}</p> }.into_any();
                }
                if let Some((url, msg)) = flash.get() {
                    return view! {
                        <p class="success">
                            <a href=url>{msg}</a>
                        </p>
                    }
                        .into_any();
                }
                ().into_any()
            }}
        </div>
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 3: Commit**

```bash
git add web/src/pages/ui.rs
git commit -m "refactor: InlineComposer now uses shared ComposerFields"
```

---

### Task 7: Update `CreatePostPage` to use `ComposerFields`

**Files:**
- Modify: `web/src/pages/posts.rs`

- [ ] **Step 1: Replace `CreatePostPage` body**

Replace the entire `CreatePostPage` function:

```rust
#[component]
pub fn CreatePostPage() -> impl IntoView {
    let create_post_action = ServerAction::<CreatePost>::new();
    let current_user = Resource::new(|| (), |_| current_user());
    let body = RwSignal::new(String::new());
    let format = RwSignal::new(String::new());
    let slug = RwSignal::new(String::new());
    let char_count = move || body.get().len();

    view! {
        <Topbar title="New post".to_string() sub="Long-form".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match current_user.await {
                    Ok(Some(username)) => {
                        view! {
                            <div class="j-compose-header">
                                <Avatar name=username.clone() size=40 />
                                <span>{username}</span>
                            </div>
                            <ActionForm action=create_post_action>
                                <ComposerFields
                                    body=body
                                    format=format
                                    default_format="markdown".to_string()
                                    show_slug=true
                                    slug=Some(slug)
                                    slug_editable=true
                                >
                                    <span class="j-count" style="margin-left:0">
                                        {char_count}
                                    </span>
                                    <span class="j-spacer"></span>
                                    <button
                                        class="j-btn"
                                        type="submit"
                                        name="publish"
                                        value="false"
                                    >
                                        "Draft"
                                    </button>
                                    <button
                                        class="j-btn is-primary"
                                        type="submit"
                                        name="publish"
                                        value="true"
                                    >
                                        "Publish \u{2192}"
                                    </button>
                                </ComposerFields>
                            </ActionForm>
                            {move || {
                                create_post_action
                                    .value()
                                    .get()
                                    .map(|result: Result<CreatePostResult, ServerFnError>| {
                                        match result {
                                            Ok(created) => {
                                                let message = if created.published_at.is_some() {
                                                    "Post published."
                                                } else {
                                                    "Draft saved."
                                                };
                                                let slug_value = created.slug.clone();
                                                let slug_for_attr = slug_value.clone();
                                                view! {
                                                    <div class="success" style="padding:16px 32px">
                                                        <p>{message}</p>
                                                        <p
                                                            data-test="slug-value"
                                                            data-slug=slug_for_attr
                                                        >
                                                            "Slug: "
                                                            {slug_value}
                                                        </p>
                                                        <a
                                                            data-test="preview-link"
                                                            href=created.preview_url.clone()
                                                        >
                                                            "Preview draft"
                                                        </a>
                                                        {created
                                                            .permalink
                                                            .as_ref()
                                                            .map(|href| {
                                                                view! {
                                                                    <a
                                                                        data-test="permalink-link"
                                                                        href=href.clone()
                                                                    >
                                                                        "View permalink"
                                                                    </a>
                                                                }
                                                            })}
                                                    </div>
                                                }
                                                    .into_any()
                                            }
                                            Err(err) => {
                                                view! {
                                                    <p class="error" style="padding:16px 32px">
                                                        {err.to_string()}
                                                    </p>
                                                }
                                                    .into_any()
                                            }
                                        }
                                    })
                            }}
                        }
                            .into_any()
                    }
                    Ok(None) => {
                        view! {
                            <div style="padding:32px">
                                <p>"You must be logged in to create a post."</p>
                                <p>
                                    <a href="/login" class="j-btn is-primary">"Sign in"</a>
                                </p>
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 3: Commit**

```bash
git add web/src/pages/posts.rs
git commit -m "refactor: CreatePostPage now uses shared ComposerFields; remove title input"
```

---

### Task 8: Update `EditPostPage` to use `ComposerFields`

**Files:**
- Modify: `web/src/pages/posts.rs`

**Note on existing posts:** Posts created before this change have their bodies stored without the title heading (it was stripped). When loading such posts, if the post has a cached title but the body doesn't start with the expected heading syntax, prepend the heading so the user sees the complete content. For markdown: `# {title}\n\n{body}`. For org: `#+TITLE: {title}\n{body}`. This is a one-time migration on first edit.

- [ ] **Step 1: Add a `reconstruct_body_if_needed` helper at the bottom of `web/src/pages/posts.rs`**

```rust
/// For posts created before verbatim storage was introduced, the title heading
/// was stripped from the body. Reconstruct it on the edit page so the user sees
/// (and saves) the complete content.
fn reconstruct_body_if_needed(
    title: Option<&str>,
    body: &str,
    format: &str,
) -> String {
    let Some(title) = title else {
        return body.to_string();
    };
    match format {
        "markdown" if !body.trim_start().starts_with("# ") => {
            format!("# {title}\n\n{body}")
        }
        "org"
            if !body.trim_start().starts_with("#+TITLE:")
                && !body.trim_start().starts_with("#+title:")
                && !body.trim_start().starts_with("* ") =>
        {
            format!("#+TITLE: {title}\n{body}")
        }
        _ => body.to_string(),
    }
}
```

- [ ] **Step 2: Replace `EditPostPage` body**

```rust
#[component]
pub fn EditPostPage() -> impl IntoView {
    let params = use_params_map();
    let update_post_action = ServerAction::<UpdatePost>::new();

    let post = Resource::new(
        move || {
            params
                .get()
                .get("post_id")
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(-1)
        },
        get_post_preview,
    );

    view! {
        <Topbar title="Edit Post".to_string() sub="".to_string() />
        <Suspense fallback=|| {
            view! { <p class="j-loading">"Loading\u{2026}"</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched) => {
                        let post_id = fetched.post_id;
                        let is_published = fetched.published_at.is_some();
                        let current_slug = fetched.slug.clone();
                        let reconstructed = reconstruct_body_if_needed(
                            fetched.title.as_deref(),
                            &fetched.body,
                            &fetched.format,
                        );
                        let body = RwSignal::new(reconstructed);
                        let format = RwSignal::new(fetched.format.clone());
                        let slug = RwSignal::new(current_slug.clone());

                        view! {
                            <ActionForm action=update_post_action>
                                <input type="hidden" name="post_id" value=post_id />
                                <ComposerFields
                                    body=body
                                    format=format
                                    show_slug=true
                                    slug=Some(slug)
                                    slug_editable=!is_published
                                >
                                    {if is_published {
                                        view! {
                                            <button
                                                class="j-btn is-primary"
                                                type="submit"
                                                name="publish"
                                                value="true"
                                            >
                                                "Save"
                                            </button>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <button
                                                class="j-btn is-primary"
                                                type="submit"
                                                name="publish"
                                                value="true"
                                            >
                                                "Publish \u{2192}"
                                            </button>
                                            <button
                                                class="j-btn"
                                                type="submit"
                                                name="publish"
                                                value="false"
                                            >
                                                "Save Draft"
                                            </button>
                                        }
                                            .into_any()
                                    }}
                                </ComposerFields>
                            </ActionForm>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            update_post_action
                .value()
                .get()
                .map(|result: Result<UpdatePostResult, ServerFnError>| match result {
                    Ok(updated) => {
                        let message = if updated.published_at.is_some() {
                            "Post updated."
                        } else {
                            "Draft saved."
                        };
                        let slug_value = updated.slug.clone();
                        let slug_for_attr = slug_value.clone();
                        view! {
                            <div class="success">
                                <p>{message}</p>
                                <p data-test="slug-value" data-slug=slug_for_attr>
                                    "Slug: "
                                    {slug_value}
                                </p>
                                <a data-test="preview-link" href=updated.preview_url.clone()>
                                    "Preview draft"
                                </a>
                                {updated
                                    .permalink
                                    .as_ref()
                                    .map(|href| {
                                        view! {
                                            <a data-test="permalink-link" href=href.clone()>
                                                "View permalink"
                                            </a>
                                        }
                                    })}
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
    }
}
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 4: Update e2e test for edit page**

In `posts.spec.ts`, the edit tests currently fill `input[name="title"]`. Update them to fill the body instead (no title input exists anymore). Also update format selection to use button clicks:

```typescript
// For "authenticated user can edit a draft post":
// Old: await page.fill('input[name="title"]', "Original Draft");
// New:
await page.fill('textarea[name="body"]', "# Original Draft\n\noriginal body");
// (Remove the separate title fill)

// For editing:
// Old: await page.fill('input[name="title"]', "Edited Draft");
// Old: await page.fill('textarea[name="body"]', "**edited content**");
// New:
await page.fill('textarea[name="body"]', "# Edited Draft\n\n**edited content**");
```

For the slug-freeze test:
```typescript
// Old: await page.fill('input[name="title"]', "Published Article");
// New:
await page.fill('textarea[name="body"]', "# Published Article\n\noriginal content");

// Old: await page.fill('input[name="title"]', "Updated Article");
// New: (just save, no title change needed — the heading in body handles it)
// If needed: await page.fill('textarea[name="body"]', "# Updated Article\n\noriginal content");
```

Also update the check for slug_override visibility: it's now read-only text rather than hidden, so update:
```typescript
// Old: await expect(page.locator('input[name="slug_override"]')).not.toBeVisible();
// New: the slug_override hidden input is always present; the visible text field is absent for published posts.
// Check the input[name="slug_override"] exists but the editable text input does not:
await expect(page.locator('input[type="text"][name="slug_override"]')).not.toBeVisible();
```

- [ ] **Step 5: Run all tests**

```bash
cargo nextest run
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add web/src/pages/posts.rs end2end/tests/posts.spec.ts
git commit -m "refactor: EditPostPage uses ComposerFields; reconstruct body heading for legacy posts"
```

---

## Section 3 — Author actions and permalink links

### Task 9: Add `unpublish_post` storage method and server function

**Files:**
- Modify: `common/src/storage.rs`
- Modify: `server/src/storage/sqlite.rs`
- Modify: `server/src/storage/postgres.rs`
- Modify: `web/src/posts.rs`

- [ ] **Step 1: Add `unpublish_post` to the `PostStorage` trait**

In `common/src/storage.rs`, add to the `PostStorage` trait after `soft_delete_post`:

```rust
/// Sets `published_at = NULL` for a post. Does not check ownership; the caller
/// must verify ownership before calling. No-ops silently if the post is already
/// a draft or does not exist.
async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()>;
```

- [ ] **Step 2: Implement `unpublish_post` for SQLite**

In `server/src/storage/sqlite.rs`, add after the `soft_delete_post` implementation:

```rust
async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE posts SET published_at = NULL, updated_at = ? WHERE post_id = ? AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(post_id)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 3: Implement `unpublish_post` for PostgreSQL**

In `server/src/storage/postgres.rs`, add after `soft_delete_post`:

```rust
async fn unpublish_post(&self, post_id: i64) -> sqlx::Result<()> {
    let now = Utc::now();
    sqlx::query(
        "UPDATE posts SET published_at = NULL, updated_at = $1 WHERE post_id = $2 AND deleted_at IS NULL",
    )
    .bind(now)
    .bind(post_id)
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Add `unpublish_post` server function to `web/src/posts.rs`**

Add after the `delete_post` function:

```rust
/// Moves a published post back to draft for the authenticated author.
#[server(endpoint = "/unpublish_post")]
pub async fn unpublish_post(post_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let auth = require_auth().await?;
        let state = expect_context::<Arc<AppState>>();

        let existing = state
            .posts
            .get_post_by_id(post_id)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| ServerFnError::new("Post not found"))?;

        if existing.deleted_at.is_some() || existing.user_id != auth.user_id {
            return Err(ServerFnError::new("Post not found"));
        }

        state
            .posts
            .unpublish_post(post_id)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))
    }
    #[cfg(not(feature = "ssr"))]
    {
        let _ = post_id;
        Err(ServerFnError::new("Not implemented"))
    }
}
```

- [ ] **Step 5: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 6: Write an e2e test for unpublish**

Add to `end2end/tests/posts.spec.ts`:

```typescript
test("author can unpublish a post from the permalink page", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  // Create and publish a post
  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Unpublish Test\n\nsome content");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".success");

  const permalinkHref = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  // Go to permalink page
  await goto(page, permalinkHref!);

  // Author sees Unpublish button
  const unpublishBtn = page.locator('button[data-test="unpublish-btn"]');
  await expect(unpublishBtn).toBeVisible();

  // Unpublish with confirmation
  page.once("dialog", (dialog) => dialog.accept());
  await click(page, 'button[data-test="unpublish-btn"]');

  // Should redirect to drafts or show success
  await waitForSelector(page, '[data-test="unpublish-success"]');
});
```

- [ ] **Step 7: Run unit + build tests**

```bash
cargo nextest run
```

Expected: pass. E2e test will pass after Task 15 wires up the button.

- [ ] **Step 8: Commit**

```bash
git add common/src/storage.rs server/src/storage/sqlite.rs server/src/storage/postgres.rs web/src/posts.rs end2end/tests/posts.spec.ts
git commit -m "feat: add unpublish_post storage method and server function"
```

---

### Task 10: Add `is_author` to `TimelinePostSummary`

**Files:**
- Modify: `web/src/posts.rs`

- [ ] **Step 1: Add `is_author` to the struct**

In `web/src/posts.rs`, update `TimelinePostSummary`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelinePostSummary {
    pub post_id: i64,
    pub username: String,
    pub title: Option<String>,
    pub slug: String,
    pub rendered_html: String,
    pub created_at: String,
    pub published_at: String,
    pub permalink: String,
    pub is_author: bool,
}
```

- [ ] **Step 2: Update `timeline_post_summary` to accept `current_user_id`**

Replace the `timeline_post_summary` function:

```rust
#[cfg(feature = "ssr")]
fn timeline_post_summary(
    username: &Username,
    post: PostRecord,
    current_user_id: Option<i64>,
) -> Option<TimelinePostSummary> {
    let PostRecord {
        post_id,
        user_id,
        title,
        slug,
        rendered_html,
        created_at,
        published_at,
        ..
    } = post;
    let published_at = published_at?;
    let permalink = build_permalink(username, published_at, &slug);
    Some(TimelinePostSummary {
        post_id,
        username: username.to_string(),
        title,
        slug: slug.to_string(),
        rendered_html,
        created_at: created_at.to_rfc3339(),
        published_at: published_at.to_rfc3339(),
        permalink,
        is_author: current_user_id.map(|id| id == user_id).unwrap_or(false),
    })
}
```

- [ ] **Step 3: Update callers of `timeline_post_summary`**

`list_home_feed` — all posts are the auth user's own, so `is_author` is always true. Pass `Some(auth.user_id)`:

```rust
// In list_home_feed, change the posts collection:
let posts = rows
    .into_iter()
    .filter_map(|post| timeline_post_summary(&auth.username, post, Some(auth.user_id)))
    .collect();
```

`list_user_posts` — add a soft auth check:

```rust
// At top of the #[cfg(feature = "ssr")] block in list_user_posts:
let current_user_id = require_auth().await.ok().map(|a| a.user_id);
// ...
let posts = rows
    .into_iter()
    .filter_map(|post| timeline_post_summary(&username, post, current_user_id))
    .collect();
```

`list_local_timeline` — add a soft auth check:

```rust
// At top of the #[cfg(feature = "ssr")] block in list_local_timeline:
let current_user_id = require_auth().await.ok().map(|a| a.user_id);
// ...
// In the for loop, pass current_user_id:
if let Some(summary) = timeline_post_summary(&author.username, post, current_user_id) {
    posts.push(summary);
}
```

- [ ] **Step 4: Update the unit test for `timeline_post_summary`**

The existing test `timeline_post_summary_keeps_titleless_posts_titleless` needs `current_user_id`:

Find and update the test call to pass `None`:
```rust
timeline_post_summary(&username, post, None)
```

- [ ] **Step 5: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 6: Commit**

```bash
git add web/src/posts.rs
git commit -m "feat: add is_author to TimelinePostSummary; soft auth check in public timeline endpoints"
```

---

### Task 11: Update `PostCard` for three-column layout with author actions

**Files:**
- Modify: `web/src/pages/ui.rs`

- [ ] **Step 1: Replace `PostCard` component**

```rust
#[component]
pub fn PostCard(post: TimelinePostSummary, delete_action: ServerAction<DeletePost>, unpublish_action: ServerAction<UnpublishPost>) -> impl IntoView {
    let time_label = format_post_time(&post.published_at);
    let is_author = post.is_author;
    let post_id = post.post_id;
    let edit_href = format!("/posts/{}/edit", post_id);

    view! {
        <article class="j-post">
            <Avatar name=post.username.clone() size=38 />
            <div style="min-width:0;flex:1">
                <header class="j-post-head">
                    <span class="j-post-name">{post.username.clone()}</span>
                    <span class="j-post-handle">"@"{post.username.clone()}</span>
                    <span class="j-spacer"></span>
                </header>
                {post
                    .title
                    .clone()
                    .map(|title| {
                        view! { <div class="j-post-title">{title}</div> }
                    })}
                <div class="j-post-body" inner_html=post.rendered_html.clone()></div>
                <footer class="j-post-foot">
                    <span class="j-spacer"></span>
                </footer>
            </div>
            <div class="j-post-acts">
                <a class="j-post-plink" href=post.permalink.clone()>
                    {time_label} " →"
                </a>
                {is_author
                    .then(|| {
                        view! {
                            <a href=edit_href class="j-btn">"Edit"</a>
                            <ActionForm action=unpublish_action>
                                <input type="hidden" name="post_id" value=post_id />
                                <button
                                    type="submit"
                                    class="j-btn"
                                    data-test="unpublish-btn"
                                    onclick="return confirm('Move this post back to drafts?')"
                                >
                                    "Unpublish"
                                </button>
                            </ActionForm>
                            <ActionForm action=delete_action>
                                <input type="hidden" name="post_id" value=post_id />
                                <button
                                    type="submit"
                                    class="j-btn"
                                    onclick="return confirm('Delete this post?')"
                                >
                                    "Delete"
                                </button>
                            </ActionForm>
                        }
                    })}
            </div>
        </article>
    }
}
```

- [ ] **Step 2: Update `HomePage` to pass the new server actions to `PostCard`**

In `web/src/pages/home.rs`, add server actions and pass them:

```rust
// At the top of HomePage, add:
let delete_action = ServerAction::<DeletePost>::new();
let unpublish_action = ServerAction::<UnpublishPost>::new();

// Update PostCard rendering from:
//   .map(|p| view! { <PostCard post=p /> })
// to:
.map(|p| view! {
    <PostCard
        post=p
        delete_action=delete_action
        unpublish_action=unpublish_action
    />
})
```

Add the necessary imports to `home.rs`:
```rust
use crate::posts::{DeletePost, UnpublishPost, /* existing imports */};
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/ui.rs web/src/pages/home.rs
git commit -m "feat: PostCard three-column layout with timestamp permalink and author actions"
```

---

### Task 12: Update `render_timeline_post_row` for new layout

**Files:**
- Modify: `web/src/pages/posts.rs`

- [ ] **Step 1: Update `render_timeline_post_row`**

Replace the function:

```rust
fn render_timeline_post_row(
    post: TimelinePostSummary,
    delete_action: ServerAction<DeletePost>,
    unpublish_action: ServerAction<UnpublishPost>,
) -> impl IntoView {
    let is_author = post.is_author;
    let post_id = post.post_id;
    let edit_href = format!("/posts/{}/edit", post_id);
    let time_label = format_post_time(&post.published_at);

    view! {
        <li data-test="timeline-item">
            {post.title.map(|title| view! { <h2>{title}</h2> })}
            <div class="j-post-acts" style="float:right;margin-left:12px">
                <a class="j-post-plink" href=post.permalink.clone()>
                    {time_label} " →"
                </a>
                {is_author
                    .then(|| {
                        view! {
                            <a href=edit_href class="j-btn">"Edit"</a>
                            <ActionForm action=unpublish_action>
                                <input type="hidden" name="post_id" value=post_id />
                                <button
                                    type="submit"
                                    class="j-btn"
                                    onclick="return confirm('Move this post back to drafts?')"
                                >
                                    "Unpublish"
                                </button>
                            </ActionForm>
                            <ActionForm action=delete_action>
                                <input type="hidden" name="post_id" value=post_id />
                                <button
                                    type="submit"
                                    class="j-btn"
                                    onclick="return confirm('Delete this post?')"
                                >
                                    "Delete"
                                </button>
                            </ActionForm>
                        }
                    })}
            </div>
            <div class="content" inner_html=post.rendered_html></div>
        </li>
    }
}
```

- [ ] **Step 2: Update `UserTimelinePage` to create actions and pass them to `render_timeline_post_row`**

In `UserTimelinePage`, add:
```rust
let delete_action = ServerAction::<DeletePost>::new();
let unpublish_action = ServerAction::<UnpublishPost>::new();
```

Update the map call:
```rust
rows.into_iter()
    .map(|p| render_timeline_post_row(p, delete_action, unpublish_action))
    .collect::<Vec<_>>()
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/posts.rs
git commit -m "feat: render_timeline_post_row with timestamp permalink and author actions"
```

---

### Task 13: Update `render_draft_row` for new two-column layout

**Files:**
- Modify: `web/src/pages/posts.rs`

- [ ] **Step 1: Replace `render_draft_row`**

```rust
fn render_draft_row(
    draft: DraftSummary,
    publish_action: ServerAction<PublishPost>,
    delete_action: ServerAction<DeletePost>,
) -> impl IntoView {
    let post_id = draft.post_id;
    let label = draft.title.clone().unwrap_or(draft.summary_label.clone());

    view! {
        <li class="j-draft-row">
            <div class="j-draft-row-content">
                <strong>{label}</strong>
                <span style="color:var(--muted);font-size:12px;margin-left:6px">
                    {draft.slug}
                </span>
            </div>
            <div class="j-draft-actions">
                <a href=draft.preview_url class="j-btn">"Preview"</a>
                <a href=draft.edit_url class="j-btn">"Edit"</a>
                <a href=draft.permalink class="j-btn">"Permalink"</a>
                <ActionForm action=publish_action>
                    <input type="hidden" name="post_id" value=post_id />
                    <button type="submit" class="j-btn is-primary">
                        "Publish"
                    </button>
                </ActionForm>
                <ActionForm action=delete_action>
                    <input type="hidden" name="post_id" value=post_id />
                    <button
                        type="submit"
                        class="j-btn"
                        onclick="return confirm('Delete this draft?')"
                    >
                        "Delete"
                    </button>
                </ActionForm>
            </div>
        </li>
    }
}
```

- [ ] **Step 2: Update `DraftsPage` to use `<ul class="j-draft-list">`**

In `DraftsPage`, wrap the draft list with a style that removes list bullets:

Find `<ul>` in `DraftsPage` and change to `<ul style="list-style:none;padding:0;margin:0">`.

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 4: Commit**

```bash
git add web/src/pages/posts.rs
git commit -m "feat: draft list two-column layout; all actions as buttons in actions column"
```

---

### Task 14: Update `PostPage` for two-column layout with Edit/Unpublish/Delete

**Files:**
- Modify: `web/src/pages/posts.rs`

- [ ] **Step 1: Add `ServerAction<UnpublishPost>` to `PostPage` and update the view**

Replace the `PostPage` function:

```rust
#[component]
pub fn PostPage() -> impl IntoView {
    let delete_action = ServerAction::<DeletePost>::new();
    let unpublish_action = ServerAction::<UnpublishPost>::new();
    let params = use_params_map();

    let post_data = move || {
        let params = params.get();
        let raw_username = params.get("username").unwrap_or_default();
        let username = raw_username.strip_prefix('~').map(str::to_string);
        let year = params
            .get("year")
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or_default();
        let month = params
            .get("month")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let day = params
            .get("day")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or_default();
        let slug = params.get("slug").unwrap_or_default();
        (username, year, month, day, slug)
    };

    let post = Resource::new(
        post_data,
        |(username, year, month, day, slug): (Option<String>, i32, u32, u32, String)| async move {
            let username = match username {
                Some(value) if !value.is_empty() => value,
                _ => return Err(ServerFnError::new("Invalid permalink")),
            };
            get_post(username, year, month, day, slug).await
        },
    );

    view! {
        <Suspense fallback=|| {
            view! { <p>"Loading..."</p> }
        }>
            {move || Suspend::new(async move {
                match post.await {
                    Ok(fetched_post) => {
                        let banner = fetched_post.is_draft.then_some("Draft - visible only to you");
                        let post_id = fetched_post.post_id;
                        let is_author = fetched_post.is_author;
                        let is_draft = fetched_post.is_draft;
                        let edit_href = format!("/posts/{}/edit", post_id);
                        let article = render_post_article(fetched_post, banner);
                        view! {
                            <div class="j-post-page">
                                {article}
                                {is_author
                                    .then(|| {
                                        view! {
                                            <aside class="j-post-acts">
                                                <a href=edit_href class="j-btn">"Edit"</a>
                                                {(!is_draft)
                                                    .then(|| {
                                                        view! {
                                                            <ActionForm action=unpublish_action>
                                                                <input
                                                                    type="hidden"
                                                                    name="post_id"
                                                                    value=post_id
                                                                />
                                                                <button
                                                                    type="submit"
                                                                    class="j-btn"
                                                                    data-test="unpublish-btn"
                                                                    onclick="return confirm('Move this post back to drafts?')"
                                                                >
                                                                    "Unpublish"
                                                                </button>
                                                            </ActionForm>
                                                        }
                                                    })}
                                                {render_delete_form(
                                                    delete_action,
                                                    post_id,
                                                    "Delete this post?",
                                                )}
                                            </aside>
                                        }
                                    })}
                            </div>
                        }
                            .into_any()
                    }
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                }
            })}
        </Suspense>
        {move || {
            unpublish_action
                .value()
                .get()
                .map(|result: Result<(), ServerFnError>| match result {
                    Ok(()) => view! {
                        <p class="success" data-test="unpublish-success">
                            "Post moved to drafts. "
                            <a href="/drafts">"View drafts"</a>
                        </p>
                    }
                        .into_any(),
                    Err(err) => view! { <p class="error">{err.to_string()}</p> }.into_any(),
                })
        }}
        {render_delete_result(delete_action, "Post deleted.", "/", "Go to home")}
    }
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -40
```

- [ ] **Step 3: Write e2e test for Edit button on permalink page**

Add to `end2end/tests/posts.spec.ts`:

```typescript
test("author sees Edit and Delete on published post permalink page", async ({
  page,
}, testInfo) => {
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Actions Test\n\ncontent");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".success");

  const permalinkHref = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");
  expect(permalinkHref).toBeTruthy();

  await goto(page, permalinkHref!);

  // Author sees action buttons
  await expect(page.locator('a[href*="/edit"]')).toBeVisible();
  await expect(page.locator('button[data-test="unpublish-btn"]')).toBeVisible();
  await expect(page.locator('button[onclick*="Delete this post"]')).toBeVisible();
});

test("non-author does not see action buttons on permalink page", async ({
  page,
}, testInfo) => {
  // Create a post as one user, view as another (unauthenticated visitor)
  await register(
    page,
    hydrationHeavyFirstNavigationTimeoutMs(testInfo, 10_000),
  );

  await goto(page, "/posts/new");
  await page.fill('textarea[name="body"]', "# Visitor Test\n\ncontent");
  await click(page, 'button[name="publish"][value="true"]');
  await waitForSelector(page, ".success");

  const permalinkHref = await page
    .locator('[data-test="permalink-link"]')
    .getAttribute("href");

  // Log out
  await goto(page, "/logout");
  await waitForSelector(page, 'a[href="/login"]');

  // Visit permalink as unauthenticated
  await goto(page, permalinkHref!);

  await expect(page.locator('button[data-test="unpublish-btn"]')).not.toBeVisible();
  await expect(page.locator('a[href*="/edit"]')).not.toBeVisible();
});
```

- [ ] **Step 4: Run full test suite**

```bash
scripts/verify
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add web/src/pages/posts.rs end2end/tests/posts.spec.ts
git commit -m "feat: PostPage two-column layout with Edit, Unpublish, Delete for author"
```

---

## Self-Review

**Spec coverage:**
- ✅ Section 1: verbatim body storage, `derive_post_metadata` no longer mutates body, `title` column is derived cache
- ✅ Section 1: updated org extraction algorithm (task 1)
- ✅ Section 1: `render_post_article` checks `rendered_html` for `<h1>` before injecting (task 3)
- ✅ Section 2: `ComposerFields` shared component with segmented format control (tasks 4–8)
- ✅ Section 2: `default_format` prop as forward-compatible hook for user config
- ✅ Section 2: `InlineComposer`, `CreatePostPage`, `EditPostPage` all use `ComposerFields`
- ✅ Section 3: `is_author` on `TimelinePostSummary` (task 10)
- ✅ Section 3: `unpublish_post` storage + server function (task 9)
- ✅ Section 3: `PostCard` three-column with timestamp permalink + author actions (task 11)
- ✅ Section 3: `render_timeline_post_row` updated (task 12)
- ✅ Section 3: draft list two-column layout (task 13)
- ✅ Section 3: `PostPage` two-column with Edit/Unpublish/Delete (task 14)

**Placeholder scan:** No TBDs or incomplete steps found.

**Type consistency:**
- `UnpublishPost` is used in tasks 9, 11, 12, 14 — all reference the same server function name. ✅
- `ComposerFields` props are consistent across tasks 5, 6, 7, 8. ✅
- `timeline_post_summary(username, post, current_user_id)` signature used consistently in task 10. ✅
- `render_timeline_post_row(post, delete_action, unpublish_action)` consistent in tasks 12. ✅
