# Web Component Style Guide

How page components in `web/src/pages/` and the shared widgets in
`web/src/pages/ui.rs` should be structured so that pages look and feel the same.

This guide is **descriptive of the design system we already have**
(`server/assets/jaunder.css` + the `Topbar` / `PostCard` / `PostCreateForm`
helpers). Follow it when adding a new page; don't re-invent local variants.

---

## 1. Page chrome

Every full-window page renders, in order:

```rust
view! {
    <Topbar title="…" sub="…" />
    <div class="j-scroll">
        <div class="j-page">
            // page body
        </div>
    </div>
}
```

- **`Topbar`** lives in `web/src/pages/ui.rs`. Do not write a bare
  `<h1>"Title"</h1>` at the top of a page — that is the legacy style and should
  be migrated.
- `title` is required; `sub` is optional but should describe the page
  ("Operations", "Your uploads", "Unpublished posts"). **Don't pass
  `sub=String::new()`** — omit the prop instead, which suppresses the `j-sub`
  line.
- `Topbar` accepts `children` for right-aligned actions (sign-in buttons,
  primary CTA). See `home.rs` Local mode for the pattern.
- `title` and `sub` are `leptos::TextProp`: pass a bare `&'static str` literal
  (`title="Posts"`) or `String` for static content, or a closure (`move || …`) /
  signal for reactive content — see `UserTimelinePage` for the closure form.
- The outer `<div class="j-scroll">` is the scrollable region; the inner padded
  div is the gutter. Pages that follow a dense card-grid layout (Backup,
  settings-style forms) may use `<div class="j-settings">` instead — it provides
  the same padding plus a max-width.

## 2. Suspense and loading placeholders

- Always use `<p class="j-loading">"Loading\u{2026}"</p>` (U+2026 ellipsis, not
  three ASCII dots) inside a `Suspense` fallback.
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
- **Error:** `<p class="error">{e.to_string()}</p>`

```rust
{move || {
    action.value().get().map(|r: Result<T, WebError>| match r {
        Ok(_)   => view! { <p class="success">"…"</p> }.into_any(),
        Err(e)  => view! { <p class="error">{e.to_string()}</p> }.into_any(),
    })
}}
```

- **Do not** use `<div class="success">` to wrap multiple elements. If you need
  more than a sentence + a link, use `<div class="j-save-summary">` (the
  post-publish / draft-saved confirmation card) and put a `<p class="success">`
  inside as the lead. New variants of this pattern should add their own
  `.j-…-summary` class rather than re-purposing the flash class.
- The CSS rules backing `.success` and `.error` are currently unspecified — see
  jaunder-styles bd issue. Treat the classes as contracts: when CSS lands, every
  flash already wears the right class.

## 4. Forms

Forms hang off a `ServerAction::<T>::new()` plus an `ActionForm`.

- Bind any controlled input through an `RwSignal` (`prop:value`, `on:input`).
  See `auth.rs` for the canonical lowercase-username pattern.
- Every submit button gets a `j-btn` class (and `is-primary` for the primary
  action of the form). Plain `<button type="submit">"Save"</button>` is the
  legacy style.
- Group label + input as `<label>"Field" <input … /></label>` — short fields can
  stay inline, longer ones break onto their own lines via the `j-backup-field`
  pattern in `backup.rs`.
- Card-style settings pages should follow `backup.rs`: an
  `<ActionForm attr:class="j-card j-…-form">` containing
  `<div class="j-card-head"><h2>…</h2></div>` and a
  `<div class="j-…-form-actions">` footer.

## 5. Buttons

| Class               | When                                                                                           |
| ------------------- | ---------------------------------------------------------------------------------------------- |
| `j-btn`             | Default — neutral form / row action (Edit, Publish, Unpublish, Revoke, secondary form submits) |
| `j-btn is-primary`  | One per form — the action the user is here to take                                             |
| `j-btn is-danger`   | Destructive action (Delete, and anything else that removes data) — themed via `--err`          |
| `j-btn is-accent`   | Reserved for emphasis (rare)                                                                   |
| `j-btn is-active`   | Toggle in active state                                                                         |
| `j-btn is-selected` | Inside a `j-seg` segmented control                                                             |

`is-ghost` has been retired — `j-btn` now covers everything that was
"transparent secondary", and destructive actions wear `is-danger`.

`onclick="return confirm('…')"` is the established pattern for destructive
confirmations on row buttons (see `drafts`, `media`). Don't hand-roll
`web_sys::window().confirm_with_message` unless the action is inside an
effectful `dispatch` that has no surrounding form (see `PostCard`'s delete
button).

## 6. Shared components

Always reach for the helper in `web/src/pages/ui.rs` before writing a new layout
primitive:

