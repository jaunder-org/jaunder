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
