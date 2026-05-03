# M7 Media UI Supplement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make M7 media features actually usable: an in-page JS upload widget on the create/edit post pages (uploads without navigation, shows URL for insertion), a nav link to the media manage page, and an upload button on the manage page too.

**Architecture:** A reusable `MediaUploadButton` Leptos component (`web/src/pages/upload.rs`) uses `web-sys` fetch + `wasm-bindgen-futures` to POST multipart/form-data to the existing `/media/upload` endpoint and surface the result URL without a page reload. The component is used in two places: the post composer aside panel (primary use case) and the media manage page. The upload handler requires no changes — it already returns JSON 201 with the URL. Wire up the missing `/media` route and sidebar nav link.

**Tech Stack:** Rust, Leptos 0.8, web-sys (File, FormData, Request, Response), wasm-bindgen-futures (JsFuture + spawn_local), serde_json (JSON parsing), leptos_router.

---

## File Structure

- Create: `web/src/pages/upload.rs` — `MediaUploadButton` component
- Modify: `web/Cargo.toml` — add web-sys features, wasm-bindgen-futures, serde_json
- Modify: `Cargo.toml` (workspace) — add wasm-bindgen-futures, serde_json if not present
- Modify: `web/src/pages/mod.rs` — add `pub mod upload`, import `MediaUploadButton` and `MediaPage`, register `/media` route
- Modify: `web/src/pages/ui.rs` — add `Icons::MEDIA`, add "Media" to `NAV_ITEMS`
- Modify: `web/src/pages/posts.rs` — add `MediaUploadButton` to the compose aside
- Modify: `web/src/pages/media.rs` — add `MediaUploadButton` at top, wire refresh

---

## Task 1: Dependency Setup

**Files:**
- Modify: `Cargo.toml` (workspace)
- Modify: `web/Cargo.toml`

- [ ] **Step 1.1: Add workspace deps if missing**

In the root `Cargo.toml` `[workspace.dependencies]` section, add:

```toml
wasm-bindgen-futures = "0.4"
serde_json = "1"
```

(Check first — `serde_json` may already be present for other crates. Only add entries that are missing.)

- [ ] **Step 1.2: Update `web/Cargo.toml`**

Add new deps under `[dependencies]`:

```toml
wasm-bindgen-futures = { workspace = true }
serde_json = { workspace = true }
```

Extend the existing `web-sys` entry with the features required for file upload and fetch:

```toml
web-sys = { workspace = true, features = [
    "Window",
    "Location",
    "Storage",
    "File",
    "FileList",
    "FormData",
    "HtmlInputElement",
    "Request",
    "RequestInit",
    "RequestMode",
    "Response",
] }
```

- [ ] **Step 1.3: Verify it compiles**

```bash
cargo build -p web --features ssr
```

Expected: compiles without errors.

- [ ] **Step 1.4: Commit**

```bash
git add Cargo.toml web/Cargo.toml Cargo.lock
git commit -m "M7-supplement.1: add web-sys fetch/file features and wasm-bindgen-futures to web crate"
```

---

## Task 2: `MediaUploadButton` Component

**Files:**
- Create: `web/src/pages/upload.rs`

The component renders a file input and an "Attach media" button. On the client (WASM), clicking "Attach media" opens the file picker; when a file is chosen, it is immediately uploaded via `fetch`. On success the caller receives the URL via the `on_uploaded` callback. The component tracks uploading state and surfaces errors inline.

- [ ] **Step 2.1: Write `web/src/pages/upload.rs`**

