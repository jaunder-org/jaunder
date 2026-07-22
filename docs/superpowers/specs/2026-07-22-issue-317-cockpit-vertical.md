# Spec — #317: converge the `cockpit` vertical onto the co-located Leptos layout

**Status:** awaiting approval. **Milestone:** #11 (Web: canonical Leptos CSR
convergence). **Parent:** #303 (umbrella). **Decision record:** ADR-0070
(`docs/adr/0070-web-vertical-wasm-only-component-files.md`) + `web-style-guide.md`
§8; this vertical adds no new ADR. **Template:** the home vertical (#319, shipped)
— the closest precedent (a server-less composite/landing vertical).

## Problem

The cockpit UI lives in the legacy `web/src/pages/cockpit.rs` (108 lines), not in a
co-located vertical. It defines one component, `CockpitPage` (the `/app` authed-only
personalized feed, #181/ADR-0044 D6), and is **server-less** — it owns no `#[server]`
fn or wire DTO; it composes other verticals (`auth::current_user`, `posts::{list_home_feed,
InlineComposer}`, `timeline::{TimelineRows, TimelineState}`) plus a reactive `Topbar`.

It reaches into the `pages/` module for two things ADR-0070 wants gone:

- `use crate::pages::ui::Topbar` — the `pages::ui` re-export (dissolving in #312), not
  `Topbar`'s own home `crate::topbar`.
- `use crate::pages::signal_read::read_signal` — the vestigial `read_signal!` macro,
  now a pure `.get()` pass-through (`signal_read.rs:9-13`; #304 removes it wholesale).

`pages/cockpit.rs` is **already** wasm-only (the whole `pages` module is
`#[cfg(target_arch = "wasm32")]`, `lib.rs:34`), so this is a **relocation + pages/
decoupling**, not a gating change — and cockpit, being **authed-only** (anonymous
visitors bounce to `/login` before any chrome paints), has **no** anon masthead, no
`inner_html` pure-fn coincidence, and no `j-anon-only` owner-flash concern (that is the
*home* pattern, not cockpit's).

> **Stale-body note.** Issue #317's body says "Blocked by the prereq (#312)." The
> native dependency graph says the opposite — #317 has no open blockers, and #312 is
> blocked *by* the nine verticals (incl. #317). Per project convention (native deps
> over body notes), and confirmed by home #319 (a sibling composite vertical) already
> shipping the same way, #317 is unblocked. The body text predates the #526 re-scope.

## Decisions (interview-resolved)

1. **Cockpit becomes a server-less two-file vertical `web/src/cockpit/`** — `mod.rs`
   (wiring only) + wasm-only `component.rs` — mirroring home #319. **No `api.rs` /
   `server.rs`** (ADR-0070 permits omitting them; cockpit owns no server surface).

2. **`CockpitPage` moves verbatim into `component.rs`** (keeping its authed-only
   `current_user` gate, `/login` bounce, `Topbar`/`InlineComposer`/`TimelineRows` body,
   and the two anti-remount guards on `username`/`status`), declared
   `#[cfg(target_arch = "wasm32")] mod component;`. No pure host-testable logic to
   extract (ADR-0070 §6) — the component is entirely reactive/async composition, as
   home's was.

3. **Full `pages/` decoupling** (maintainer-directed scope):
   - **Inline `read_signal!` → `.get()`** at its three sites (`pages/cockpit.rs:77-79`):
     `read_signal!(state.status)` → `state.status.get()`, `read_signal!(bounce)` →
     `bounce.get()`, `read_signal!(username)` → `username.get()`. Drop the
     `use crate::pages::signal_read::read_signal` import.
   - **Import `Topbar` from `crate::topbar`** directly; drop `use crate::pages::ui::Topbar`.
   - Result: `component.rs` carries **zero `crate::pages::*` imports**. (This is
     stricter than the home template, which still imports `read_signal!`
     (`home/component.rs:9`) — cockpit deliberately goes one step further on
     `pages/` decoupling than #319 did.)

4. **Register the vertical at the web crate root:** add `pub mod cockpit;` to `lib.rs`
   (alphabetical, after `pub mod backup;`), ungated — mirroring `pub mod home;`.

5. **Rewire `pages/mod.rs`:** remove `pub mod cockpit;` (line 1) and change the import
   `use crate::pages::cockpit::CockpitPage;` → `use crate::cockpit::CockpitPage;`
   (line 26, ungated like `use crate::home::HomePage;`). The `/app` route
   (`<Route path=StaticSegment("app") view=CockpitPage />`) is **unchanged**.

6. **Delete `web/src/pages/cockpit.rs`.** `pages::signal_read` and `pages::ui` **stay**
   (other `pages/` files still use them; #312/#330 remove them); cockpit merely stops
   importing them.

## Acceptance criteria

- **AC1 (co-located).** `web/src/cockpit/mod.rs` and `web/src/cockpit/component.rs`
  exist; `CockpitPage` is defined in `component.rs`; `web/src/pages/cockpit.rs` no
  longer exists.
- **AC2 (mod.rs wiring-only).** By inspection, `cockpit/mod.rs` contains only
  `#[cfg(target_arch = "wasm32")] mod component;` and
  `#[cfg(target_arch = "wasm32")] pub use component::CockpitPage;` (plus a doc
  comment) — no items of its own, matching `home/mod.rs`.
- **AC3 (pages/ decoupled).** `web/src/cockpit/component.rs` contains **no**
  `crate::pages::` reference; `Topbar` comes from `crate::topbar`; there is no
  `read_signal!` use — the three reads are plain `.get()`.
- **AC4 (root registration).** `lib.rs` declares `pub mod cockpit;`; `pages/mod.rs` no
  longer declares `pub mod cockpit;` and imports `CockpitPage` from `crate::cockpit`.
- **AC5 (behavior unchanged).** `/app` still routes to `CockpitPage`; the owner boots
  straight into the personalized feed + composer, and an anonymous/expired visitor
  bounces to `/login` — the two component-level guarantees in
  `end2end/tests/authed-flash.spec.ts` (owner-boots-into-feed `:53`, anon-bounces
  `:83`) stay green. (The `:64` pre-paint `/`→`/app` redirect test exercises the
  app-shell redirect script, not `CockpitPage` itself.)
- **AC6 (gate).** `cargo xtask validate` passes, including cockpit's e2e flows
  (authed-flash, plus the `/app` feed/composer flows in `posts.spec.ts` /
  `media.spec.ts`); no new `cov:ignore` / `crap:allow` markers.

## Out of scope

- `read_signal!`'s wholesale removal across the other verticals (#304) — only cockpit's
  three uses are inlined here.
- Dissolving `pages::ui` / `web::render` (#312) and deleting the `pages/` module (#330).
- Any change to the composed verticals (`auth`, `posts`, `timeline`, `topbar`) or to
  the `/app` route path / redirect behavior.

## Test impact

None expected. The relocation, `read_signal!` inlining, and `Topbar` re-home are
behavior-preserving refactors; the existing e2e (`authed-flash.spec.ts`,
`posts.spec.ts`, `media.spec.ts`) already covers the `/app` cockpit contract and is the
conformance check for AC5. `component.rs` is wasm-only (not host-coverage-measured), as
`pages/cockpit.rs` already was — no host-coverage delta.
