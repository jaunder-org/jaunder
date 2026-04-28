# UI Polish Design

**Date:** 2026-04-23
**Scope:** Sidebar auth-gating, inline composer improvements, timestamp formatting, home page timeline refresh.

---

## 1. Sidebar auth-gating and "Sign in" removal

**File:** `web/src/pages/ui.rs` — `Sidebar` component.

### Nav item model

Each nav item gains an `auth_required: bool` field alongside its existing `href: Option<&'static str>`:

```
(key, label, icon_path, href, auth_required)
```

Current items:

| Key        | href      | auth_required |
|------------|-----------|---------------|
| home       | Some("/")  | false         |
| local      | None      | true          |
| federated  | None      | true          |
| replies    | None      | true          |
| bookmarks  | None      | true          |
| settings   | None      | true          |

### Rendering rules

- **Unauthenticated:** render only items where `href.is_some() && !auth_required`. Today: just "Home".
- **Authenticated:** render only items where `href.is_some()`. Today: just "Home". Auth-required items with real hrefs will appear automatically as pages are implemented.

The filtering happens inside the existing `Suspense` block that already resolves `user`.

### Sidebar footer

- **Authenticated:** avatar + username + "Sign out" link (unchanged).
- **Unauthenticated:** nothing. The unauthenticated home page `Topbar` already provides "Sign in" and "Register" CTAs.

---

## 2. Timestamp formatting

**File:** `web/src/pages/ui.rs` — `format_post_time`.

Currently returns only `YYYY-MM-DD` (splits on `T` and discards the rest).

**New behaviour:** return `YYYY-MM-DD HH:MM` by slicing the date and time portions of the RFC-3339 string without adding new dependencies.

Example: `"2026-04-23T10:30:00+00:00"` → `"2026-04-23 10:30"`.

### Call sites updated

- `PostCard`: already passes `post.published_at` — gains the time portion automatically.
- `render_post_article`: currently passes `created_at`. Change to pass `published_at`, falling back to `created_at` for drafts where `published_at` is `None`.

---

## 3. `InlineComposer` improvements

**File:** `web/src/pages/ui.rs` — `InlineComposer` component.

### 3a. Remove character count display

Remove the `<span class="j-count">` element entirely. No replacement.

### 3b. Larger textarea

Add `rows="6"` to the textarea element.

### 3c. Format toggle

Replace the hidden `<input type="hidden" name="format" value="markdown" />` with:

- A `RwSignal<&'static str>` named `format`, defaulting to `"markdown"`.
- Two toggle buttons rendered in the toolbar: `[Markdown]` and `[Org]`. The active one gets the `is-primary` class; clicking the inactive one switches the signal.
- A hidden input `<input type="hidden" name="format" prop:value=format />` carries the value on submit.

### 3d. Draft saving

Add a "Save draft" button alongside the existing "Publish" button. It submits the form with `name="publish" value="false"`. The "Publish" button retains `value="true"`.

### 3e. Flash message: auto-dismiss linked messages

Replace the `create_action.value()` inline display with a dedicated flash signal:

```rust
let flash = RwSignal::new(Option::<(String, String)>::None); // (message, href)
```

On action success:
- Published post: `flash.set(Some(("Post published!".into(), permalink)))`.
- Draft saved: `flash.set(Some(("Draft saved.".into(), preview_url)))`.

The flash renders as `<a href=href>{message}</a>`.

**Auto-dismiss:** on success, schedule a 30-second `set_timeout` (WASM only) that calls `flash.set(None)`.

**Dismiss on typing:** the `on:input` handler on the textarea, in addition to updating `body`, also calls `flash.set(None)`.

### 3f. New prop: `on_publish: WriteSignal<u32>`

`InlineComposer` receives a new required prop:

```rust
on_publish: WriteSignal<u32>
```

On a **successful publish** (i.e. the result has a `permalink`, meaning `published_at.is_some()`), increment this signal. Draft saves do **not** increment it, since drafts do not appear in the timeline.

---

## 4. `HomePage` timeline refresh

**File:** `web/src/pages/home.rs` — `HomePage` component.

### Refresh version signal

```rust
let refresh_version = RwSignal::new(0u32);
```

Passed to `InlineComposer` as `on_publish=refresh_version.write_only()`.

### Timeline resource

The current `spawn_local` pattern is replaced with a `Resource` keyed on `(auth_state, refresh_version)`:

```rust
let initial_page = Resource::new(
    move || (auth_state.get(), refresh_version.get()),
    |(auth_state, _)| async move { /* fetch based on auth_state */ },
);
```

`auth_state` is a signal derived from a `current_user()` call, replacing the ad-hoc `spawn_local` auth check. The resource resolves to the first page of posts (feed or local depending on auth state).

### Pagination

The "load more" cursor signals (`next_cursor_created_at`, `next_cursor_post_id`) and `has_more` remain as `RwSignal`s. They are reset when the resource re-fetches (i.e. when `refresh_version` increments). The "load more" button appends to the `timeline` signal as before. This design preserves infinite-scroll as a future option.

### Draft saves

Incrementing `refresh_version` is not triggered by draft saves. The home page timeline only shows published posts.

---

## Files changed

| File | Changes |
|------|---------|
| `web/src/pages/ui.rs` | `Sidebar` auth-gating; `format_post_time` time portion; `InlineComposer` all improvements |
| `web/src/pages/home.rs` | Timeline `Resource` refactor; `refresh_version` signal; pass `on_publish` to `InlineComposer` |