| Helper                            | Purpose                                             |
| --------------------------------- | --------------------------------------------------- |
| `Topbar`                          | Page header (see §1)                                |
| `BackupBanner`                    | Global "backups not configured" banner              |
| `Avatar`                          | User initials chip; size in px                      |
| `Chip`, `Dot`, `Icon`             | Sidebar / inline accents                            |
| `PostCard`                        | Renders a `TimelinePostSummary` with author actions |
| `PostDisplay`                     | Renders a post without the author action column     |
| `PostCreateForm`                  | Compose-new-post form (compact and full variants)   |
| `InlineComposer`                  | Home-feed inline composer with flash                |
| `ComposerFields`                  | Body textarea + format toggle, reusable             |
| `MediaUploadButton`, `MediaPanel` | File-picker wired to `/media/upload`                |

For list views, the available CSS primitives are:

- `.j-table` — collapsed table with themed borders and meta-cased headers. Use
  for any tabular list (Media is the reference).
- `.j-draft-list` + `.j-draft-row` — card-styled list of rows with per-row
  action column. Use for ad-hoc lists that don't fit a table.

If you find yourself copying a layout block (e.g. a draft row, a toolbar) into a
second place, lift it into `ui.rs`.

## 7. CSS conventions

- All bespoke classes are prefixed `j-` and live in `server/assets/jaunder.css`.
  Themes (variables only) live in `jaunder-themes.css`.
- Component variants use BEM-ish modifier classes: `.j-btn.is-primary`,
  `.j-card-head`, `.j-backup-field-wide`.
- Inline `style="…"` is permitted for one-off layout tweaks (`margin-top:8px`,
  dynamic colors). Repeated patterns belong in a class — the page gutter is
  `.j-page`, not inline `padding:16px 32px`.

## 8. Server function module structure

Feature modules in `web/src/` follow the **server submodule pattern** — see
[ADR-0013](../decisions/0013-server-submodule-pattern.md) for the full
rationale.

Each feature is a directory module:

```text
web/src/feature/
├── mod.rs     # Shared DTOs + #[server] functions with real bodies
└── server.rs  # Module-private helpers and tests (omit if not needed)
```

At the top of `mod.rs`:

```rust
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
use server::*;   // all server-only helpers come into scope here
```

Every `#[server]` body is wrapped with `boundary!("function_name", { ... })`. No
per-import `#[cfg(feature = "server")]` annotations appear inside function
bodies — the `#[server]` proc-macro already cfg-gates bodies to SSR, and
`use server::*` covers all server-only imports in one place.

`server.rs` is only created when the module has genuine private helpers worth
naming (multi-step transactions, helpers shared across multiple server
functions, unit tests). Small features may keep everything in `mod.rs`.

## 9. SSR-safe Resource patterns

Two anti-patterns to avoid (both have caused production panics — see the saved
`bd memories`):

1. **`Effect::new_isomorphic` (or unwrapped `Effect::new`) that copies a
   `Resource` into `RwSignal`s.** The future can resolve on a tokio worker after
   the per-request reactive owner is disposed. An isomorphic effect firing then
   would access disposed signals and panic. Even a plain `Effect::new` runs its
   closure once initially on the server during SSR, and can rerun if the
   resource resolves before SSR finishes, causing random/flaky server-side test
   coverage (e.g., in `home.rs` or `posts.rs`).

   **Always wrap client-only `Effect::new` calls (and their containing blocks if
   necessary) in `#[cfg(target_arch = "wasm32")]`** so they are completely
   stripped from server-side compilation, ensuring 100% deterministic
   server-side test coverage and avoiding unnecessary execution.

2. **SSR-eager `Resource` calling `expect_context::<Arc<dyn FooStorage>>()`.**
   The same disposal race can hit the context lookup. Consumers take a specific
   `Arc<dyn FooStorage>` handle, never the whole `AppState` (ADR-0016). Replace
   `expect_context::<Arc<dyn FooStorage>>()` with
   `use_context::<Arc<dyn FooStorage>>().ok_or_else(…)?` — returning the `Err`
   branch gracefully instead of panicking and wedging the worker. Do _not_
   switch to `LocalResource` as a structural fix; it never resolves inside an
   SSR-rendered `Suspense`.

   **Read context before the first `.await`.** When such a function runs as a
   `Resource` resolved during SSR, the renderer can resume the future on a
   worker thread where the Leptos task-local context is no longer installed — so
   any `use_context` placed _after_ an await point (e.g. after
   `require_auth().await`) returns `None`, and because an SSR-resolved
   `Resource` serializes its value to the client and is **not** re-fetched on
   hydration, that `Err` sticks. Fetch every `Arc<dyn FooStorage>` handle first,
   then `await` (mirror `get_registration_policy`; `require_auth` is await-safe
   because it reads its `Parts` context synchronously before its own await).