```rust
use leptos::prelude::*;

/// A button that lets the user pick a file and immediately uploads it to
/// `/media/upload` via JavaScript fetch (no page navigation).
///
/// `on_uploaded` is called with the media URL string on success.
/// `on_error` is called with a human-readable message on failure.
///
/// The component is a no-op on SSR (renders the file input but does nothing on
/// the server — upload logic only runs after WASM hydration).
#[allow(clippy::must_use_candidate)]
#[component]
pub fn MediaUploadButton(
    /// Called with the `/media/upload/...` URL when the upload succeeds.
    #[prop(into)]
    on_uploaded: Callback<String>,
    /// Called with an error message when the upload fails.
    #[prop(into, optional)]
    on_error: Option<Callback<String>>,
) -> impl IntoView {
    let uploading = RwSignal::new(false);
    let file_input = NodeRef::<leptos::html::Input>::new();

    // Trigger the hidden file-picker when the button is clicked.
    let open_picker = move |_| {
        if let Some(input) = file_input.get() {
            input.click();
        }
    };

    // Called when the user picks a file from the OS dialog.
    let on_file_change = move |_ev: leptos::ev::Event| {
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use wasm_bindgen_futures::JsFuture;

            let Some(input) = file_input.get() else {
                return;
            };
            // Cast the NodeRef element to HtmlInputElement to access .files().
            let input_el: web_sys::HtmlInputElement = input.unchecked_into();
            let Some(files) = input_el.files() else {
                return;
            };
            let Some(file) = files.get(0) else {
                return;
            };

            let form_data = match web_sys::FormData::new() {
                Ok(fd) => fd,
                Err(_) => return,
            };
            if form_data.append_with_blob("file", &file).is_err() {
                return;
            }

            uploading.set(true);

            spawn_local(async move {
                let result = upload_file(form_data).await;
                uploading.set(false);
                match result {
                    Ok(url) => on_uploaded.run(url),
                    Err(msg) => {
                        if let Some(cb) = on_error {
                            cb.run(msg);
                        }
                    }
                }
            });
        }

        // SSR / non-WASM: nothing to do.
        #[cfg(not(target_arch = "wasm32"))]
        let _ = _ev;
    };

    view! {
        // Hidden real file input — triggered programmatically.
        <input
            type="file"
            node_ref=file_input
            style="display:none"
            on:change=on_file_change
        />
        <button
            type="button"
            class="j-btn"
            disabled=move || uploading.get()
            on:click=open_picker
        >
            {move || if uploading.get() { "Uploading\u{2026}" } else { "Attach media" }}
        </button>
    }
}

/// Performs the actual fetch upload on the WASM target.
/// Returns the media URL string on success, or an error message string.
#[cfg(target_arch = "wasm32")]
async fn upload_file(form_data: web_sys::FormData) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().ok_or("no window")?;

    let mut opts = web_sys::RequestInit::new();
    opts.method("POST");
    opts.body(Some(&form_data));

    let request =
        web_sys::Request::new_with_str_and_init("/media/upload", &opts).map_err(|e| {
            e.as_string()
                .unwrap_or_else(|| "failed to build request".to_string())
        })?;

    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| "network error".to_string()))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "unexpected response type".to_string())?;

    if !resp.ok() {
        return Err(format!("upload failed (HTTP {})", resp.status()));
    }

    let text_promise = resp.text().map_err(|_| "failed to read response body")?;
    let text_value = JsFuture::from(text_promise)
        .await
        .map_err(|_| "failed to await response text")?;

    let body: String = text_value
        .as_string()
        .ok_or_else(|| "response body is not a string".to_string())?;

    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| "invalid JSON in response".to_string())?;

    parsed["url"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "response JSON missing 'url' field".to_string())
}
```

- [ ] **Step 2.2: Register the module and export in `mod.rs`**

In `web/src/pages/mod.rs`, add:

```rust
pub mod upload;
```

Add `MediaUploadButton` to the existing `pub use ui::{...}` line, or add a separate:

```rust
pub use upload::MediaUploadButton;
```

- [ ] **Step 2.3: Build to verify it compiles**

```bash
cargo build -p web --features ssr
```

