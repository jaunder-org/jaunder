# Spec — #516: `client` navigation + confirm-dialog primitives for `web`'s direct window calls

- Issue: [#516](https://github.com/jaunder-org/jaunder/issues/516)
- Milestone: client crate: browser glue out of web (M14)
- Governing ADR: [ADR-0069](../../adr/0069-client-crate-wasm-only-home.md)
  (client charter — wasm-only raw browser glue, no domain types),
  [ADR-0070](../../adr/0070-web-vertical-wasm-only-component-files.md)
  (component.rs is wasm-only, so call sites need no new cfg).
- Predecessor: #514 (`client::storage`), #515 (`client::reactive`) — the same
  relocation pattern; this adds the navigation/dialog primitives.
- Date: 2026-07-22

## Problem

`web` makes direct `web_sys::window()` browser calls for navigation and confirm
dialogs — the last of its raw `window().location()` / `confirm_with_message`
glue. Milestone 14 relocates raw browser infrastructure into the wasm-only
`client` crate. These are pure browser primitives (no domain types, no reactive
plumbing), so they belong in `client`; `web` keeps the surrounding logic (which
URL, which message, what happens on confirm).

Current sites (all wasm-only — `posts/component.rs` and `pages/mod.rs` are gated
wasm-only, ADR-0070/the `pages` module cfg):

- **Navigation — `window().location().replace(…)` ×4:** `posts/component.rs:239`
  (post-publish → canonical permalink), `posts/component.rs:1231` (unpublish →
  `/drafts`), `posts/component.rs:1581` (post-update → updated permalink),
  `pages/mod.rs:80` (a redirect to `loc`). The permalink args are typed
  `RootRelativeUrl` (#560).
- **Reload — `href()` + `replace(href)`:** `posts/component.rs:1212-1214`
  reloads the current URL so the server handles a non-SPA route (e.g.
  `/media/…`).
- **Confirm — `confirm_with_message(…)` ×2:** `posts/component.rs:254`
  (publish), `posts/component.rs:296` (delete), each as
  `window().and_then(|w| w.confirm_with_message(msg).ok()).unwrap_or(false)`.

## Out of scope (sibling M14 issues, not nav/dialog)

- `web/src/media/component.rs`'s `FormData` / `Request` / `fetch` / `Response`
  glue — the upload-fetch primitive, **#517**.
- `web/src/lib.rs:79,98`'s `web_sys::window()` DOM-seed reads (theme/seed JSON)
  — the CSR-boot relocation, **#519**.

These are deliberately untouched; #516 covers only `location()` navigation and
`confirm` dialogs.

## Decision

Two small `client` modules — raw browser infrastructure, no domain types,
**unconditional** (like `client::storage`; they use only `web-sys`, no `leptos`,
so they need no `csr` feature gate). Each primitive is behaviour-preserving with
the code it replaces (including the `if let Some(window)` guard and the
swallowed `Result`s).

### `client::navigation`

```rust
/// Replace the current history entry with `url` (browser `location.replace`). No-op
/// off-DOM (no window); the navigation `Result` is swallowed, matching the call sites.
pub fn replace(url: impl AsRef<str>) {
    if let Some(window) = web_sys::window() {
        let _ = window.location().replace(url.as_ref());
    }
}

/// Reload the current URL by replacing it with itself — forces a full server
/// round-trip (used to hand a non-SPA route back to the server). No-op off-DOM.
pub fn reload() {
    if let Some(window) = web_sys::window() {
        let location = window.location();
        if let Ok(href) = location.href() {
            let _ = location.replace(&href);
        }
    }
}
```

`replace` takes `impl AsRef<str>` so `web` passes a `RootRelativeUrl` permalink
or a `&str` path directly, no `.as_ref()` at the call site, while `client` stays
domain-type-free.

### `client::dialog`

```rust
/// Show a native confirm dialog; `true` only if the user confirmed. `false` off-DOM
/// or if the dialog can't be shown — matching the current `unwrap_or(false)`.
pub fn confirm(message: &str) -> bool {
    web_sys::window()
        .and_then(|w| w.confirm_with_message(message).ok())
        .unwrap_or(false)
}
```

### Wiring

- `client/Cargo.toml`: add `"Location"` to the `web-sys` features (`"Window"` is
  already present and carries `confirm_with_message`). No new crate deps.
- `client/src/lib.rs`: `pub mod navigation;` + `pub mod dialog;` (unconditional,
  alongside `pub mod storage;`).
- Rewrite the `web` call sites to the primitives, dropping the
  `web_sys::window()` guards and imports:
  - `posts/component.rs:238-240` →
    `client::navigation::replace(&published.permalink);`
  - `posts/component.rs:253-255` / `:295-297` →
    `client::dialog::confirm("Publish this draft?")` /
    `client::dialog::confirm("Delete this post?")`
  - `posts/component.rs:1212-1216` → `client::navigation::reload();`
  - `posts/component.rs:1230-1232` → `client::navigation::replace("/drafts");`
  - `posts/component.rs:1580-1582` →
    `client::navigation::replace(&updated.permalink);`
  - `pages/mod.rs:79-81` → `client::navigation::replace(loc);`

## Acceptance criteria

1. **Primitives exist.** `client::navigation::{replace, reload}` and
   `client::dialog::confirm` exist with the signatures above; `replace` takes
   `impl AsRef<str>`, `confirm` returns `bool`.
2. **No `window().location()` / `confirm_with_message` in `web`.**
   `rg '\.location\(\)|confirm_with_message' web/src` returns nothing (the `\.`
   excludes leptos's `use_location()` router hook; the #517/#519 `web_sys` uses
   are `FormData`/`fetch`/DOM-seed, not `location`/`confirm`).
3. **All 7 sites rewritten.** The 4 `replace`, 1 `reload`, and 2 `confirm` sites
   call the `client` primitives; the `web_sys::window()` guards there are gone.
4. **`client` wiring.** `web-sys` gains the `"Location"` feature; `navigation`
   and `dialog` are unconditional modules (no `csr` gate), and `client` gains
   **no** `leptos` or domain-type dependency for them.
5. **Out-of-scope untouched.** `media/component.rs` fetch glue and `lib.rs`
   DOM-seed reads are unchanged (they belong to #517/#519).
6. **Behaviour preserved.** Each primitive matches the replaced code: `replace`/
   `reload` swallow the `Result` and no-op off-DOM; `confirm` returns `false`
   off-DOM. The post publish/unpublish/delete redirects and the confirm gating
   are exercised by the existing posts e2e in CI.
7. **Gate green.** `cargo xtask validate --no-e2e` passes; the csr wasm build
   compiles the rewritten call sites.

## Out of scope (design)

- Any typed-URL / newtype work on the permalink args beyond passing them through
  `impl AsRef<str>`.
- The media-fetch (#517), DOM-seed / CSR-boot (#519), and `js_sys::Date` (#518)
  browser glue — separate M14 issues.
- Retiring the `#[client_only]` macro / the endgame ratchet (#520).
