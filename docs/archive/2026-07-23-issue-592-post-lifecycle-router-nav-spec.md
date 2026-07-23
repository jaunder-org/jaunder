# Spec — #592: post-lifecycle full reloads → router navigation; `~`-only permalink route; no-full-load gate

**Issue:** jaunder-org/jaunder#592 · **Milestone:** 8 (Off concurrent SSR / web
re-architecture v1) · **Blocked-by:** #591 (closed) · **Branch:**
`worktree-issue-592-router-nav`

## Problem

The app is a pure-CSR `leptos_router` SPA, but the post-lifecycle flows still
force **full document reloads** — an SSR-era pattern where a reload was how you
refreshed server-rendered state. In pure CSR each reload tears down and re-boots
the wasm app for no benefit. This is the last batch of SSR-vestige reloads (#591
removed the login/logout redirect hook; this removes
publish/unpublish/permalink).

Since the issue was filed the raw `window.location` calls were extracted into a
`client::navigation` module (`replace`/`reload`). The four callers are all in
`web/src/posts/component.rs`:

| Site (line) | Flow                                | Current call                                        |
| ----------- | ----------------------------------- | --------------------------------------------------- |
| 243         | Publish (PostCard)                  | `client::navigation::replace(&published.permalink)` |
| 1579        | Publish (EditPostPage)              | `client::navigation::replace(&updated.permalink)`   |
| 1227        | Unpublish (PostPage `on_unpublish`) | `client::navigation::replace("/drafts")`            |
| 1213        | Permalink-misroute escape           | `client::navigation::reload()`                      |

These are the **only** callers, so removing them makes `client::navigation` dead
code, and it is deleted with this change.

### Framework facts that shape the design (verified against leptos_router 0.8.14)

- **All same-origin `<a>` clicks are intercepted client-side.**
  `handle_anchor_click` `prevent_default`s and hands _every_ same-origin click
  to the router's `navigate` **before** any route is matched. A no-match renders
  `<Routes fallback>` **client-side**; leptos does **not** hand off to the
  server for a same-origin, plain, in-app anchor click. (It _does_ let the
  browser hard-navigate for `target=`, `download`, `rel=external`, modifier-key
  clicks, and cross-origin hrefs — none of which apply to these flows.) So
  "constrain the route and it falls through to the server" does not happen
  automatically.
- **The server serves the SPA shell (200) for unmatched GETs**, and its
  projector permalink route already requires a _literal_ `~`
  (`/~{username}/{y}/{m}/{d}/{slug}`). A blanket "fallback reloads to the
  server" would therefore **infinite-loop** on a genuinely-unknown URL (shell →
  boot → no match → reload → …).
- **Confirmed navigation surface (interview Q1):** there is **no** in-app
  `<a href>` — first-party chrome or user-authored post content — that targets a
  server-owned, non-`~`, 5-segment URL. First-party permalink anchors are all
  `~`-prefixed (they match `PostPage` correctly); feed/RSD links are `<link>` in
  `<head>` (never clicked); media file URLs are 6-segment `<img src>`, not
  5-segment anchors. The `reload()` escape at `component.rs:1213` is therefore
  **unreachable in practice today**; constraining the route makes it _provably_
  unreachable.

## Resolved decisions

1. **Publish/unpublish → router navigation.** Replace the
   `client::navigation::replace` calls with
   `leptos_router::hooks::use_navigate()` to the permalink / `/drafts`.
2. **In-place-publish staleness (interview Q2 → option a).** Publishing a draft
   from its **own** permalink page when the permalink does **not** move
   (same-UTC-day publish → identical date segments + slug → identical URL) makes
   `use_navigate(same_url)` a router no-op, so `PostPage`'s param-keyed
   `Resource` never re-keys and the page stays showing the draft banner +
   "Publish" affordance. The full reload masks this today; router-nav exposes
   it. Fix: on publish success, navigate to the (possibly identical) permalink
   **and** explicitly refetch `PostPage`'s resource; redundant-but- harmless
   when the URL actually changed. This is real new wiring, not a drop-in reuse:
   today `PostPage`'s `Resource` is keyed on route params **alone**
   (component.rs:1205, no `mutate_version`), the publish `Effect`
   (component.rs:237) navigates directly and **never** calls a mutate callback
   (unlike the delete/unpublish effects at 220/227), and `PostPage` passes only
   `on_unpublish` — not `on_mutate` — into its `PostCard` (component.rs:1264).
   The implementer must (i) give `PostPage`'s `Resource` a refetch trigger (add
   a version signal to its key, matching the timeline-page `on_mutate`
   convention), and (ii) run that trigger from the publish path (pass an
   `on_mutate` into the permalink `PostCard`, and have the publish `Effect` fire
   it alongside the navigate). `EditPostPage→PostPage` and `PostPage→/drafts`
   are always fresh mounts that refetch, so they need no extra invalidation.
