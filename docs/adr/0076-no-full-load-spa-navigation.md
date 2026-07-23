# ADR-0076: No in-app full document loads; `~`-prefixed SPA user namespace

- Status: accepted
- Date: 2026-07-23
- Issue: [#592](https://github.com/jaunder-org/jaunder/issues/592)

## Context

The app is a pure-CSR `leptos_router` SPA (SSR was removed — ADR-0044 lineage,
#173/#179/#239). Several flows were still written in the SSR idiom, where a full
document reload was how you refreshed server-rendered state: login/logout
redirects (fixed in #591) and the post lifecycle — publish, unpublish, and a
permalink "misroute" escape hatch (this issue, #592). In pure CSR each reload
tears down and re-boots the wasm app for no benefit, and the reloads were only
kept alive because nothing forbade them.

The permalink escape hatch existed because the SPA's 5-segment permalink route
(`/{username}/{year}/{month}/{day}/{slug}`) matched _any_ 5-segment URL,
including server-owned ones with the same shape (e.g. `/media/…`). Two framework
facts (verified against leptos_router 0.8) shape the fix:

- `handle_anchor_click` intercepts **every** same-origin `<a>` click and hands
  it to the router's `navigate` **before** any route is matched; a no-match
  renders the `<Routes>` fallback **client-side**. There is no automatic server
  handoff (only cross-origin/`target`/`download`/`rel=external` clicks
  hard-navigate).
- The server serves the SPA shell (200) for unmatched GETs, and its projector
  permalink route already requires a **literal `~`** (`/~{username}/…`). So a
  blanket "fallback reloads to the server" would infinite-loop on a
  genuinely-unknown URL.

A design interview established that **nothing in-app links to a bare, non-`~`,
5-segment server URL** (first-party permalink anchors are all `~`-prefixed;
feed/RSD links are `<head>` `<link>`s, never clicked; media files are 6-segment
`<img src>`), so the escape hatch was already unreachable in practice.

## Decision

1. **No in-app full document loads.** Within a live SPA session, all navigation
   is client-side `leptos_router` (`use_navigate()` / `<a>`). Raw
   `window.location` navigation — `.replace()`, `.assign()`, `.reload()`,
   `.set_href()` — is **forbidden** in `web/src` and `client/src`, enforced by
   the `no-full-reload` xtask static check (a host source-scan, because these
   are wasm-gated call sites the default clippy pass does not lint). There is no
   allowlist: the only remaining document loads are inherent and outside the
   scan — cold entry (typing a URL, external links, hard refresh, crawlers), the
   browser fetching a server-owned non-HTML resource (a media file, a feed), and
   the pre-paint `/`→`/app` redirect (`web/src/render/mod.rs`, a JS _string_ the
   AST scan never sees).

2. **The SPA user namespace is `~`-prefixed.** The permalink route's leading
   segment is a custom `TildeUsername` `PossibleRouteMatch` that matches only
   when the segment begins with `~`, mirroring the server's literal-`~`
   projector routes. A non-`~` same-segment-count URL matches no route and
   renders the `<Routes>` fallback ("Page not found.") — accepted, because
   nothing in-app navigates there — rather than mounting a page that bounces via
   a reload.

## Consequences

- The post lifecycle (publish, unpublish) and the permalink page no longer
  reboot the wasm app; publish refetches its page in place when a same-day
  publish leaves the URL unchanged. This completes the "proper SPA" behavior
  begun by #591.
- Reintroducing a `window.location` navigation in `web`/`client` fails
  `cargo xtask check` (`no-full-reload`), with a message pointing at
  `use_navigate()`.
- **Known limitation:** only the 5-segment permalink route is tightened to
  `~`-only — the other username-first SPA routes (`/{username}`,
  `/{username}/tags/{tag}`) stay `ParamSegment`. They carry no reload escape and
  swallow no known server URL, so this is a deliberately partial application of
  the namespace rule, resting on the same in-app link-inventory fact. If a
  future feature adds an in-app link to a bare non-`~` 5-segment server URL,
  that click would render "Page not found." client-side instead of loading the
  resource, and this decision must be revisited (a scoped server handoff, or
  extending the route rule) — not a blanket fallback reload, which would
  infinite-loop against the server's shell fallback.
