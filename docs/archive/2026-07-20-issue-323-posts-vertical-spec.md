# Spec — #323: converge the `posts` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella), milestone #11 — the
**largest** vertical. **Decision record:**
`docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes ADR-0056),
layout template `docs/web-style-guide.md` §8; **ADR-0072**
(`common::time::UtcInstant`, landed by #91/PR #555) governs the boundary
timestamps. **Retained principles:** ADR-0055 (pure logic extracted host-tested
before gating; no fake host stub). **Reconciles** the banked 2026-07-17
interview (mdorman's #323 comment) to ADR-0070 + post-#91. **Templates:** the
shipped `web/src/media/` and `web/src/auth/` verticals.

> **Reconciliation note (2026-07-20).** The banked design was written against
> **ADR-0056** ("host-compiling `posts/ui.rs`; `#[component]`s host-compile
> dual-target, coverage-exempt; no `target_arch` gate"). ADR-0070 supersedes it:
> the `#[component]` UI moves into a **wasm-only `component.rs`**
> (`#[cfg(target_arch = "wasm32")] mod component;`) that never host-compiles.
> The convergence _scope_ is unchanged; only the _gating_ flips. Separately,
> **#91 already typed every posts boundary timestamp** as `UtcInstant`, so the
> banked "type `DraftSummary`/`publish_at`" work is complete and dropped here.

## Problem

The `posts` vertical is split by technology, not feature, and its `mod.rs` mixes
concerns:

- `web/src/posts/mod.rs` — module wiring **+** 10 `#[server]` fns **+** all wire
  DTOs (`CreatePostResult`, `UpdatePostResult`, `AudienceSelection`,
  `DraftSummary`, `PublishPostResult`, `PostResponse`) **+** pure host helpers
  (`audience_*`) **+** tests. Host-compiled.
- `web/src/posts/listing.rs` — 5 more `#[server]` fns (`list_user_posts`,
  `list_local_timeline`, `list_home_feed`, `list_posts_by_tag`,
  `list_user_posts_by_tag`) + `TimelinePostSummary`/`TimelinePage` DTOs +
  server-only fetch helpers.
- `web/src/posts/server.rs` — host-only marshalling (`timeline_post_summary`,
  `post_response`, not-found helpers). Already the ADR-0070 `server.rs` shape.
- `web/src/pages/posts.rs` — the 9 routed `#[component]`s (`CreatePostPage`,
  `PostPage`, `UserTimelinePage`, `DraftPreviewPage`, `EditPostPage`,
  `DraftsPage`, `SiteTagPage`, `UserTagPage`) + `SubscribeButton` + helpers
  (`permalink_first_paint`, `post_data` param parsing, `render_draft_row`,
  delete helpers), wasm-gated via the `pages/` module gate. Four `cov:ignore`
  blocks. 17 `read_signal!` sites.
- Posts UI also depends on widgets and render code living in the **shared**
  `web/src/pages/ui.rs` and `web/src/render/mod.rs`.

## Decisions (interview-resolved, reconciled to ADR-0070 + #91)

1. **ADR-0070 layout for posts.** End state:
   `web/src/posts/{mod.rs, api.rs, server.rs, component.rs, render.rs}`:
   - `mod.rs` → **wiring only** (mirror `media/mod.rs`): `mod` declarations +
     re-exports preserving the stable `crate::posts::…` / registrar paths. No
     items.
   - `api.rs` → the **15 `#[server]` fns** (both the `mod.rs` and `listing.rs`
     sets)
     - all wire DTOs + `deserialize_rendered_html` + the one grouped
       `#[cfg(feature = "server")]` support block. An internal `mod listing;`
       submodule re-exported from `api.rs` is allowed if it keeps files readable
       (plan decides).
   - `server.rs` → host-only marshalling (keep; absorb `listing.rs`'s
     server-only fetch helpers). `#[cfg(feature = "server")] mod server`.
   - `render.rs` (new, **ungated, host-compiled, coincidence-tested**) → the
     pure render twins carved from `render/mod.rs` (Decision 4).
   - `component.rs` (**wasm-only**,
     `#[cfg(target_arch = "wasm32")] mod component;`, zero cfgs inside) → the
     routed `#[component]`s + the pulled widgets + browser glue.

