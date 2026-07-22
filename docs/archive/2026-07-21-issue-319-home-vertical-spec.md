# Spec — #319: converge the `home` vertical + restore ADR-0041 coincidence

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision
records:** `docs/adr/0070-web-vertical-wasm-only-component-files.md` (the
layout), **`docs/adr/0041-public-projector-and-csr-client.md` §2 (coincidence by
construction — the load-bearing rule this issue restores)**, and
`docs/adr/0044-authenticated-owner-flash-free-enhancement.md` §4 (additive
decoration; render-fn drift is the flash risk). Sibling precedent: #329
(timeline). Reference implementation to copy: `Sidebar` (`web/src/pages/ui.rs`).

## Problem

`web/src/pages/home.rs` (`#[component] HomePage`, the routed `/` public
Local-timeline landing page, `pages/mod.rs:113`) has three issues:

1. **It violates ADR-0041 §2.** It re-implements the home **masthead** — the
   topbar (title/sub + the anonymous Sign-in/Register links) and the hero — as
   reactive `view!` markup (`home.rs:68-88`), while the projector renders the
   _same_ masthead as pure strings (`web/src/render/mod.rs::render_hero()`,
   whose doc literally says _"mirroring `home.rs`"_, and the
   `topbar::render(…)` + CTA string inlined in
   `web/src/posts/render.rs::render_body`'s `SiteTimeline` arm,
   `render.rs:44-64`). ADR-0041 §2: _"Coincidence is by construction, not by
   parallel markup … there is no `view!`-macro twin to keep in sync. Sharing a
   reactive component rendered to string is the prohibited trap door back to
   isomorphic SSR."_ This hand-synced duplication is the exact drift risk
   ADR-0041 / ADR-0044 exist to forbid.

2. **A latent authed-owner flash.** The reactive CTA links carry `j-anon-only`
   (`home.rs:73,76`); the projector's twin does **not** (`render.rs:48-49`). The
   `/` projector HTML is served byte-identically to _everyone_ including the
   authed owner (ADR-0044 §5, D10). `html.authed .j-anon-only{display:none}`
   (`server/assets/jaunder.css:1287`) therefore cannot hide the
   **server-painted** CTA for the owner until the reactive re-render adds the
   class — the owner briefly sees Sign-in/Register.

3. **It lives in `pages/`** (technology-grouped), which ADR-0070 §5 dissolves;
   §5 names `home` as a vertical needing a new dir. It also imports `Topbar`
   through the stale `crate::pages::ui::Topbar` re-export shim.

**Not in play:** the timeline posts already coincide correctly — `PostCard` →
`PostDisplay` renders anon bodies via `inner_html=render_post_inner(…)`
(`posts/component.rs:150`), layering the owner action column additively. Only
the masthead is a violation. On the standing newtype steer: no domain-value/wire
surface here.

## Decisions (interview-resolved)

1. **New `web/src/home/` vertical (ADR-0070 §5).** A **two-file** vertical:
   `mod.rs` (wiring) + `component.rs` (the wasm-only `HomePage`). No
   `api.rs`/`server.rs`/`state.rs` — `HomePage` has no server surface and no
   pure logic of its own.

2. **Restore coincidence: one pure masthead fn, rendered by both sides (ADR-0041
   §2).** Add `pub(crate) fn render_home_masthead() -> String` to
   `web/src/render/mod.rs` (beside `render_hero`; dual-compiled, host-tested) =
   `topbar::render( "jaunder.local", Some("Read-only · posts originating on this instance"), <CTA>) + render_hero()`,
   where `<CTA>` is the two Sign-in/Register links.
   - The projector's `render_body` `SiteTimeline` arm calls
     `render_home_masthead()` in place of its inlined topbar+hero.
   - The reactive `HomePage` renders it the **Sidebar way** —
     `let masthead = crate::render::render_home_masthead(); view!{ <div style="display:contents" inner_html=masthead></div> }`
     — replacing the `view!` topbar+hero+CTA block. `HomePage` **drops the
     reactive `<Topbar>` component and the `crate::pages::ui` import entirely**
     (the topbar now comes from the pure `topbar::render` inside the masthead).
     The `render_hero` "mirroring `home.rs`" drift twin is eliminated — one
     source, coincidence by construction.

3. **Fix the owner flash in the shared fn.** `<CTA>` carries `j-anon-only`
   (`class="j-btn j-anon-only"` / `class="j-btn is-primary j-anon-only"`), so
   the projector-painted masthead now hides Sign-in/Register for the authed
   owner pre-paint (ADR-0044). Both sides use the one fn, so they stay
   byte-identical. Update the `render_body` unit assertion (`render.rs:458,462`)
   to the new class, and add an owner-view e2e assertion (Sign-in/Register
   **hidden** for the registered owner on `/`) in `authed-flash.spec.ts`.

4. **What stays reactive** (legitimately, per ADR-0044 §4): `FeedDiscovery`, the
   `read_error` banner (reads the `LoadStatus` `Memo`), and `TimelineRows` (the
   `on_load_more` / `on_mutate` handlers, the `has_more` gate, and `PostCard`'s
   marker-driven action column). Post bodies already coincide via `PostCard`.

