# 0041. Public projector and CSR client (the "SSR the data, not the components" mechanism)

- Status: accepted
- Date: 2026-07-01
- Issue: #178 + #179 (milestone 8 — "Off concurrent SSR"); implements ADR-0040; amends ADR-0002; relates #173, ADR-0016

## Context

ADR-0040 chose the *direction* — server UI-free, web client leptos-CSR, content
rendered by a shared pure fn so the projector and client coincide. This ADR records
the concrete *mechanism* decisions made building the first slice (#178 projector +
#179 CSR client), which future readers would otherwise reverse-engineer. Everything
lands behind the existing `csr` cargo feature; the `not(csr)` reactive-SSR path is
untouched and removed later at #180.

## Decision

1. **The pure render fn lives in `web/src/render/`, not `common` or a new
   `jaunder-core`.** `web` already compiles to wasm (it *is* the CSR client) and
   `server` already depends on `web`, so a single reactivity-free module there is
   reachable by both the server-side projector and the wasm client with no plumbing,
   and the public DTOs already live in `web` (no migration). `jaunder-core` is
   deferred (§4); when it materializes the fn moves there regardless of where it
   starts, so "cleanest today" wins. The module uses **no leptos reactivity** — plain
   string building like `common::feed` — and must compile under `ssr`/`hydrate`/`csr`.

2. **Coincidence is by construction, not by parallel markup.** `render_body(&seed)`
   returns an HTML `String`; the projector embeds it and the CSR public-page component
   renders it via `inner_html=render_body(&seed)`. Both sides emit identical bytes
   because they call the identical fn — there is no `view!`-macro twin to keep in sync.
   Sharing a *reactive component* rendered to string is the prohibited trap door back
   to isomorphic SSR (ADR-0040).

3. **The seed contract is `PageSeed`** — a serde enum (one variant per public page
   kind, carrying the route context the bare `TimelinePage` lacks) serialized as JSON
   into a `<script type="application/json" id="jaunder-seed">`. It is just the DTOs the
   data layer already returns, so it round-trips into identical Rust types on the wasm
   side. The client reads it on boot, provides it as an `Option<PageSeed>` context, and
   seeds first paint from it; client-side navigation falls back to the `#[server]` fns.

4. **The projector renders the anonymous view only, via an explicit-viewer fetch
   seam.** The public data queries are extracted from the `#[server]` fns into plain
   `async fn fetch_*(storage, viewer, …)` taking an explicit `ViewerIdentity`; the
   `#[server]` wrapper passes the request's real viewer, the projector always passes
   `ViewerIdentity::Anonymous`. This keeps one query with no drift, and guarantees the
   projector output is **byte-identical per URL for every visitor** (no auth branch) —
   the property that makes it CDN-cacheable. Per-viewer/authenticated enhancement is a
   client concern (#181), never the projector's.

## Consequences

- The projector is a thin axum handler with no `reactive_graph` on the path — same
  posture as the existing feed handlers, which it structurally resembles.
- The public first paint is flash-free: the CSR mount replaces `#app` with content its
  own `render_body` reproduces from the same seed.
- The `#[server]` fns stay the data API for client-side navigation and the authed
  cockpit; only the anonymous first-paint fetch is duplicated into the projector via
  the shared `fetch_*` seam.
- Amends ADR-0002 (Frontend Framework): the web surface is now a non-reactive public
  projector + a leptos-CSR client, not isomorphic SSR.

## Addendum (#179 flash-free shell + #180 cutover, 2026-07-01)

The projector/CSR path landed behind the `csr` feature alongside the SSR/hydrate
default; #180 made it the **only** path.

- **Flash-free extended from the post to the whole anonymous shell.** The initial
  increment made the projector's post markup coincide with the reactive `PostDisplay`;
  it was completed so the projector serves the **entire** anonymous `j-root` layout
  (sidebar + main region + per-route topbar/hero/wrappers/posts) that the reactive
  `App` produces for an anonymous viewer, so removing `#app` and mounting the CSR
  client on boot causes no reflow. Technique: "share the pure fn, not the component" —
  the reactive `PostDisplay`/`Sidebar` render their anonymous DOM via `inner_html` of
  the **same** pure `render` fns the projector uses, so coincidence holds by
  construction. **Only anonymous paths get this**; authed/author UI (own-post action
  columns, authed sidebar footer/nav) stays ordinary client-reactive (the projector
  never renders it) — full authed-flash handling is #181. `render_head` gained the
  feed + RSD autodiscovery `<link>`s the reactive components used to inject via SSR.
- **#180 removed the reactive SSR render (closes #173).** `create_router`'s projector
  arm is now unconditional; the `leptos_axum` `leptos_routes_with_context` /
  `generate_route_list` / `file_and_error_handler(shell)` arm, `web::shell()`, the
  `hydrate` crate + `web/hydrate` feature, and `server`'s `csr` feature are gone.
  `flake.nix` builds only the CSR client (`site`), and the standard
  `{backend}×{browser}` e2e matrix now exercises the CSR build (the manual
  `csr-e2e-postgres-chromium` check is retired). KEPT: `handle_server_fns_with_context`
  (the `/api` data API), `leptos/ssr` (server-fn impls), `server_boundary`/`server_resource`.
- The `web` feature still named `ssr` (it now means "the server-side data-API build,
  no page render"); a cosmetic rename to `server` is a deferred follow-up.