Expected: no errors. (WASM-specific code is gated and won't be compiled for SSR.)

- [ ] **Step 2.4: Commit**

```bash
git add web/src/pages/upload.rs web/src/pages/mod.rs
git commit -m "M7-supplement.2: add MediaUploadButton component with JS fetch upload"
```

---

## Task 3: Wire Upload Button into Post Composer

**Files:**
- Modify: `web/src/pages/posts.rs`

The compose aside already has an "Options" section. Add a "Media" section below it with `MediaUploadButton`. When a file is uploaded, show the resulting URL in a read-only text input so the user can copy-paste it into their post body.

- [ ] **Step 3.1: Add upload state signals to `CreatePostPage`**

In `CreatePostPage`, add two signals near the top of the function:

```rust
let last_media_url = RwSignal::new(Option::<String>::None);
let upload_error = RwSignal::new(Option::<String>::None);
```

- [ ] **Step 3.2: Add the `MediaUploadButton` to the compose aside**

Import at the top of `posts.rs`:

```rust
use crate::pages::MediaUploadButton;
```

Inside the `j-compose-aside` `<aside>`, after the format buttons block, add:

```rust
<div style="margin-top:16px">
    <div class="j-sb-head" style="padding:0 0 10px">"Media"</div>
    <MediaUploadButton
        on_uploaded=Callback::new(move |url: String| {
            last_media_url.set(Some(url));
            upload_error.set(None);
        })
        on_error=Callback::new(move |msg: String| {
            upload_error.set(Some(msg));
        })
    />
    {move || last_media_url.get().map(|url| view! {
        <div style="margin-top:8px">
            <div style="font-size:12px;color:#888;margin-bottom:4px">"Uploaded URL:"</div>
            <input
                type="text"
                readonly
                value=url.clone()
                class="j-field-val"
                style="font-size:12px;cursor:text"
                on:click=move |ev| {
                    use wasm_bindgen::JsCast;
                    let _ = ev.target()
                        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                        .map(|i| i.select());
                }
            />
        </div>
    })}
    {move || upload_error.get().map(|msg| view! {
        <p class="error" style="margin-top:6px;font-size:12px">{msg}</p>
    })}
</div>
```

- [ ] **Step 3.3: Repeat for `EditPostPage`**

`EditPostPage` also has a `j-compose-aside`. Apply the same pattern: add `last_media_url` and `upload_error` signals, add the same Media section to the aside.

- [ ] **Step 3.4: Run `scripts/verify`**

```bash
scripts/verify
```

Expected: all checks pass. Fix any clippy warnings before continuing.

- [ ] **Step 3.5: Commit**

```bash
git add web/src/pages/posts.rs
git commit -m "M7-supplement.3: add media upload widget to post composer aside"
```

---

## Task 4: Wire Upload Button into Media Manage Page + Route + Nav

**Files:**
- Modify: `web/src/pages/media.rs`
- Modify: `web/src/pages/mod.rs`
- Modify: `web/src/pages/ui.rs`

### Step 4.1 — Add upload to `MediaPage`

In `web/src/pages/media.rs`:

Add import:
```rust
use crate::pages::MediaUploadButton;
```

Add an `upload_version` signal that the media list and usage resources also depend on, so they refresh after an upload:

```rust
let upload_version = RwSignal::new(0u32);
```

Update both `Resource::new` calls to include `upload_version` in their keys:

```rust
let usage = Resource::new(
    move || (delete_action.version().get(), upload_version.get()),
    |_| media_usage(),
);
let media_list = Resource::new(
    move || (delete_action.version().get(), upload_version.get()),
    |_| list_my_media(None, Some(50), Some(0)),
);
```

Add the upload section at the top of the `<div style="padding:16px 32px">` block, before the storage usage `<Suspense>`:

```rust
<div class="j-sb-head" style="margin-bottom:8px">"Upload"</div>
<div style="margin-bottom:24px">
    <MediaUploadButton
        on_uploaded=Callback::new(move |_url: String| {
            upload_version.update(|v| *v += 1);
        })
        on_error=Callback::new(move |msg: String| {
            // Errors surface inline within the button; no extra handling needed here.
            leptos::logging::warn!("upload error: {msg}");
        })
    />
</div>
```

### Step 4.2 — Add `Icons::MEDIA` to `ui.rs`

In `web/src/pages/ui.rs`, in the `Icons` impl block, add after `SHIELD`:

```rust
pub const MEDIA: &'static str =
    "M3 5h14v10H3z M7 9a1 1 0 1 0 0-2 1 1 0 0 0 0 2z M5 13l3-3 2 2 3-3 5 5H3z";
```

### Step 4.3 — Add "Media" to `NAV_ITEMS` in `ui.rs`

Replace the existing `NAV_ITEMS` constant (around line 554):

```rust
const NAV_ITEMS: &[(&str, &str, &str, Option<&'static str>, bool)] = &[
    ("home",      "Home",      Icons::HOME,     Some("/"),       false),
    ("local",     "Local",     Icons::LOCAL,    None,            true),
    ("federated", "Federated", Icons::FED,      None,            true),
    ("replies",   "Replies",   Icons::REPLY,    None,            true),
    ("bookmarks", "Bookmarks", Icons::BOOKMARK, None,            true),
    ("drafts",    "Drafts",    Icons::EDIT,     Some("/drafts"), true),
    ("media",     "Media",     Icons::MEDIA,    Some("/media"),  true),
    ("settings",  "Settings",  Icons::COG,      None,            true),
];
```

### Step 4.4 — Register route and import in `mod.rs`

In `web/src/pages/mod.rs`:

1. Add module declaration with the other `pub mod` entries:
   ```rust
   pub mod media;
   ```

2. Add import with the other page imports:
   ```rust
   use crate::pages::media::MediaPage;
   ```

3. Add route in the `<ParentRoute>` block, after the `drafts` route:
   ```rust
   <Route path=StaticSegment("media") view=MediaPage />
   ```

- [ ] **Step 4.5: Run `scripts/verify`**

```bash
scripts/verify
```

Expected: all checks pass. Fix any compilation or clippy errors.

- [ ] **Step 4.6: Commit**

```bash
git add web/src/pages/media.rs web/src/pages/ui.rs web/src/pages/mod.rs
git commit -m "M7-supplement.4: add upload button to media page, wire /media route and sidebar nav"
```

---

## Task 5: Tests

**Files:**
- Modify: `end2end/tests/media.spec.ts`

The existing e2e tests cover API-level upload (Task 11 of the original plan). This task adds UI-level tests.

- [ ] **Step 5.1: Add e2e tests for upload widget and nav link**

Add to `end2end/tests/media.spec.ts`, inside the existing `test.describe` block:

```typescript
test("media nav link appears for authenticated users", async ({
    page,
}, testInfo) => {
    await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 30000));
    await waitForSelector(page, "a[href='/media']");
});

test("media manage page is reachable via nav link", async ({
    page,
}, testInfo) => {
    await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 30000));
    await click(page, "a[href='/media']");
    await waitForSelector(page, "input[type='file']");
});

test("upload widget on create-post page uploads file and shows URL", async ({
    page,
}, testInfo) => {
    await register(page, hydrationHeavyFirstNavigationTimeoutMs(testInfo, 30000));
    await goto(page, "/posts/new");

    // Click the "Attach media" button to trigger the file picker, but use
    // setInputFiles on the hidden file input to bypass the OS dialog.
    const fileInput = page.locator("input[type='file']").first();
    await fileInput.setInputFiles({
        name: "test-image.png",
        mimeType: "image/png",
        buffer: Buffer.from("fake png content"),
    });

    // The upload should complete and show the URL.
    const urlInput = page.locator("input[readonly]");
    await urlInput.waitFor({ state: "visible", timeout: 10000 });
    const url = await urlInput.inputValue();
    expect(url).toContain("/media/upload/");
});
```

Ensure the imports at the top of the file include `click` and `waitForSelector`:

```typescript
import {
    test,
    expect,
    hydrationHeavyFirstNavigationTimeoutMs,
} from "./fixtures";
import { BASE_URL, goto, register, click, waitForSelector } from "./helpers";
```

- [ ] **Step 5.2: Run `scripts/verify`**

```bash
scripts/verify
```

Expected: all checks pass.

- [ ] **Step 5.3: Commit**

```bash
git add end2end/tests/media.spec.ts
git commit -m "M7-supplement.5: add e2e tests for media nav link and upload widget"
```
