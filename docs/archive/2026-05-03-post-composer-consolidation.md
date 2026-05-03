# Post Composer Consolidation

**Goal:** Eliminate duplication between `InlineComposer` (home page) and `CreatePostPage` by extracting a single `PostCreateForm` component that handles all post-creation logic. Also extract the repeated media widget state/UI into a `MediaPanel` component used by all three composer surfaces (create inline, create full-page, edit).

**Problem:** `InlineComposer` (`web/src/pages/ui.rs`) and `CreatePostPage` (`web/src/pages/posts.rs`) each independently own `ServerAction::<CreatePost>`, `body`/`format` signals, submit buttons, and (post-M7-supplement) `MediaUploadButton`. Changes to one must be manually mirrored to the other. This is what caused the media widget to be missing from the home page.

**Scope:**
- `InlineComposer` and `CreatePostPage` share a new `PostCreateForm` component.
- `EditPostPage` uses `UpdatePost` (different action) so it cannot share the create form, but it shares the new `MediaPanel` component.
- No changes to server functions, routing, CSS classes, or storage.

---

## Components after refactor

### `MediaPanel` (new, in `web/src/pages/upload.rs`)

Owns `last_media_url` and `upload_error` signals. Renders `MediaUploadButton`, the uploaded-URL readonly input, and the error paragraph. No props. Self-contained.

Used by: `PostCreateForm` (inside the aside) and `EditPostPage` (inside its aside).

### `PostCreateForm` (new, in `web/src/pages/ui.rs`)

Owns `ServerAction::<CreatePost>`, `body`, `format` signals. Renders the full `ActionForm`. Calls `on_success` callback when a post is created. Clears `body` on success.

Props:
- `compact: bool` — selects layout:
  - `true`: `j-composer` / `j-composer-row` with `<Avatar>` (home page inline)
  - `false`: `j-compose-grid` with `j-compose-body` + `j-compose-aside` (full-page)
- `username: Option<String>` — avatar label; only used when `compact=true`
- `on_success: Callback<CreatePostResult>` — called after a successful create
- `rows: usize` — textarea row count (default: 6 for compact, 16 for full)
- `placeholder: &'static str` — textarea placeholder

The aside (compact=false only) contains: slug override input, format toggle (Markdown / Org), `MediaPanel`, Save draft + Publish buttons.

The compact layout contains: `ComposerFields`, Save draft + Publish buttons (in `j-composer-toolbar`), `MediaPanel` (below toolbar).

### `InlineComposer` (simplified wrapper, stays in `web/src/pages/ui.rs`)

Keeps its `flash` signal and flash/error display. Delegates the form entirely to `PostCreateForm compact=true`. Its `on_success` callback: sets flash, starts 30-second timeout to clear it, calls `on_publish` if the post was published.

### `CreatePostPage` (simplified, stays in `web/src/pages/posts.rs`)

Keeps its `last_result` signal and the success panel display. Delegates the form entirely to `PostCreateForm compact=false`. Its `on_success` callback: sets `last_result`.

### `EditPostPage` (minor change, stays in `web/src/pages/posts.rs`)

Replaces its inline media widget (signals + `MediaUploadButton` + URL display + error) with `<MediaPanel />`. No other changes.

---

## Tasks

### Task 1: Extract `MediaPanel`

**Files:** `web/src/pages/upload.rs`, `web/src/pages/mod.rs`

- [ ] **1.1** In `web/src/pages/upload.rs`, add `MediaPanel` below `MediaUploadButton`:

```rust
/// Self-contained media upload widget: button, uploaded-URL display, and error.
/// Drop this into any `ActionForm` aside that needs media upload.
#[allow(clippy::must_use_candidate)]
#[component]
pub fn MediaPanel() -> impl IntoView {
    let last_media_url = RwSignal::new(Option::<String>::None);
    let upload_error = RwSignal::new(Option::<String>::None);

    view! {
        <MediaUploadButton
            on_uploaded=Callback::new(move |url: String| {
                last_media_url.set(Some(url));
                upload_error.set(None);
            })
            on_error=Callback::new(move |msg: String| {
                upload_error.set(Some(msg));
            })
        />
        {move || {
            last_media_url.get().map(|url| {
                view! {
                    <div style="margin-top:8px">
                        <div style="font-size:12px;color:#888;margin-bottom:4px">
                            "Uploaded URL:"
                        </div>
                        <input
                            type="text"
                            readonly
                            value=url.clone()
                            class="j-field-val"
                            style="font-size:12px;cursor:text"
                            on:click=move |ev| {
                                use leptos::wasm_bindgen::JsCast;
                                let _ = ev
                                    .target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                    .map(|i| i.select());
                            }
                        />
                    </div>
                }
            })
        }}
        {move || {
            upload_error.get().map(|msg| {
                view! {
                    <p class="error" style="margin-top:6px;font-size:12px">{msg}</p>
                }
            })
        }}
    }
}
```

- [ ] **1.2** In `web/src/pages/mod.rs`, add `pub use upload::MediaPanel;` alongside the existing `pub use upload::MediaUploadButton;`.

- [ ] **1.3** In `web/src/pages/posts.rs` (`EditPostPage`): replace the `last_media_url`/`upload_error` signals and the inline media block with `<MediaPanel />`. Remove the now-unused signal declarations. Keep the `use crate::pages::MediaUploadButton;` import replaced with `use crate::pages::MediaPanel;`.

- [ ] **1.4** Run `cargo build -p web --features ssr`. Fix any errors.

- [ ] **1.5** Run `scripts/verify`. Fix any issues.

- [ ] **1.6** Commit:
  ```
  refactor: extract MediaPanel component, use in EditPostPage
  ```