3. **`~`-only permalink route (interview Q1).** Introduce a custom route segment
   implementing `leptos_router::PossibleRouteMatch` that matches a first segment
   **only** when it begins with `~`, and use it for the permalink route's
   leading segment (replacing `ParamSegment("username")` there). A non-`~`
   5-segment URL then matches no route and renders `<Routes fallback>` ("Page
   not found.") — acceptable, since Q1 established nothing in-app navigates
   there. **No** server-handoff / reload machinery is added (it would risk the
   infinite loop above). Then **delete** the `reload()` escape; a
   `~`-prefixed-but-malformed permalink (unparseable username or slug) 404s
   client-side without a reload, exactly like the existing invalid-slug branch.
4. **Enforcement gate — xtask syn-AST source-scan (interview Q3).** Add a new
   `xtask/src/steps/*_check.rs` (the established family:
   `proffered_secret_check`, `server_fn_registrar_check`,
   `sqlx_newtype_bind_check`, …) that scans `web/src` **and** `client/src` and
   rejects `window().location().{replace,assign,reload,set_href}` call chains
   (`set_href` is the Rust/`web_sys` equivalent of a `location.href = …`
   assignment). Runs in `cargo xtask check`. Rationale: clippy
   `disallowed-methods` would **not** reliably bite — these are wasm-target
   paths that the default clippy pass skips (this repo's known "default check
   skips wasm-gated web code" gotcha). **No allowlist:** after deleting
   `client::navigation` there are zero legitimate callers; leptos's
   `use_location()` (a free fn) and the pre-paint JS _string_ in
   `render/mod.rs:42` are not `web_sys` call-chains and don't match. The
   pre-paint `/`→`/app` redirect stays out of scope.
5. **E2E + docs (interview Q4).** Rewrite any e2e spec that waits for a document
   reload after publish/unpublish to `waitForURL` + a content assertion —
   **not** `body[data-hydrated]`, which is per-document and trivially already
   set after an SPA navigation. Remove the now-obsolete
   `location.replace`/`waitForURL`-caveat guidance from its committed home.
   **Note:** the `end2end/CLAUDE.md` the issue cites is untracked and absent
   from this worktree; the live caveat guidance to prune is in
   `end2end/tests/helpers.ts` / `hydration.ts` (exact sites pinned in the plan).

## Out of scope

- The server-fn login/logout redirect hook (#591, already landed).
- The pre-paint `/`→`/app` redirect (`web/src/render/mod.rs:42`) — a JS string
  constant that runs before wasm; a legitimate document-load site the gate never
  sees.
- Tightening the **other** username-first SPA routes (`/{username}`,
  `/{username}/tags/{tag}`) to `~`-only. They have no reload escape and swallow
  no known server URL; out of this issue's scope (potential separable
  follow-up). This is a deliberately _partial_ application of the `~`-namespace
  rule — it leans on the same Q1 link-inventory fact (nothing in-app links to a
  bare non-`~` server URL at those segment counts), so the rule is enforced
  where the bug lives (the 5-segment permalink route) and left as a follow-up
  elsewhere.

## Acceptance criteria (observable)

**No-reload observer.** Criteria 1–4 assert "no document reload" using the
existing `#591` no-reload sentinel (a value set on `window`/in-page state at
first boot that a full document load would clear); the e2e checks it survives
the flow. Every "no reload" criterion below is asserted through that sentinel,
not by timing.

1. **No full reload on publish (moved permalink).** Publishing a draft whose
   permalink changes navigates to the new permalink via client-side router
   navigation (URL updates, `#591` no-reload sentinel intact) and the page shows
   published state.
2. **No full reload on publish (in-place permalink).** Publishing a draft from
   its own permalink page when the URL is unchanged still flips the page to
   published state (draft banner and "Publish" affordance gone) with the
   sentinel intact — i.e. the resource is refetched, not the document reloaded.
   **Test setup:** force the identical-URL case by creating the draft and
   publishing it within the same test so creation and publication fall on the
   same UTC day (identical date segments, slug unchanged → identical permalink);
   assert the published state renders without the sentinel clearing.
3. **No full reload on unpublish.** Unpublishing from a permalink page
   client-side navigates to `/drafts`, which shows the just-unpublished post
   (fresh fetch), with the `#591` sentinel intact.
4. **Publish from the editor** navigates to the post's permalink client-side
   (sentinel intact) and shows published content.
5. **`~`-only permalink route.** The permalink SPA route matches a first segment
   only when it starts with `~`. Unit test: the custom segment's
   `.test("/~alice")` is `Some`, `.test("/media")` (and any non-`~` first
   segment) is `None`.
6. **Misroute escape deleted.** `client::navigation::reload()` no longer exists
   in `web/src`; a `~`-prefixed-but-malformed permalink renders a client-side
   not-found (no reload); a non-`~` 5-segment in-app navigation renders
   `<Routes fallback>`, not a mounted `PostPage`. **Dependency (interview Q1):**
   accepting `<Routes fallback>` ("Page not found.") for such a nav — rather
   than a server handoff — is correct only because the interview established
   that **nothing in-app links to a bare non-`~` 5-segment server URL** (a
   runtime link-inventory fact, not a code invariant). If a future feature adds
   such a link, that click regresses to a client 404 and this decision must be
   revisited (a scoped server-handoff, or extending the gate/route rule). Called
   out here so the assumption is explicit, not silently settled.
7. **`client::navigation` deleted.** The module and its `mod` declaration are
   gone; the crate builds with zero references to it.
8. **Gate bites.** `cargo xtask check` runs the new source-scan and passes on
   the cleaned tree; a deliberately-added `window().location().replace(...)` in
   `web/src` (or `client/src`) makes it fail. (Demonstrated by a temporary
   violation during development.)
9. **E2E green.** All four `{sqlite,postgres}×{chromium,firefox}` combos pass
   with the rewritten publish/unpublish specs (router-nav assertions, no reload
   waits).
10. **Docs pruned.** The obsolete `location.replace`/`waitForURL` reload caveat
    is removed from its committed location.
11. **Decision recorded.** The no-full-load invariant + `~`-namespace
    route-ownership rule are captured in an ADR (or ADR-0044 addendum) that is
    present and promoted at ship (see Decision record).

## Decision record

The no-full-load invariant (SPA is the sole in-app navigator; raw
`window.location` navigation forbidden in `web/src`/`client/src` and
gate-enforced) plus the `~`-prefixed user-namespace route-ownership rule are
novel, enforced architectural decisions → record as an ADR (or an addendum to
the CSR/routing ADR lineage, ADR-0044) during the plan/iterate phase.
