# Public projector + leptos-CSR client — design spec

**Issues:** #178 (public projector) + #179 (convert web client to leptos-CSR) —
implemented as **one combined cycle / one PR**, closes both.
**Milestone:** 8 — Off concurrent SSR (web re-architecture v1).
**Design source:** `docs/inbound-data-handling.md` §4 ("SSR the data, not the
components"); background `docs/issue-173-findings-and-pivot-handoff.md`. Both live on
unpushed sibling branches, not `main`.
**Date:** 2026-07-01.

## 1. Why

The current web stack renders public pages via Leptos **reactive SSR** (Suspense +
`Resource` + `ServerAction` running in the concurrent server request path). That
machinery is the home of the #173 concurrent-SSR reactive-disposal panic —
upstream-unfixed by policy (`leptos #4590` NOT_PLANNED) — and of the ADR-0016 SSR
owner/context panics (#124/#138). The #177 spike (ADR-0040) confirmed **leptos-CSR is
panic-free** and landed the feature-gated CSR infra on `main`.

This slice removes the reactive runtime from the public request path by adopting §4's
model: the server emits **one cacheable response per public URL** — semantic HTML +
an embedded data blob + a boot script — and the **client's capability** decides what
it becomes (crawlable document for bots/no-JS; booted SPA for JS humans). Degradation
stops being a code path and becomes an emergent property of one response meeting
clients of differing capability.

## 2. Scope

**In scope (combined #178 + #179):**

- A pure, non-reactive render module shared by the server-side projector and the
  wasm CSR client.
- Server-side **projector** axum handlers for the public/discoverability routes.
- The **data-blob** seed contract and the client-side **seed harness** that boots the
  CSR client from it.
- Converting the public-page components to render first paint from the seed (via the
  shared render fn), dropping their reactive first-paint fetch.

**All of it lands behind the existing `csr` cargo feature.** The `not(csr)` default
build — today's reactive `leptos_routes` SSR — is **untouched**, keeping the current
green gate at zero risk. The new behaviour is exercised by the existing
`csr-e2e-postgres-chromium` nix check (4 workers, `fullyParallel` — the #173
reproduction harness).

**Public routes the projector owns:**

| Route | Page | Data DTO |
|---|---|---|
| `/` | site / local timeline | `TimelinePage` |
| `/:username` | profile timeline | `TimelinePage` |
| `/:username/:year/:month/:day/:slug` | post permalink | `PostResponse` |
| `/tags/:tag` | site tag page | `TimelinePage` |
| `/:username/tags/:tag` | user tag page | `TimelinePage` |

Syndication feeds (`/feed.{ext}`, `/~{username}/feed.{ext}`, …) already have
non-reactive handlers (`common/src/feed/*` + `server/src/feed/handlers.rs`) — **out of
scope**, but they are the shape-precedent this work follows.

**Out of scope (later milestone-8 issues, do not touch):**

- Removing the `leptos_axum` reactive render / flipping `csr` to default / closing
  #173 → **#180**.
- Authenticated-owner flash handling (enhance-don't-replace, pre-paint auth) → **#181**.
- Re-enabling parallel e2e `workers>1` for the whole suite → **#182**.
- `jaunder-core` / sync engine / REST — **deferred** (§4). The `#[server]` fns stay as
  the data API; the render fn lives in `web`, not a new core crate.

## 3. Architecture

The §4 "trinity over the shared core" shape, in miniature, all behind `csr`:

```
                web::render   (pure, no reactivity, compiles on all targets)
                  /                       \
   server projector handler          CSR client (web App, csr feature)
   (fetch → render → HTML+blob)       (read blob → render_body → #app)
```

- **Integration point.** The `csr` feature already forks the server router:
  `not(csr)` = reactive `leptos_routes` SSR; `csr` = static-SPA `ServeDir` fallback.
  The projector slots into the `csr` arm — public routes are matched by projector
  handlers **ahead of** the static-SPA fallback. `/api/*` server fns, assets, and the
  SPA shell for non-public routes are unchanged.
- **Server projector** = a plain axum handler (no `reactive_graph` on the path — same
  posture as the feed handlers): shared fetch fn → `web::render` → assemble document.
- **Client** = the `web` App under `csr`; public-page components render first paint
  from the seed via the same `web::render` fn.

## 4. Components

### 4.1 `web/src/render/` — the shared pure render module

A reactivity-free module in `web` (chosen over `common` because `web` already compiles
to wasm *and* `server` already depends on `web`, so both consumers reach it with zero
plumbing, and the public DTOs already live in `web` — no migration; when `jaunder-core`
eventually materializes the fn moves there regardless of where it starts).

- Uses **no** leptos reactivity — plain string-building like `common/src/feed/*`.
  Must compile under all of `web`'s `ssr` / `hydrate` / `csr` features.
- Two pure fns per page kind:
  - `render_head(&PageSeed) -> String` — per-page `<title>` + meta / Open-Graph
    derived from the data (the SEO/discoverability payload) + the CSS/asset links the
    csr shell uses.
  - `render_body(&PageSeed) -> String` — the semantic content HTML.
- **Coincidence is trivial-by-construction:** the fn returns an HTML `String`; the
  projector embeds it and the CSR component does `<div inner_html=render_body(&seed)>`.
  Both sides emit identical bytes because they call the identical fn — no parallel
  `view!`-macro markup to keep in sync. This is §4's "share the pure render fn, not
  the component"; the §4 **trap door** (sharing a *reactive* component rendered to
  string = isomorphic SSR again) is thereby avoided.
- Post *body* HTML is already pre-rendered (`TimelinePostSummary.rendered_html`,
  innerHTML today); the render fn produces the surrounding semantic **chrome**
  (article, byline, permalink, tag list, timeline wrapper) and injects that pre-rendered
  body, escaping all other text.

### 4.2 Fetch layer — shared plain-async fns

Extract each public query into `async fn fetch_*(storage: &Arc<dyn PostStorage>, …) ->
WebResult<Dto>` (ssr-gated, in `web/src/posts/`). The existing `#[server]` fn becomes a
thin wrapper (`expect_context` the storage, call `fetch_*`); the projector handler
(holding app state) grabs the `Arc` and calls the same `fetch_*`. One query, two
callers, no drift. Covers `fetch_user_posts`, `fetch_local_timeline`,
`fetch_posts_by_tag`, `fetch_post`.

### 4.3 Data-blob contract — `PageSeed`

A serde-tagged enum, serialized as `serde_json` into a
`<script type="application/json" id="jaunder-seed">` in the projector's document:

```rust
enum PageSeed {
    SiteTimeline(TimelinePage),
    Profile(TimelinePage),
    SiteTag(TimelinePage),
    UserTag(TimelinePage),
    Permalink(PostResponse),
}
```

It is just the DTOs the fetch fns already return (already `Serialize`/`Deserialize`
because they cross the server-fn boundary), so it round-trips into the identical Rust
types on the wasm side.

### 4.4 The projector document

For each public URL the projector emits a full HTML document (not the generic static
shell):

- `<head>` = `render_head(&seed)` — real per-page title + meta/OG + asset links.
- `<body>` = a known mount container `<div id="app">render_body(&seed)</div>`, the
  `#jaunder-seed` blob script, and the existing csr **boot script** (`/pkg/jaunder.js`
  — the same bundle the static shell loads).

The JS-off document is a finished, crawlable semantic page. Bytes are **identical for
every anonymous visitor** (no per-request nonce/timestamp/auth branch) → CDN-cacheable;
an ETag is derived from the content hash (feed-handler pattern).

### 4.5 Client seed harness (the #179 half)

On boot the CSR client: reads `#jaunder-seed` → deserializes `PageSeed` → provides it
as an `Option<PageSeed>` context. The CSR mount targets `#app` and **replaces** its
contents (no duplicate paint). Each public-page component:

- if the seed matches the current route → render first paint synchronously from it via
  `render_body` (no `Resource`/`Suspense`);
- on client-side navigation (no seed) → fall back to the existing `#[server]` fetch.

Replacing `#app` with the same-string render is what makes boot **flash-free** for the
anonymous visitor.

## 5. Data flow

1. Anonymous GET `/:username` (csr build).
2. Projector handler matches, calls `fetch_user_posts(&storage, username, …)`.
3. `web::render::{render_head,render_body}(&PageSeed::Profile(page))` → HTML strings.
4. Handler assembles `<head>` + `<div id="app">…</div>` + `#jaunder-seed` + boot
   script; returns with cache headers + ETag.
5. Bot/no-JS: done (crawlable). JS client: wasm boots, reads seed, mounts into `#app`
   replacing its content with the coincident `render_body` output; further navigation
   is API-driven via the `#[server]` fns.

## 6. Testing

- **Unit** (`web/src/render/`): pure DTO→HTML assertions — semantic tags, HTML-escaping
  of all non-body text, permalink/tag structure, empty/edge cases. Fast, no I/O.
- **Cacheable / byte-identical**: integration test — two requests to one public URL →
  identical body bytes + an ETag present.
- **Crawlable, no reactivity on path**: e2e on the `csr-e2e-postgres-chromium` VM,
  **JS off**, asserts the post title/content is present in the raw HTML (proves the
  server painted it). The concurrent-workers harness simultaneously guards **no
  `reactive_graph` panic** — the whole point.
- **Flash-free coincidence**: e2e (JS on) — content stable across boot; `#app` not
  wiped-and-rebuilt to different markup.
- **Backend parity**: fetch-fn / projector integration tests run on **both sqlite +
  postgres** per CONTRIBUTING; the `test-backend-pattern` guard covers them.
- Gate: `cargo xtask check` green (and the manual `csr-e2e-postgres-chromium` check for
  the e2e slices).

## 7. Commit plan (verticals — foundation first, then thin per-route slices)

Organized by **vertical** so coincidence is proven at every commit rather than deferred
to a big end-of-PR integration. Each commit references **#178 and #179** (a vertical
advances both sides).

0. **Foundation + ADR.** `web/src/render/` module scaffold; `PageSeed` contract; the
   projector axum router seam under `csr`; the client boot/seed harness (read blob →
   context; mount into/replace `#app`). Write **ADR-0041** (next after ADR-0040)
   recording the projector architecture + shared-pure-fn placement + blob-seed contract
   and amending **ADR-0002** (Frontend Framework) for the web surface; add its row to
   the ADR table in `docs/README.md`. (Rails only — no route rendered yet.)
1. **Permalink vertical** (`/:username/:y/:m/:d/:slug`): simplest data shape (one
   `PostResponse`). `fetch_post` extraction + `render_*` + projector handler + client
   seed/render + coincidence e2e. First end-to-end proof.
2. **Profile / site-timeline vertical** (`/:username`, `/`): timeline list + cursor.
3. **Tag-pages vertical** (`/tags/:tag`, `/:username/tags/:tag`): reuses timeline
   rendering, mostly wiring.
4. **Cross-cutting guards** (if not folded into the verticals): byte-identical/cacheable
   integration test + the JS-off crawlable + no-`reactive_graph` e2e assertions.

## 8. Acceptance criteria

From #178:
- Public routes render correct, crawlable (JS-off) semantic HTML with **no
  `reactive_graph` on the request path**.
- The projector's content markup **coincides** with the CSR client's render of the same
  data (shared pure fn).
- Anonymous responses are **byte-identical per URL** (cacheable).

From #179:
- The web client boots via **CSR mount** (`mount_to_body`/`mount_to #app`), **no
  hydration**, on the public routes.
- Data flows via the existing `#[server]` fns for client-side navigation.
- First paint is **seeded from the projector's blob**.

Both:
- `cargo xtask check` green; `csr-e2e-postgres-chromium` green (panic-free under
  concurrent workers).

## 9. Risks & gotchas

- **wasm-bindgen ↔ leptos version lock; leptos 0.8.20 regresses rendering** (issue
  note). Stay on the pinned versions the #177 infra established; do not bump.
- **Mount-replace flash.** `mount_to_body` appends; the harness must mount into / clear
  `#app` so the server-painted content isn't duplicated or visibly rebuilt. Proven per
  vertical by the flash-free e2e.
- **Cacheability discipline.** The projector output must not vary per request (no
  clock/nonce/auth branch) or byte-identity + CDN-cacheability break. Enforced by the
  byte-identical test.
- **`web/src/render/` must stay reactivity-free** and compile under `ssr`/`hydrate`/`csr`
  — a stray `Resource`/signal reintroduces the #173 class. Kept pure by construction
  (string-building only).
- **Backend parity**: any storage-touching test needs the dual-backend template or the
  `test-backend-pattern` guard fails.
