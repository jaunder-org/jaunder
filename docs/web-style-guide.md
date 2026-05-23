# Web Component Style Guide

How page components in `web/src/pages/` and the shared widgets in
`web/src/pages/ui.rs` should be structured so that pages look and feel
the same.

This guide is **descriptive of the design system we already have**
(`server/assets/jaunder.css` + the `Topbar` / `PostCard` / `PostCreateForm`
helpers). Follow it when adding a new page; don't re-invent local
variants.

---

## 1. Page chrome

Every full-window page renders, in order:

```rust
view! {
    <Topbar title="…".to_string() sub="…".to_string() />
    <div class="j-scroll">
        <div class="j-page">
            // page body
        </div>
    </div>
}
```

- **`Topbar`** lives in `web/src/pages/ui.rs`. Do not write a bare
  `<h1>"Title"</h1>` at the top of a page — that is the legacy style
  and should be migrated.
- `title` is required; `sub` is optional but should describe the page
  ("Operations", "Your uploads", "Unpublished posts"). **Don't pass
  `sub=String::new()`** — omit the prop instead, which suppresses the
  `j-sub` line.
- `Topbar` accepts `children` for right-aligned actions (sign-in
  buttons, primary CTA). See `home.rs` Local mode for the pattern.
- Both `title` and `sub` accept either a static `String` or a closure
  (`move || …`) for reactive content — see `UserTimelinePage` for the
  closure form.
- The outer `<div class="j-scroll">` is the scrollable region; the
  inner padded div is the gutter. Pages that follow a dense
  card-grid layout (Backup, settings-style forms) may use
  `<div class="j-settings">` instead — it provides the same padding plus
  a max-width.

## 2. Suspense and loading placeholders

- Always use `<p class="j-loading">"Loading\u{2026}"</p>` (U+2026
  ellipsis, not three ASCII dots) inside a `Suspense` fallback.
- Loading text in button labels follows the same convention:
  `"Loading\u{2026}"`, not `"Loading..."`.

```rust
<Suspense fallback=|| view! { <p class="j-loading">"Loading\u{2026}"</p> }>
    {move || Suspend::new(async move { /* … */ })}
</Suspense>
```

## 3. Flash messages

Every server action result must render as a single styled paragraph.

- **Success:** `<p class="success">"Done."</p>`
- **Error:**   `<p class="error">{e.to_string()}</p>`

```rust
{move || {
    action.value().get().map(|r: Result<T, WebError>| match r {
        Ok(_)   => view! { <p class="success">"…"</p> }.into_any(),
        Err(e)  => view! { <p class="error">{e.to_string()}</p> }.into_any(),
    })
}}
```

- **Do not** use `<div class="success">` to wrap multiple elements.
  If you need more than a sentence + a link, use `<div class="j-save-summary">`
  (the post-publish / draft-saved confirmation card) and put a
  `<p class="success">` inside as the lead. New variants of this
  pattern should add their own `.j-…-summary` class rather than
  re-purposing the flash class.
- The CSS rules backing `.success` and `.error` are currently
  unspecified — see jaunder-styles bd issue. Treat the classes as
  contracts: when CSS lands, every flash already wears the right class.

## 4. Forms

Forms hang off a `ServerAction::<T>::new()` plus an `ActionForm`.

- Bind any controlled input through an `RwSignal` (`prop:value`,
  `on:input`). See `auth.rs` for the canonical lowercase-username
  pattern.
- Every submit button gets a `j-btn` class (and `is-primary` for the
  primary action of the form). Plain
  `<button type="submit">"Save"</button>` is the legacy style.
- Group label + input as
  `<label>"Field" <input … /></label>` — short fields can stay inline,
  longer ones break onto their own lines via the `j-backup-field`
  pattern in `backup.rs`.
- Card-style settings pages should follow `backup.rs`: an
  `<ActionForm attr:class="j-card j-…-form">` containing
  `<div class="j-card-head"><h2>…</h2></div>` and a
  `<div class="j-…-form-actions">` footer.

## 5. Buttons