When in doubt, mirror `home.rs`: a plain `Effect::new` (gated with
`#[cfg(target_arch = "wasm32")]`) that copies the resolved page into signals and
only writes when the value actually changes (to prevent remounting child
components).

**Don't hand-roll the sticky copy for a flat list.** When the retained value is
a plain `Vec`/scalar (not a keyed store — that's §10's `patched`) driven by an
`Invalidator`, use
**`Invalidator::sticky(fetch) -> Signal<Option<Result<T, String>>>`**: it owns
the `resource` + retain-on-resolve effect, is `None` until the first resolve,
then holds the last **result** across every refetch — `Some(Ok(v))` on success,
`Some(Err(msg))` on failure. **Surface the `Err`** (render `<p class="error">`);
do **not** swallow it into a default — that silently misrepresents state (e.g.
an empty member set reading as "nobody is a member", #346), and is inconsistent
with the list-level resource which shows its error. `MemberChecklist` is the
reference (`members.sticky(move || list_audience_members(id))`, matched
three-way `None` / `Some(Err)` / `Some(Ok)`). A **constant-source** resource
that never refetches needs no retention — use a one-line `Signal::derive`
instead (the audiences subscriber roster).

## 10. Keyed lists (reactive `Store`)

Decision record: `docs/adr/0061-web-keyed-list-reactive-store.md`.

A `map`/`collect` list rendered inside a reactive closure rebuilds **every** row
whenever its source signal changes. For a list whose rows carry **per-row
identity that can mutate** (a rename) or **nested component state to preserve**
(a child that has fetched its own data), that rebuild remounts every row and
loses the child state — e.g. the audiences screen's per-row `MemberChecklist`
reflashing "Loading members…" on an unrelated create/rename/delete (#348).

Render such a list from a `reactive_stores::Store`, wired with
`Invalidator::patched`:

```rust
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Store, Patch)]
struct Row { id: i64, name: String }
#[derive(Default, Store, Patch)]
struct Rows { #[store(key: i64 = |r| r.id)] rows: Vec<Row> }

let store = Store::new(Rows::default());
let state = list.patched(fetch_rows, move |rows| store.rows().patch(rows)); // Signal<ListState>
// <ul><For each=move || store.rows() key=|r| r.key() let:row>
//   <Row name={move || row.name().get()} /> …
// </For></ul>
// {move || match state.get() { ListState::Loading => …, Empty => …, Error(e) => …, Loaded => () }}
```

- **`Invalidator::patched(fetch, patch) -> Signal<ListState>`** owns the
  plumbing: it refetches `fetch` when the invalidator fires, and on **success
  only** hands the rows to your `patch` closure (never on a pending or failed
  fetch — last-good rows are retained), returning the list's `ListState`
  (`Loading` / `Empty` / `Loaded` / `Error`). Later verticals copy the two-line
  wiring, not a hand-rolled effect.
- The `patch` step is a **closure** (`move |rows| store.rows().patch(rows)`),
  and that is load-bearing. `store.rows().patch(vec)` is the keyed field's
  **inherent, in-place** patch: it reconciles by key and notifies only the
  subfields whose value changed, so unchanged rows keep their DOM (and their
  children's state) and a rename updates just that row's field. **Never**
  `.write()`/`.set()`, and never route the patch through a _generic_ bound — a
  generic `field.patch(vec)` resolves to the `Patch` trait's **unkeyed,
  positional** patch and remounts the whole list (the bug this pattern exists to
  prevent), which is exactly why `patched` takes a closure rather than the
  field.
- Iterate with a keyed
  `<For each=move || store.rows() key=|r| r.key() let:row>`, **mounted
  unconditionally** — never inside a reactive loading/error branch that would
  tear the whole `<For>` down on a refetch. Render `state` (loading / empty /
  error) in a **sibling** node.
- Read a row's mutable fields as reactive subfields
  (`{move || row.name().get()}`) so a rename updates in place. Keep fields bound
  to editable inputs **uncontrolled** (an initial `row.name().get_untracked()`
  snapshot), so a background refetch cannot clobber an in-progress edit.
  `patch`-on-success also doubles as the sticky retention from §9 (never blanks
  to "Loading…"); the refetch is driven by an `Invalidator` (ADR-0060 / #359).

**Do not** reach for `Store` for a flat, read-only, or stateless list — one with
no per-row identity that mutates and no nested state to keep (the audiences
screen's subscriber roster, or a `MemberChecklist`'s own `<li>` items). Those
stay plain `map`/`collect`; a keyed store there is ceremony for no benefit. The
audiences vertical is the reference: `Store` for the audience list, plain
rendering for the two flat lists inside it.
