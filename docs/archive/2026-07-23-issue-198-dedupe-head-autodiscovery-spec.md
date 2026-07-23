# Spec — #198: dedupe `<head>` autodiscovery links (projector head + reactive components both emit)

**Issue:** jaunder-org/jaunder#198 · **Milestone:** 8 (Off concurrent SSR / web
re-architecture v1) · **Branch:** `worktree-issue-198-dedupe-head-links`

## Problem

A public page's `<head>` ends up with **two identical sets** of feed/RSD
autodiscovery `<link>`s post-boot:

1. **Server (projector):** `web::render::render_discovery`
   (`web/src/render/mod.rs:148`, appended by `render_head`) paints the
   RSS/Atom/JSON `<link rel="alternate">`s per surface, plus — on the
   user-profile surface only — the RSD `<link rel="EditURI">`. Crawlers and
   no-JS browsers see exactly this and stop; **wasm never runs for them**.
2. **Client (reactive):** after wasm boots, the page mounts `FeedDiscovery` /
   `RsdDiscovery` (`web/src/feed_discovery/component.rs`), whose
   `leptos_meta::Link`s hoist the **same** links again → duplicate, invisible
   `<head>` entries.

Harmless (crawlers use the first occurrence; browsers ignore duplicates) but
untidy.

**Why the reactive components stay (issue premise correction):** client-side SPA
navigation between public surfaces (`/` → a permalink → a tag page) does **not**
re-serve the projector head, so the reactive components are what keep the links
correct across those navigations. They are load-bearing, not redundant — the
dedupe removes the **server's** copy at boot, not the reactive one. (#592
removed the last full-reload flows, making the reactive components strictly more
essential.)

### Code-reference correction

The issue predates #519 (merged), which moved CSR boot out of `web` into the
`csr`/`client` crates. The "`web::mount_csr` (`web/src/lib.rs:70-93`) drops the
projector `#app`" pattern the issue says to mirror now lives in
**`csr::mount()`** (`csr/src/lib.rs:30-41`), which calls
**`client::dom::remove_element_by_id("app")`** before `mount_to_body`. The fix
lands there.

### Parity (why boot-time removal is safe)

Every surface the projector paints discovery links for has a matching reactive
mount, so removing the server's copies never leaves a surface linkless:

| Projector surface (`render_discovery`) | Reactive mount                                                           |
| -------------------------------------- | ------------------------------------------------------------------------ |
| `SiteTimeline` → `FeedSurface::Site`   | `home/component.rs:72` `<FeedDiscovery Site>`                            |
| `Profile` → `FeedSurface::User` + RSD  | `posts/component.rs:1472/1475` `<FeedDiscovery User>` + `<RsdDiscovery>` |
| `SiteTag` → `FeedSurface::SiteTag`     | `posts/component.rs:2075` `<FeedDiscovery SiteTag>`                      |
| `UserTag` → `FeedSurface::UserTag`     | `posts/component.rs:2266` `<FeedDiscovery UserTag>`                      |
| `Permalink` → none                     | none                                                                     |

The projector head also carries two `rel="stylesheet"` `<link>`s
(`render/mod.rs:126-127`) that **must survive** — so removal must target only
the discovery links.

## Resolved decisions

1. **Direction (issue):** the projector head serves crawlers/no-JS; the reactive
   components own the `<head>` post-boot. Dedupe by removing the
   **projector-painted** discovery links at CSR boot; keep the reactive
   components mounted.
2. **Scope the removal with an explicit marker (interview Q1 → option a).** Tag
   each projector-emitted discovery `<link>` with a `data-jaunder-discovery`
   attribute (a marker, no value needed), and remove by that attribute — not by
   `rel`. Explicit ("server-painted copy the client replaces") and robust to any
   future `rel="alternate"` link added server-side for another purpose. The
   marker name is a shared `const` in `web::render` so the emitter and the
   boot-time remover cannot drift.
3. **Generic removal primitive.** Add
   `client::dom::remove_elements_by_selector(selector: &str)` —
   `document.query_selector_all(selector)` then `.remove()` each — mirroring the
   existing generic, domain-free `client::dom` primitives
   (`remove_element_by_id`, `text_content_by_id`; ADR-0069). The domain-specific
   selector is supplied by the caller.
4. **Removal site + timing.** In `csr::mount()`, immediately after the existing
   `remove_element_by_id("app")` and **before** `mount_to_body`, call
   `remove_elements_by_selector` with the marker selector. Doing it before mount
   means the reactive `FeedDiscovery`/`RsdDiscovery` then produce the _only_ set
   — no duplicate window. It is a no-op on the static SPA shell (no projector
   links) and for authed pages (no projector head). The links are invisible head
   metadata, so unlike the `#app` drop there is no flash concern.
5. **Reactive components unchanged.** `FeedDiscovery`/`RsdDiscovery` stay
   exactly as they are — they remain mounted on the public pages and own the
   post-boot `<head>`.

## Out of scope

- Changing what links each surface emits, or the feed/RSD endpoints themselves.
- The authed/cockpit surfaces (no projector discovery head to dedupe).
- Any change to the reactive components' behavior or mount sites.

## Acceptance criteria (observable)

1. **Crawler/no-JS path unchanged.** `render_discovery` still emits the same
   per-surface discovery `<link>`s (now carrying `data-jaunder-discovery`).
   Primary check is a Rust **host test** on `render_discovery` output: each
   surface carries the expected `rel`s and the marker attribute. E2E backstop: a
   **no-wasm HTTP fetch** of a public surface — `page.request.get(url)` (the
   idiom `feeds.spec.ts` already uses; it never boots wasm, so it is the crawler
   path) — returns HTML still containing the feed links (and the profile's RSD
   link).
2. **Exactly one set post-boot.** After wasm boots (`body[data-hydrated]`) on a
   public feed surface, the `<head>` contains exactly **one**
   `<link rel="alternate">` per format (RSS/Atom/JSON) — three total, not six —
   and, on the user profile, exactly one `<link rel="EditURI">`.
3. **Updates across client-side nav.** After a genuine **client-side**
   navigation between two public surfaces with _different_ feeds, the `<head>`
   discovery links update to the destination surface's feeds and there is still
   exactly one set (the reactive components, now the sole owner, rewrote them).
   **Concrete driver:** on `/` (Site feed), a post's footer tag chip is an
   `<a class="j-tag" href="/tags/{slug}">` (`taglist/markup.rs:20`, painted by
   `taglist::render`); clicking it is a leptos_router-intercepted same-origin
   nav to the SiteTag feed. **Fixture requirement:** the test must seed a public
   post carrying a tag so a chip exists to click.
4. **Removal is scoped.** The two `rel="stylesheet"` `<link>`s survive boot;
   only the discovery links are removed.
5. **No e2e asserts the duplicates.** Any existing feed e2e that tolerated
   duplicates (`feeds.spec.ts` uses first-match `.find`) is tightened to assert
   the single set.
6. **All four `{sqlite,postgres}×{chromium,firefox}` e2e combos green.**

## Decision record

The dedupe direction (server head for crawlers; reactive components own the
post-boot `<head>`, with server copies dropped at CSR boot via a marker) is a
small, local convention rather than a cross-cutting architectural invariant.
Assess during planning whether it warrants an ADR or is adequately captured by
the code comments + this spec; default to **no ADR** unless the soundness review
flags a reversible decision a future reader would otherwise re-derive.