2. **Move `pages/posts.rs` UI into wasm-only `component.rs`.** All 9 routed
   components + `SubscribeButton` + the component helpers move in. Components
   **do not host-compile** — no `cov:ignore`, no `#[component]`-exemption
   reliance. The three `web_sys::window().location().replace` sites are carried
   **as-is** (the #516 `client` nav primitive does not exist yet — confirmed);
   #516 swaps them later.

3. **Pull posts widgets from `pages/ui.rs` into posts (FULL scope).** Move
   `ComposerFields`, `PostCreateForm`, `InlineComposer`, `PostCard`,
   `PostDisplay`, `AudiencePicker`, `TagInput`, `marker_matches`, and the
   datetime helpers into posts. The `#[component]`s land in `component.rs`;
   re-export the ones consumed outside posts (`PostCard` → `pages/timeline.rs`;
   `PostCreateForm`/`InlineComposer` → `pages/cockpit.rs`) from `posts/mod.rs`
   and repoint those call sites to `crate::posts::…`. (The shared remainder of
   `pages/ui.rs` and `escape_html` stay for **#312**.)

4. **Carve the pure render twins into `posts/render.rs` (FULL scope).** Move
   `permalink_article`, `render_posts`, `PostView`, `render_post_article`,
   `render_post_inner`, `render_post_content`, `render_timeline_page`,
   `format_post_time`, `render_body` from `render/mod.rs` into `posts/render.rs`
   (host-compiled, used by both the server projector and the wasm
   `PostDisplay`), with their coincidence tests. **`escape_html` stays** in
   `render/mod.rs` (shared, used by topbar — #312). **`PageSeed` stays** in
   `render/mod.rs` (the shared projector seam); posts imports it. Preserve the
   flash-free projector/reactive byte-coincidence.

5. **Migrate the local-datetime helper to `chrono` (host-testable).** Replace
   the `js_sys::Date`-based `local_datetime_to_utc_rfc3339` with a
   `chrono`-based pure conversion (browser/host local wall-clock →
   `UtcInstant`), host-compiled and host-tested (fixed-TZ unit tests), per
   ADR-0055. Feed it from `publish_at_from_local` (which #91 added). Requires
   making `chrono` usable in the wasm `web` build — do the **minimal**
   dependency change (chrono already reaches the wasm bundle transitively via
   `common`, so cost is marginal); do **not** put `chrono` types in any
   `#[server]` signature (ADR-0072 — the boundary stays `UtcInstant`). The
   `js_sys::Date` path is removed.

6. **#299 — fold `create_post`/`update_post` into typed arg-structs.** Bundle
   the 8 positional args into `CreatePostArgs` / `UpdatePostArgs` (fields incl.
   `publish_at: Option<UtcInstant>`, already typed by #91), removing the
   `#[allow(clippy::too_many_arguments)]`. The `#[server(input = Json)]` wire
   nests under the arg name — internal to leptos's own client, so acceptable;
   **verify via the create/edit-post e2e** (no external client depends on the
   raw shape).

7. **Retire all four `cov:ignore` blocks.** `permalink_first_paint`,
   `render_draft_row`, `render_delete_form`, `render_delete_result` move into
   wasm-only `component.rs` (not coverage-measured → no `cov:ignore` needed,
   like media's `render_media_row`). Extract the **pure** bits into ungated
   host-tested fns (per #306): the `post_data` permalink-param parsing
   (username/date/slug from route params) and `render_draft_row`'s
   title/schedule-badge logic. None of the four markers survive.

8. **Inline `read_signal!` (folds #304).** Replace posts' 17 `read_signal!`
   sites with `.get()`.

9. **Cross-vertical widgets carried, relocated later.** `SubscribeButton` (→
   subscriptions **#327**) and `SiteTagPage`/`UserTagPage` (→ tags **#328**)
   move into `posts/component.rs` as part of this convergence; #327/#328 (both
   blocked-by #323) relocate them to their own verticals afterward.

## Target end state (acceptance floor)

1. `posts`' UI, `#[server]` fns, wire types, and pure render twins live under
   `web/src/posts/`; **no `web/src/pages/posts.rs` remains**; its `pages/mod.rs`
   declaration, imports, and the 8 `<Route>` `use`s are repointed to
   `crate::posts::…` (routes themselves unchanged — App/Router relocation is
   #330).
2. The `#[component]` UI lives in **wasm-only `component.rs`** —
   `target_arch = "wasm32"` only on the `mod`/`pub use` lines, zero cfgs inside
   the file; components **do not host-compile**; **no `cov:ignore` /
   `#[component]`-exemption** added for UI.
3. `mod.rs` is **wiring only**; the 15 `#[server]` fns + DTOs live in `api.rs`;
   host-only support in `#[cfg(feature = "server")] server.rs`; the pure render
   twins in the host-compiled `render.rs`.
4. Registrar/stable paths preserved via re-exports: every `web::posts::…`
   `#[server]` arg-struct that `server/tests/helpers/mod.rs` registers still
   resolves (the `server-fn-registrar` gate is green).
5. `pages/ui.rs` no longer defines the pulled posts widgets;
   `timeline.rs`/`cockpit.rs` import them from `crate::posts::…`.
   `escape_html` + `PageSeed` remain in their shared homes.
6. No `js_sys::Date` in the datetime path; the local→UTC conversion is a
   host-tested pure fn; no `chrono` type crosses a `#[server]` boundary (stays
   `UtcInstant`).
7. `create_post`/`update_post` take a single typed arg-struct; no
   `too_many_arguments` allow remains on them.
8. All four `cov:ignore` blocks gone; pure param/draft-row logic extracted +
   host-tested; no fake host stub (ADR-0055).
9. `cargo xtask validate` green, including the posts e2e (`posts.spec.ts` + the
   posts flows in `visibility`/`feeds`/`atompub`/`media`/`authed-flash`) and the
   render-twin **coincidence tests** (projector ↔ reactive byte-equality).

## Shape of the work

Sequenced so each step gates green (plan will task this precisely):

- **`api.rs` split** — move the 15 `#[server]` fns + DTOs out of
  `mod.rs`/`listing.rs`; reduce `mod.rs` to wiring; verify registrar paths (like
  media Task 2). Keep `server.rs` host-only (absorb listing's fetch helpers).
- **`render.rs` carve** — relocate the render twins + coincidence tests out of
  `render/mod.rs`; keep `escape_html`/`PageSeed` shared; prove byte-coincidence.
- **`component.rs`** — move the routed components + pulled widgets; retire
  `cov:ignore`; extract the pure param/draft-row helpers host-tested; inline
  `read_signal!`; carry the `location().replace` glue.
- **Widget rewire** — repoint
  `timeline.rs`/`cockpit.rs`/`pages/mod.rs`/`pages/ui.rs`.
- **datetime → chrono** (host-tested) and **#299 arg-structs** (typed;
  e2e-verified).
- Delete `pages/posts.rs`.

## Out of scope

- Dissolving the shared remainder of `pages/ui.rs` / `web::render`
  (`escape_html`, `Topbar`, `Sidebar`), and moving `PageSeed` — **#312**.
- Moving `App`/Router out of `pages/mod.rs` — **#330**.
- Final relocation of `SubscribeButton` → **#327**, tag pages → **#328** (they
  land in posts here; those issues pull them out).
- The `client` navigation primitive swap for the `location().replace` sites —
  **#516**.
- Any change to the `UtcInstant`/ADR-0072 boundary typing — **done by #91**.

## Verification

`cargo xtask validate` (static + wasm-clippy + coverage + full e2e matrix).
Load-bearing behavioral checks: the posts e2e (`posts.spec.ts`:
create/edit/publish/schedule/delete, drafts, permalink, tag + timeline pages)
and the posts flows in `visibility`/`feeds`/`atompub`/`media`/`authed-flash`.
The **render-twin coincidence tests** (host-compiled) guard the flash-free
projector ↔ reactive byte-equality across the carve. Because the UI is now
wasm-only, **wasm-clippy is load-bearing gate surface** for this vertical's UI
type-checking (ADR-0070). #299's wire change is validated by the
create/edit-post e2e.