| Class                 | When                                                |
|-----------------------|-----------------------------------------------------|
| `j-btn`               | Default — neutral form / row action (Edit, Publish, Unpublish, Revoke, secondary form submits) |
| `j-btn is-primary`    | One per form — the action the user is here to take  |
| `j-btn is-danger`     | Destructive action (Delete, and anything else that removes data) — themed via `--err` |
| `j-btn is-accent`     | Reserved for emphasis (rare)                        |
| `j-btn is-active`     | Toggle in active state                              |
| `j-btn is-selected`   | Inside a `j-seg` segmented control                  |

`is-ghost` has been retired — `j-btn` now covers everything that was
"transparent secondary", and destructive actions wear `is-danger`.

`onclick="return confirm('…')"` is the established pattern for
destructive confirmations on row buttons (see `drafts`, `media`). Don't
hand-roll `web_sys::window().confirm_with_message` unless the action is
inside an effectful `dispatch` that has no surrounding form
(see `PostCard`'s delete button).

## 6. Shared components

Always reach for the helper in `web/src/pages/ui.rs` before writing a
new layout primitive:

| Helper              | Purpose                                       |
|---------------------|-----------------------------------------------|
| `Topbar`            | Page header (see §1)                          |
| `BackupBanner`      | Global "backups not configured" banner        |
| `Avatar`            | User initials chip; size in px                |
| `Chip`, `Dot`, `Icon` | Sidebar / inline accents                    |
| `PostCard`          | Renders a `TimelinePostSummary` with author actions |
| `PostDisplay`       | Renders a post without the author action column |
| `PostCreateForm`    | Compose-new-post form (compact and full variants) |
| `InlineComposer`    | Home-feed inline composer with flash         |
| `ComposerFields`    | Body textarea + format toggle, reusable      |
| `MediaUploadButton`, `MediaPanel` | File-picker wired to `/media/upload` |

For list views, the available CSS primitives are:

- `.j-table` — collapsed table with themed borders and meta-cased
  headers. Use for any tabular list (Media is the reference).
- `.j-draft-list` + `.j-draft-row` — card-styled list of rows with
  per-row action column. Use for ad-hoc lists that don't fit a table.

If you find yourself copying a layout block (e.g. a draft row, a
toolbar) into a second place, lift it into `ui.rs`.

## 7. CSS conventions

- All bespoke classes are prefixed `j-` and live in
  `server/assets/jaunder.css`. Themes (variables only) live in
  `jaunder-themes.css`.
- Component variants use BEM-ish modifier classes:
  `.j-btn.is-primary`, `.j-card-head`, `.j-backup-field-wide`.
- Inline `style="…"` is permitted for one-off layout tweaks
  (`margin-top:8px`, dynamic colors). Repeated patterns belong in a
  class — the page gutter is `.j-page`, not inline `padding:16px 32px`.

## 8. SSR-safe Resource patterns

Two anti-patterns to avoid (both have caused production panics — see
the saved `bd memories`):

1. **`Effect::new_isomorphic` (or unwrapped `Effect::new`) that copies a `Resource` into `RwSignal`s.**
   The future can resolve on a tokio worker after the per-request
   reactive owner is disposed. An isomorphic effect firing then would access disposed
   signals and panic. Even a plain `Effect::new` runs its closure once initially on the server during SSR, and can rerun if the resource resolves before SSR finishes, causing random/flaky server-side test coverage (e.g., in `home.rs` or `posts.rs`).
   
   **Always wrap client-only `Effect::new` calls (and their containing blocks if necessary) in `#[cfg(target_arch = "wasm32")]`** so they are completely stripped from server-side compilation, ensuring 100% deterministic server-side test coverage and avoiding unnecessary execution.

2. **SSR-eager `Resource` calling `expect_context::<Arc<AppState>>()`.**
   The same disposal race can hit the context lookup. Use
   `use_context::<Arc<AppState>>().ok_or_else(…)?` — the `Err` branch
   in operator-only UI is already harmless. Do *not* switch to
   `LocalResource` as a structural fix; it never resolves inside an
   SSR-rendered `Suspense`.

When in doubt, mirror `home.rs`: a plain `Effect::new` (gated with `#[cfg(target_arch = "wasm32")]`) that copies the
resolved page into signals and only writes when the value actually
changes (to prevent remounting child components).