5. **Pure render fns stay in `web/src/render` (ADR-0041 §1); co-location is
   #312.** This issue does **not** move `render_home_masthead`/`render_hero`/the
   CTA into the `home` or `auth` verticals, add a `markup.rs` to any vertical,
   or touch the `auth` vertical. Those relocations are the `web::render`
   dissolution (#312). `HomePage` simply _calls_ the shared fn.

6. **Router repoint + deferred couplings.** `pages/mod.rs`'s
   `use crate::pages::home::HomePage;` → `use crate::home::HomePage;`; its
   `pub mod home;` is deleted; `lib.rs` gains `pub mod home;`; the
   `<Route path=StaticSegment("") view=HomePage />` line is unchanged.
   `HomePage` keeps `crate::pages::signal_read::read_signal` (its inlining is
   #304) and the `crate::timeline::…` / `crate::posts::list_local_timeline` /
   `PageSeed` couplings (no cycle). `mod.rs` is ungated `pub mod home;` (empty
   on host, where `component` is compiled out).

## Target end state (acceptance floor — observable)

1. `web/src/pages/home.rs` **gone**; `pub mod home;` at `pages/mod.rs` deleted;
   `lib.rs` declares `pub mod home;`; `web/src/home/` contains exactly `mod.rs`
   - `component.rs`.
2. `render_home_masthead()` exists in `web/src/render/mod.rs`, is
   **host-tested**, and returns the topbar (`jaunder.local` / the read-only sub
   / the two `j-anon-only` CTA links) followed by the hero. Its CTA links carry
   `j-anon-only`.
3. `posts/render.rs::render_body` (`SiteTimeline`) renders the masthead **via
   `render_home_masthead()`** — no inlined topbar/hero/CTA remains in that arm.
4. `home/component.rs`'s `HomePage` renders the masthead via
   `<div style="display:contents" inner_html=render_home_masthead()></div>`; it
   has **no `view!` hero, no inline `j-anon-only` links, no `<Topbar>` element,
   and no `crate::pages::ui` import**. It contains zero cfg attributes
   (wasm-only via the `mod` declaration).
5. `target_arch = "wasm32"` appears in the vertical **only** on `mod.rs`'s two
   wiring lines. The router imports `crate::home::HomePage`; no
   `crate::pages::home` path remains.
6. No reactive `Hero`/`SignInRegisterLinks` component is created; the `auth`
   vertical is untouched; no `markup.rs` is added to any vertical.
7. `mod.rs` and `component.rs` each open with a `//!` doc naming their role.
8. `cargo xtask check` / `validate --no-e2e` green (incl. `wasm-clippy`), and
   CI's e2e matrix green — including the updated `authed-flash.spec.ts` owner
   assertion.

## Shape of the work

- **`render_home_masthead()` first** (`render/mod.rs`): compose
  `topbar::render(…)
  - render_hero()`, CTA links with `j-anon-only`; add its host test. Repoint `render_body`'s `SiteTimeline`arm to call it; update the`render_body`unit assertions to the new CTA class. (Projector output changes only by the added`j-anon-only`.)
- Create `web/src/home/component.rs`: `HomePage` = `FeedDiscovery` + the
  `inner_html` masthead (Sidebar pattern) + `TimelineRows`, dropping the `view!`
  masthead, `<Topbar>`, and `crate::pages::ui` import. Add `web/src/home/mod.rs`
  (wiring + `//!`). Add `pub mod home;` to `lib.rs`.
- Repoint `pages/mod.rs`'s import; delete `pages/home.rs` + its `pub mod home;`.
- Add the owner-view assertion to `authed-flash.spec.ts` (Sign-in/Register
  hidden for the owner on `/`).
- `git grep` for stray `pages::home` / inline `j-hero` / `j-anon-only` in
  `home/component.rs`.
- Gate with `cargo xtask check`; `validate --no-e2e` locally, CI matrix for e2e.

## Out of scope

- Moving the pure render fns into the verticals / adding `markup.rs` /
  dissolving `pages/ui.rs` + `web::render` — that is **#312**.
- Making the reactive `<Topbar>` component itself `inner_html`-backed by
  `topbar::render` (it is byte-coincident by unit test today) — a broader
  topbar-layer change, not home's concern.
- Inlining `read_signal!` (**#304**); the cockpit vertical (**#317**); the
  App/Router move (**#330**).
- Any hero/CTA copy or behaviour change beyond adding `j-anon-only` (the flash
  fix); register-flow feature (#444) / owner-redirect preference (#201).

## Verification

`cargo xtask validate` (static + `wasm-clippy` + coverage + e2e). Guards:

- **Coincidence** — `home_local_body_has_topbar_hero_signin_and_posts`
  (`posts/render.rs:454`) plus the new `render_home_masthead` host test pin the
  projector bytes; both the projector and the reactive `HomePage` call the one
  fn, so they cannot drift.
- **Anon still sees the links** — `example.spec.ts` (`main`-scoped Sign-in /
  Register visible) stays green (anon → `j-anon-only` shown).
- **Owner flash fixed** — the new `authed-flash.spec.ts` assertion (owner on
  `/`: Sign-in/Register hidden) is the behavioural proof; the existing
  owner-chrome assertions (`authed-flash.spec.ts:28-42`) stay green.

`render_home_masthead()` is the new host-testable pure logic; the reactive
`HomePage` remains a thin wasm-only composition (coverage-exempt
`#[component]`).
