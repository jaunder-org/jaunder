# Plan — flash-free anonymous shell + off-SSR cutover (#179 finish, #180, closes #173)

Supersedes the post-only "Piece A" in `docs/issue-180-handoff.md`. Scope chosen by
the user (2026-07-01): **true flash-free** — the projector must server-render the
**entire anonymous app layout**, not just the post markup, so the pre-boot paint
matches the reactive App's anonymous first paint with no reflow when wasm boots.

## The gap this closes

- **Projector `document()`** emits only `<div id="app">{render_body}</div>` where
  `render_body` today is bare content (`<h1 class="j-timeline-title">` +
  `<div class="j-timeline">` of `<article>`s, or one `<article>` for a permalink).
- **Reactive `App`** wraps every route in `j-root > j-shell > (Sidebar) +
  (main-region > main > Outlet)`, and each page adds `<Topbar>` + `j-scroll`/
  `j-page` wrappers + `<PostCard>`s. On boot `mount_csr` removes `#app` and mounts
  the whole `App`, so sidebar/topbar pop in and the content block relocates.

So the current increment is content/SEO-only, not flash-free. Post-markup
coincidence alone (the old Piece A) buys nothing perceptible because the whole
content block still shifts under the incoming shell.

## Architecture — "share the pure fn, not the component" (ADR-0041 §4), extended to the shell

Coincidence is guaranteed **by construction** where a node has no reactive
children: the reactive component renders that node's DOM by `inner_html`-ing the
**same pure `String` fn** the projector uses. Nodes that must hold reactive
children (the posts list) stay reactive, and the projector hand-matches their
(trivial, static) wrapper divs.

New/!expanded pure renderers in `web/src/render/`:

| Pure fn | Produces | Reactive consumer (anon path → `inner_html`) |
|---|---|---|
| `render_post(post, ctx, is_author) -> String` | the inner HTML of one `<article class="j-post">` (avatar + content grid, context-aware tags) | `pages::ui::PostDisplay` |
| `render_sidebar(active) -> String` | the inner HTML of `<aside class="j-sidebar">` for the anonymous viewer (brand, search, public nav, sources, empty foot) | `pages::ui::Sidebar` (anon; authed stays reactive) |
| `render_topbar(title, sub, right) -> String` | the inner HTML of `<div class="j-topbar">` | `pages::ui::Topbar` |
| `render_body(seed) -> String` | the **full** anonymous `j-root` shell for the seed (assembles sidebar + main-region + per-route page: topbar, optional hero, `j-scroll`/`j-page`, posts via `render_post`, load-more when `has_more`) | projector `document()`; the reactive pages hand-match the wrapper divs |

Shared constants both sides consume (kill drift at the leaves): `Icons::*` (already
`pub const`), the sidebar sources list, the home hero copy, the topbar sub strings.
Extract these to `render` (or a shared `web::chrome` const module) and have the
reactive components read them.

Boundary that stays reactive / hand-matched:
- The **posts list container** (`j-scroll` / `j-page`) holds reactive `PostCard`s
  (load-more, author actions). Projector emits the identical literal wrapper divs;
  these are trivial static `<div class="…">` so drift risk is negligible.
- **Authed** sidebar nav, footer avatar, `InlineComposer`, `BackupBanner`, and the
  per-post **author action column** stay fully reactive and layer on top after
  `current_user()` resolves — a truly-anonymous visitor never renders them, so
  anonymous coincidence holds. Full authed-flash polish is **#181** (out of scope).

## Commit sequence (each: `cargo xtask check` green; web-touching ones also
`csr-e2e-postgres-chromium`)

1. **`render_post` + PostDisplay unification** (old Piece A). Extract the whole
   article inner into `render_post`; `PostDisplay` anon → `<article inner_html=…>`,
   author → article + reactive action overlay (CSS re-home). `render_body` loops it.
   Unit tests for `render_post`.
2. **Sidebar coincidence.** `render_sidebar(active)`; `Sidebar` anon base via
   `inner_html`; authed path unchanged. Shared source/nav consts.
3. **Topbar coincidence.** `render_topbar`; `Topbar` via `inner_html`; shared sub
   consts. Home hero as a shared const string.
4. **Full-shell assembly.** `render_body(seed)` emits the entire anonymous `j-root`
   shell (sidebar + main + per-route topbar/hero/scroll/page + posts + load-more).
   Projector `document()` uses it. Confirm each reactive page's anonymous wrapper
   structure matches literal-for-literal. Resolve the stylesheet-path question
   (projector links `/style/jaunder.css`+`-themes.css`; App links `/pkg/jaunder.css`
   — confirm same bytes or unify). Full `cargo xtask validate` (e2e) green.
5. **#180 Rust cutover.** Delete the `#[cfg(not(csr))]` `leptos_axum` SSR-render arm
   in `server/src/lib.rs`; make the projector arm unconditional. Drop `server` `csr`
   feature, `web::shell()`, `web` `hydrate` feature, the whole `hydrate/` crate +
   workspace member. Rename `web` feature `ssr`→`server` everywhere. KEEP
   `handle_server_fns` (the `/api` data API), `leptos/ssr`, `server_boundary`/
   `server_resource`. `cargo xtask check` green.
6. **#180 flake/e2e cutover.** flake.nix: delete `hydrateWasm` + `jaunderBinCsr`;
   repoint wasm bundle + `site` to `csr*`; retire the `csr-e2e-postgres-chromium`
   special check (the matrix now IS csr). Verify `xtask/src/steps/nix.rs` resolves.
   Full `cargo xtask validate` green. ADR-0041 addendum (or new ADR) recording the
   cutover + the flash-free shell decision.
7. **Ship.** Delete `docs/issue-180-handoff.md`; archive this plan/spec; push; open
   PR closing #178/#179/#173; project Status → Done. Merge is the human halt.

## Risks / open checks
- **Wrapper hand-match drift** (steps 3–4): the only non-by-construction surface.
  Mitigate by extracting the leaf constants and keeping wrappers trivial. Consider a
  small doc note pinning the anonymous shell structure as the shared contract.
- **Stylesheet path** mismatch (`/style/*` vs `/pkg/jaunder.css`) — resolve in step 4.
- **Theme flash**: projector paints `DEFAULT_THEME`; the client restores a non-default
  theme from `localStorage` before first paint → a color flash for users who changed
  theme. Pre-existing, orthogonal, not addressed here (note for #181/theme work).
- The csr-e2e gate proves no `reactive_graph` panic and preserved selectors, **not**
  the absence of flash — coincidence is enforced by the by-construction `inner_html`
  design + manual review, not an automated pixel test.
