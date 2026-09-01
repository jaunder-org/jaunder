# Web Component Style Guide

How page components (each vertical's wasm-only `component` boundary — see §8)
and the shared leaf widgets (top-level modules like `web/src/topbar/`) should be
structured so that pages look and feel the same.

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

- **`Topbar`** lives in `web/src/topbar/` (`component.rs`). Do not write a bare
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

Always reach for an existing shared helper before writing a new layout primitive
— leaf primitives live in their own **top-level modules**
(`web/src/{avatar,icon,topbar}/`, exposing `Avatar`, `Icon`, `Topbar`); the rest
are co-located in their owning verticals (ADR-0070):

| Helper                            | Purpose                                           |
| --------------------------------- | ------------------------------------------------- |
| `Topbar`                          | Page header (see §1)                              |
| `BackupBanner`                    | Global "backups not configured" banner            |
| `Avatar`                          | User initials chip; size in px                    |
| `Icon`                            | Sidebar / inline accents                          |
| `PostCard`                        | Renders a `RenderedPost` with author actions      |
| `PostDisplay`                     | Renders a post without the author action column   |
| `PostCreateForm`                  | Compose-new-post form (compact and full variants) |
| `InlineComposer`                  | Home-feed inline composer with flash              |
| `ComposerFields`                  | Body textarea + format toggle, reusable           |
| `MediaUploadButton`, `MediaPanel` | File-picker wired to `/media/upload`              |

For list views, the available CSS primitives are:

- `.j-table` — collapsed table with themed borders and meta-cased headers. Use
  for any tabular list (Media is the reference).
- `.j-draft-list` + `.j-draft-row` — card-styled list of rows with per-row
  action column. Use for ad-hoc lists that don't fit a table.

If you find yourself copying a layout block (e.g. a draft row, a toolbar) into a
second place, lift it into a shared leaf module — a top-level
`web/src/<widget>/` directory following the `avatar`/`icon`/`topbar` shape:
`markup.rs` for any pure (host-tested) `render()` twin, wasm-only `component.rs`
for the `#[component]`.

A leaf need not have both halves. `taglist/` is pure `markup.rs` only — its
chips are injected via `inner_html` by the projector and the CSR client alike,
so there is one renderer and no twin to keep coincident.

### Read-only props take a reference

A prop the component only **reads** — one that never reaches the view itself,
only derived owned data does — takes a reference:

```rust
#[component]
pub fn Avatar<'a>(name: &'a Username, #[prop(default = 38)] size: u32)
    -> impl IntoView + use<>
```

`use<>` is precise capturing (ADR-0104 §2): the returned view captures no
lifetime, which is what lets a borrowing component be used inside a **stored**
view such as a `Suspend` body, where a captured lifetime would hit the `'static`
requirement. It is not optional here — without it the prop's lifetime is
captured and those call sites stop compiling.

This is worth doing rather than taking the value: it drops a `.clone()` at every
call site that already owns its data. Worked examples: `Avatar`,
`FeedDiscovery`, `RsdDiscovery`, `PostDisplay`, `PostCard` (#301).

Three things that bite:

- **Ownership has to terminate somewhere.** Converting one component pushes the
  same `needless_pass_by_value` to its caller. Follow the chain to whoever
  genuinely owns the data — for posts that is `TimelineRows`, which owns the
  rows it iterates — or stop and take the value.
- **An inline-constructed prop must be bound to a local first.**
  `surface=&FeedSurface::Site` compiles; `surface=&FeedSurface::User { … }` does
  not (E0716) — the temporary is dropped inside the `view!` expansion. Bind it,
  then pass `&local`.
- **A reference default needs a promotable constant.**
  `#[prop(default = &TagContext::SiteWide)]` works because that expression is
  static-promotable; an arbitrary constructor call is not.

A prop whose value **does** reach the view stays owned — the view must own it.

## 7. CSS conventions

- All bespoke classes are prefixed `j-` and live in `server/assets/jaunder.css`.
  Themes (variables only) live in `jaunder-themes.css`.
- Component variants use BEM-ish modifier classes: `.j-btn.is-primary`,
  `.j-card-head`, `.j-backup-field-wide`.
- Inline `style="…"` is permitted for one-off layout tweaks (`margin-top:8px`,
  dynamic colors). Repeated patterns belong in a class — the page gutter is
  `.j-page`, not inline `padding:16px 32px`.

## 8. Server function module structure

Feature modules in `web/src/` follow the **file-level host/wasm split** — see
[ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md) (supersedes
ADR-0056; ADR-0013 records the original server-submodule half). Existing
verticals are still converging onto this layout (the #526 migration) — new code
follows it; old code matches it after its vertical's convergence issue.

Each feature is a directory module:

```text
web/src/feature/
├── mod.rs        # Module wiring only: mod declarations + re-exports
├── api.rs        # Shared wire DTOs + #[server] functions with real bodies
├── server.rs     # Host-only helpers and tests (omit if not needed)
└── component.rs  # #[component] UI + browser-bound code, or its wiring facade
```

A **server-less** vertical — one with no `#[server]` fns or wire types of its
own (e.g. `cockpit/` and `home/`, which call other verticals' server fns) —
omits `api.rs` too, keeping just `mod.rs` + its pure host-tested and/or
wasm-only UI files.

**A vertical's `#[server]` fns live in its `api.rs`, never in a submodule**
(#714). `#[macros::server]` derives the wire endpoint and the span name from
`(vertical, ident)` and hard-errors on any file that is not
`web/src/<vertical>/api.rs` — which is what makes that pair a primary key
**rustc** enforces, rather than one a gate checks. A vertical that outgrows one
`api.rs` therefore cannot split its server fns out; see
[ADR-0070](adr/0070-web-vertical-wasm-only-component-files.md) point 1 and
[ADR-0082](adr/0082-server-fn-wire-namespace.md) for the escape hatch.

`mod.rs` declares and re-exports — nothing else:

```rust
mod api;
#[cfg(feature = "server")]
mod server;
#[cfg(target_arch = "wasm32")]
mod component;   // the vertical's UI — wasm-only, never host-compiled

pub use api::{CreateThing, ThingDto, create_thing};
#[cfg(target_arch = "wasm32")]
pub use component::ThingPage;
```

The re-exports keep external call-site and server-fn-registrar paths
(`web::feature::CreateThing`) stable, so relocating items into `api.rs` never
touches consumers. At the top of `api.rs`:

```rust
#[cfg(feature = "server")]
use super::server::*;   // all server-only helpers come into scope here
```

Server fns are declared `#[macros::server]` — spelled fully-qualified and never
`use`d, so it cannot be mistaken for leptos's own `#[server]`. It derives the
wire endpoint (`/<vertical>/<ident>`), the ADR-0011 span name
(`web.<vertical>.<ident>`), and the error-boundary wrap around the body, so none
of the three is written by hand:

```rust
#[macros::server(skip(name))]
pub async fn rename(audience_id: AudienceId, name: AudienceName) -> WebResult<()> {
    let user = require_auth().await?;
    // ... full implementation here; no boundary wrapper to write
    Ok(())
}
```

It accepts `input = …` (forwarded to `#[server]`) and `skip(…)`/`skip_all`
(forwarded to `#[tracing::instrument]`). `endpoint` and `name` are rejected —
they are derived — as are `fields`, `level`, `err`, `ret`, and any unrecognized
key. No per-import `#[cfg(feature = "server")]` annotations appear inside
function bodies — the `#[server]` proc-macro already cfg-gates bodies to SSR,
and the single grouped import covers all server-only imports in one place.

`server.rs` is only created when the module has genuine private helpers worth
naming (multi-step transactions, helpers shared across multiple server
functions, unit tests). Small features may keep everything in `api.rs`.

`component.rs` is wasm-only by its `mod` declaration and carries **zero cfg
gates inside the file** — it may call `client::` primitives and `web-sys`
directly, and it never host-compiles. Keep pure, host-testable logic
(validation, signal/form state, codecs) **out** of `component.rs`, in ungated
host-tested files — extraction precedes gating. The only `target_arch` cfgs in
`web/src` are these wiring lines — the `mod component;` declarations and their
paired `pub use` re-exports.

When a vertical's wasm-only UI grows large, its `component.rs` may be a wiring
facade containing module documentation, private leaf declarations, and explicit
re-exports over cohesive private leaves under `component/`. Those leaves inherit
the enclosing wasm-only boundary; they carry no cfg gates of their own. This
does not move pure, host-testable logic behind the facade: it remains in
ungated, host-tested files as above.

## 9. Resource → signal patterns (CSR)

Routed Leptos components serve a static CSR shell and mount fresh on the client
via `mount_to_body` — there is no server-render-then-hydrate pass
(`csr/src/lib.rs`; the `server`/`leptos/ssr` build serves the server functions,
the projector's pure render fns, and `leptos_axum` routing, **not** component
hydration). So a plain client-only `Effect::new` that copies a resolved
`Resource` into `RwSignal`s is the normal idiom, not an SSR-safety workaround.

1. **Copying a `Resource` into signals.** Mirror `home.rs`: a plain
   `Effect::new` that copies the resolved page into signals and only writes when
   the value actually changes (to prevent remounting child components). A
   **client-only `Effect::new` belongs in the vertical's wasm-only
   `component.rs`** (§8), where it is structurally stripped from every host
   build — never add an `Effect::new` to host-compiled code, and never re-gate
   it per-call inside a file.

   Three cases, narrowest first:
   - Where the signals exist _only_ to receive the copy, **skip the seed
     entirely** and consume the `Resource` directly in the view (under
     `Suspense`/`.get()`, or a derived signal) — one fewer intermediate signal
     that can drift from its source (`AudiencePicker`'s named-audience list).
   - For a **seed-then-edit** signal (the user mutates it after the fetched
     value seeds it) on a component that **already awaits** the value under
     `Suspense`, seed it inside the `Suspend` block, not a standalone `Effect`
     (the editor's current-audience seed).
   - Reach for a standalone client-only `Effect` **only** for a seed-then-edit
     signal on a component that **renders immediately**, with no `Suspense` to
     await under — the composer seeds its site-default audience after first
     paint this way, because the compose box must appear without waiting on the
     fetch.

2. **Server-fn storage handles: take a specific `Arc<dyn FooStorage>`, not the
   whole `AppState` (ADR-0016), and fail gracefully.** Prefer
   `use_context::<Arc<dyn FooStorage>>().ok_or_else(…)?` over
   `expect_context::<Arc<dyn FooStorage>>()`, returning the `Err` branch instead
   of panicking and wedging the worker. Fetch every handle **before** the first
   `.await` (mirror `registration::get_policy`; `require_auth` reads its `Parts`
   context synchronously before its own await), so a future resumed on another
   worker thread never reads a task-local context that is no longer installed.

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
reference (`members.sticky(move || list_members(id))`, matched three-way `None`
/ `Some(Err)` / `Some(Ok)`). A **constant-source** resource that never refetches
needs no retention — use a one-line `Signal::derive` instead (the audiences
subscriber roster).

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
struct Row {
    #[patch(|this, new| *this = new)] id: RowId,     // an IdNewtype
    #[patch(|this, new| *this = new)] name: RowName, // a StrNewtype
}
#[derive(Default, Store, Patch)]
struct Rows { #[store(key: RowId = |r| r.id)] rows: Vec<Row> }

let store = Store::new(Rows::default());
let state = list.patched(fetch_rows, move |rows| store.rows().patch(rows)); // Signal<ListState>
// <ul><For each=move || store.rows() key=|r| r.key() let:row>
//   <Row name={move || row.name().get().to_string()} /> …
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
  (`{move || row.name().get().to_string()}` — see the newtype bullet below) so a
  rename updates in place. Keep fields bound to editable inputs **uncontrolled**
  (an initial `row.name().get_untracked()` snapshot), so a background refetch
  cannot clobber an in-progress edit. `patch`-on-success also doubles as the
  sticky retention from §9 (never blanks to "Loading…"); the refetch is driven
  by an `Invalidator` (ADR-0060 / #359).
- **A row holds domain newtypes, not primitives** — a store row is not an
  exception to the ADR-0063 pervasiveness rule. Every **leaf** field of the row
  struct takes `#[patch(|this, new| *this = new)]`, the id field included:
  `Patch` otherwise dispatches through `reactive_stores::PatchField`, which is
  implemented only for a closed set of primitives and which the orphan rule bars
  us from implementing (it would be coherent only in `common`, which must stay
  leptos-free). What needs no attribute is the **`#[store(key: …)]` collection
  field** — its key type only has to satisfy `PatchFieldKeyed`'s bounds, all of
  which `IdNewtype` derives. (The id field's own attribute is inert at runtime:
  rows are matched _by_ that key, so its comparison never fires. It is there to
  compile.) See `docs/adr/0078-reactive-store-domain-newtype-fields.md`; the
  audiences vertical is the worked example. A newtype is not `IntoRender`, so
  read it out at view sites — `.to_string()` when the row is borrowed (as
  `web/src/subscriptions/component.rs` does for `Username`), `String::from(…)`
  to move it out of an owned row (as `uploaded_url_view` in
  `web/src/media/component.rs` does for `RootRelativeUrl`, whose comment spells
  out why the unwrap happens at the view site rather than the value being
  carried around stringly).

**Do not** reach for `Store` for a flat, read-only, or stateless list — one with
no per-row identity that mutates and no nested state to keep (the audiences
screen's subscriber roster, or a `MemberChecklist`'s own `<li>` items). Those
stay plain `map`/`collect`; a keyed store there is ceremony for no benefit. The
audiences vertical is the reference: `Store` for the audience list, plain
rendering for the two flat lists inside it.