---

### Task 2: Extract `PostCreateForm`

**Files:** `web/src/pages/ui.rs`, `web/src/pages/posts.rs`

- [ ] **2.1** In `web/src/pages/ui.rs`, add `use crate::pages::upload::MediaPanel;` to the imports.

- [ ] **2.2** Add `PostCreateForm` component in `web/src/pages/ui.rs` (place it just before `InlineComposer`):

The component owns:
- `ServerAction::<CreatePost>`
- `body: RwSignal<String>`
- `format: RwSignal<String>`

On success (after action value changes to `Ok`): calls `on_success` callback, then clears `body`.

Layout when `compact=false` (`j-compose-grid`):
- Left column (`j-compose-body`): `ComposerFields` (rows, placeholder)
- Right column (`j-compose-aside`):
  - Options section: slug override input, format toggle buttons
  - Media section heading + `<MediaPanel />`
  - Save draft + Publish buttons (`margin-top:auto`)

Layout when `compact=true` (`j-composer`):
- `j-composer-row`: `<Avatar name=username size=36>` + `j-composer-body`
  - `ComposerFields` (rows, placeholder), hidden slug_override input
  - `j-composer-toolbar`: Save draft + Publish buttons (disabled when body empty)
  - `<MediaPanel />` below toolbar

Error display: below the `ActionForm` in both layouts.

Props signature:
```rust
#[component]
pub fn PostCreateForm(
    compact: bool,
    #[prop(optional)] username: Option<String>,
    #[prop(into)] on_success: Callback<CreatePostResult>,
    #[prop(default = 6)] rows: usize,
    #[prop(default = "What\u{2019}s on your mind?")] placeholder: &'static str,
) -> impl IntoView
```

- [ ] **2.3** Refactor `InlineComposer` to delegate to `PostCreateForm`:

```rust
#[component]
pub fn InlineComposer(username: String, on_publish: WriteSignal<u32>) -> impl IntoView {
    let flash: RwSignal<Option<(String, String)>> = RwSignal::new(None);

    #[cfg(not(target_arch = "wasm32"))]
    let _ = on_publish;

    let on_success = Callback::new(move |created: CreatePostResult| {
        let url = created.permalink.unwrap_or(created.preview_url);
        let msg = if created.published_at.is_some() {
            "Post published!".to_string()
        } else {
            "Draft saved!".to_string()
        };
        flash.set(Some((url, msg)));
        #[cfg(target_arch = "wasm32")]
        {
            use leptos_dom::helpers::set_timeout;
            use std::time::Duration;
            set_timeout(move || flash.set(None), Duration::from_secs(30));
            if created.published_at.is_some() {
                on_publish.update(|v| *v += 1);
            }
        }
    });

    view! {
        <PostCreateForm
            compact=true
            username=username
            on_success=on_success
            rows=6
            placeholder="What\u{2019}s on your mind?"
        />
        {move || {
            if let Some((url, msg)) = flash.get() {
                return view! {
                    <p class="success"><a href=url>{msg}</a></p>
                }.into_any();
            }
            ().into_any()
        }}
    }
}
```

Note: the existing `InlineComposer` renders the error from the action; `PostCreateForm` handles that internally now.

- [ ] **2.4** Refactor `CreatePostPage` in `posts.rs` to use `PostCreateForm`:

Remove: `create_post_action`, `body`, `format`, `last_media_url`, `upload_error` signals and the `ActionForm` block. Keep the `Suspense`/`current_user` check wrapper and the success panel display.

```rust
let last_result: RwSignal<Option<CreatePostResult>> = RwSignal::new(None);

// inside the Ok(Some(_)) branch:
view! {
    <PostCreateForm
        compact=false
        rows=16
        placeholder="Write something\u{2026}"
        on_success=Callback::new(move |created| last_result.set(Some(created)))
    />
    {move || last_result.get().map(|created| view! { /* existing success panel */ })}
}
```

Update imports in `posts.rs`: remove `MediaUploadButton`, add `PostCreateForm` from `crate::pages::ui`. Remove `use crate::pages::MediaPanel` if it was added in Task 1 (no longer needed in posts.rs if EditPostPage uses it via `crate::pages::MediaPanel`).

- [ ] **2.5** Run `cargo build -p web --features ssr`. Fix any errors.

- [ ] **2.6** Run `scripts/verify`. Fix any issues.

- [ ] **2.7** Commit:
  ```
  refactor: extract PostCreateForm, unify InlineComposer and CreatePostPage
  ```

---

### Task 3: Update e2e tests

**Files:** `end2end/tests/media.spec.ts`, `end2end/tests/posts.spec.ts` (if needed)

- [ ] **3.1** Add to `end2end/tests/media.spec.ts` inside the describe block:

```typescript
test("upload widget on home page uploads file and shows URL", async ({
  page,
}, testInfo) => {
  test.setTimeout(hydrationHeavyTimeoutMs(testInfo, 30_000));
  await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 30000));
  // Home page shows InlineComposer after login, which now includes MediaPanel.
  const fileInput = page.locator(".j-composer input[type='file']").first();
  await fileInput.setInputFiles({
    name: "home-image.png",
    mimeType: "image/png",
    buffer: Buffer.from("fake png content for home"),
  });
  await page.locator(".j-composer input[readonly]").waitFor({ state: "visible", timeout: 10000 });
  const url = await page.locator(".j-composer input[readonly]").inputValue();
  expect(url).toContain("/media/upload/");
});
```

- [ ] **3.2** Run `scripts/verify`. Fix any issues.

- [ ] **3.3** Commit:
  ```
  test: add e2e test for media upload widget on home page
  ```
