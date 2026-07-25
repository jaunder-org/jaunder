# Spec — Issue #653: converge SiteTagPage/UserTagPage onto the shared TimelineState bundle

**Issue:** jaunder-org/jaunder#653 · **Milestone:** Web: canonical Leptos CSR
convergence · **Branch:** `worktree-issue-653-tag-pages-timeline-converge`

## Problem

`SiteTagPage` and `UserTagPage` (`web/src/posts/component.rs`) still hand-roll
the ad-hoc pagination state machine that #643 dissolved for `UserTimelinePage` —
six `RwSignal`s (`timeline`, `next_cursor_created_at`, `next_cursor_post_id`,
`has_more`, `error`, `initial_loaded`), a `ServerAction` load-more, twin
`Effect`s, and inline row rendering. After #643 this logic lives in three places
(the shared `crate::timeline` bundle used by home / cockpit / the user timeline,
plus these two). This is the **structural** refactor that finishes the timeline
family's convergence.

**Projector-coincidence finding (drives the approach).** These are
projector-seeded public pages: the server paints them (`PageSeed::SiteTag` /
`PageSeed::UserTag`, `server/src/projector/mod.rs:291,325`) via `web::render`'s
`render_timeline_page` before the CSR client mounts, and the reactive view must
**visually coincide** with that first paint or the boot flashes. Today
`render_timeline_page` emits
`{topbar}<div class="j-scroll"><div class="j-page">…</div></div>`
(`web/src/posts/render.rs:246`), while the shared `TimelineRows` emits a
**bare** `j-scroll` (no `j-page`) — so a naive `TimelineRows` swap would diverge
(a `j-page`-gutter flash) and change the empty-state copy.

This is the same coincidence constraint #643 hit but did not fully resolve: the
user timeline (also `render_timeline_page`-painted) was converged to bare
`TimelineRows` while its projector kept `j-page` — a latent first-paint gutter
shift on `/~username`.

## Decisions (resolved in interview)

- **D1 — Flush reconcile-up (chosen after a live screenshot A/B).** Rather than
  hold the components back to the projector's `j-page` layout, bring the
  **projector down to the flush structure the components already use**. Make
  `render_timeline_page` (`web/src/posts/render.rs:230-247`) emit exactly what
  `TimelineRows` emits and what home's `SiteTimeline` projector branch already
  emits: posts (or the empty `<p>`) **directly inside
  `<div class="j-scroll">`**, with **no `j-page` wrapper and no inner
  posts-`<div>`**. Concretely the target is
  `{topbar}<div class="j-scroll">{empty_p | articles + load_more}</div>` —
  matching `render_body`'s `SiteTimeline` branch (`render.rs:43-56`, pinned by
  the host test at `render.rs:466`: `<div class="j-scroll"><article…`).
  **Removing only `j-page` is insufficient** — `render_timeline_page` also wraps
  the articles in an extra `<div>{articles}</div>` (`render.rs:240-244`) that
  `TimelineRows` does **not** emit, so that inner `<div>` must go too or the
  first paint (`j-scroll > div > article`) still diverges from the CSR mount
  (`j-scroll > article`). This unifies the whole projector-seeded timeline
  family on one flush structure — posts align with the page heading, as on home.
  **This also fixes #643's user-timeline gutter flash for free**: once the
  projector emits the bare wrapper-free structure, it matches the `TimelineRows`
  CSR #643 already ships, with no change to `UserTimelinePage` and no revert.
- **D2 — `TimelineRows` gains an optional `empty_text` prop** (default
  `"No posts yet."`). The tag pages pass `"No posts with this tag yet."` so the
  tag-specific empty message is preserved (no copy regression); home / cockpit /
  the user timeline keep the default. The projector's `render_timeline_page`
  already takes `empty_text` as a parameter, so both sides stay in sync.
- **D3 — Both tag pages → full `TimelineRows` convergence.** Replace the five
  list/cursor/error signals + `ServerAction` load-more + inline rows with
  `TimelineState` + `spawn_load_more` + `TimelineRows`, mirroring
  `UserTimelinePage`. Chrome (`FeedDiscovery`, `Topbar`) is preserved.
- **D4 — `tag_context` preserved per page.** `SiteTagPage` → `TimelineRows`
  default (`SiteWide`), no prop. `UserTagPage` → pass
  `tag_context=TagContext::ForUser(username)` (its rows use per-row
  `post.username`, which equals the page `username` since every row is that
  user's post).
- **D5 — a single `loaded` gate survives on each page**, same rationale as #643:
  the shared `TimelineState`/`LoadStatus` has no "never-loaded" state, and these
  pages get `tag`/`username` from the route immediately and are not always
  seeded, so without the gate the unseeded client-nav first load would flash the
  empty state before the fetch resolves. The gate renders "Loading…" until the
  first seed/resolve.
- **D6 — projector-seed adoption preserved via `state.adopt`.** The
  `PageSeed::SiteTag { tag, page }` (guarded on `tag`) and
  `PageSeed::UserTag { username, tag, page }` (guarded on `username` **and**
  `tag`) adoptions are kept, now `state.adopt(page)` + `loaded.set(true)`.
- **D7 — load-more moves `ServerAction` → `spawn_local`** via
  `spawn_load_more(state, fetch)`, `fetch` an adapter over the tag-scoped list
  fn: `list_posts_by_tag(tag, …)` (Site),
  `list_user_posts_by_tag(username, tag, …)` (User). The `mutate_version`
  re-fetch key on the initial `Resource` is preserved.
- **D8 — error via a `Memo` over `state.status`**
  (`Memo::new(move |_| state.status.get().into_failure())`), mirroring home /
  #643.
- **D9 — orphaned imports removed.** After conversion, `ListPostsByTag` and
  `ListUserPostsByTag` are unused in `component.rs`; drop them from the
  `use crate::posts::{…}` block. Removing the `next_cursor_created_at` signals
  also orphans `use common::time::UtcInstant;` (its only uses) — drop it too
  (compiler-forced cleanup, in scope). The async fns `list_posts_by_tag` /
  `list_user_posts_by_tag` and `utc_instant_from_local` remain.
- **D10 — projector coincidence test updated.** The host test
  `body_covers_tag_page_headings` (`web/src/posts/render.rs`, the assertion at
  ~`:435` pinning `<div class="j-scroll"><div class="j-page">` for tag pages) is
  the one that breaks and must be updated to the new wrapper-free structure
  (`<div class="j-scroll">…` with the articles directly inside, mirroring the
  home assertion at `render.rs:466`). The updated assertion **must** drop both
  the `j-page` and the inner posts-`<div>`, or it silently re-pins a divergent
  structure. `timeline_page_empty_states_differ_by_route` (`render.rs:482-500`)
  and the permalink/home tests don't reference tag `j-page` and stay green;
  `web/src/render/mod.rs` has no tag/profile `j-page` coincidence test.

Out of scope: the tags vertical proper (#328); any change to `TimelineState` /
`spawn_load_more` beyond the additive `empty_text` prop; the home-timeline
projector (`SiteTimeline` branch — already bare `j-scroll`, untouched).

## Acceptance criteria

Each is observable so the ship-time conformance review can tell delivered from
not.

1. **Both pages converged.** `SiteTagPage` and `UserTagPage` use
   `TimelineState`, `spawn_load_more`, and `TimelineRows`; neither declares the
   five ad-hoc list/cursor/error signals nor a `ServerAction`-based load-more.
2. **Projector is flush and wrapper-free.** `render_timeline_page` emits
   `<div class="j-scroll">{empty_p | articles + load_more}</div>` — no `j-page`
   wrapper and no inner posts-`<div>` — structurally identical to `TimelineRows`
   and to home's `SiteTimeline` branch. (Do **not** grep the whole file for
   `j-page`: the `Permalink` branch, `render.rs:39`, legitimately keeps its own
   `j-page` and is not converged.) The home `SiteTimeline` and `Permalink`
   branches are unchanged.
3. **Empty copy preserved.** `SiteTagPage`/`UserTagPage` still render
   `"No posts with this tag yet."` (via the `empty_text` prop and the
   projector's unchanged `empty_text` argument); home / user timeline still
   `"No posts yet."`.
4. **`TimelineRows` additive.** It accepts `#[prop(default = …)] empty_text` (or
   equivalent) defaulting to `"No posts yet."`; home/cockpit/user-timeline call
   sites are otherwise unchanged.
5. **Tag context preserved.** `SiteTagPage` rows link `SiteWide`; `UserTagPage`
   rows link `ForUser(username)`.
6. **Loading gate present** on each page (the "Loading…" placeholder before the
   first seed/resolve).
7. **Projector seeds preserved** — `PageSeed::SiteTag`/`UserTag` adoption (with
   their existing guards) via `state.adopt`.
8. **Orphaned imports gone.**
   `rg -n "ListPostsByTag|ListUserPostsByTag|UtcInstant" web/src/posts/component.rs`
   returns nothing (the `next_cursor` signal removal orphans `UtcInstant` too;
   `utc_instant_from_local` is a different symbol and stays).
9. **#643 side effect verified.** The user timeline (`/~username`) first paint
   no longer shifts: projector and CSR are both bare `j-scroll` (no
   `UserTimelinePage` code change; the projector drop restores coincidence).
10. **Coincidence + behavior verified.**
    - `cargo xtask validate --no-e2e` green (incl. the updated
      `render_timeline_page` host/coincidence tests); `web` wasm clippy clean.
    - `cargo xtask e2e-local posts` green. The tag-route assertions that keyed
      on `.j-page` (`posts.spec.ts` ~:788, ~:851) are updated to the new flush
      structure; the `"No posts with this tag yet."` assertion stays and passes.
    - A **new `UserTagPage` smoke assertion** is added (today no e2e navigates
      `/~username/tags/:tag`): it loads the page and asserts the tagged post
      renders. This closes the coverage gap the review found.
    - No first-paint flash on the projector-seeded tag pages or the user
      timeline.
11. **Coverage policy respected.** No new `cov:ignore` without justification;
    CRAP threshold respected. New host-testable logic (the
    `render_timeline_page` change) stays covered by its updated tests.

## Risks

- **Concurrent `posts/component.rs` work.** Other web-vertical worktrees touch
  the same file (e.g. #656). Keep the branch current and rebase; the change is
  localized to the two tag-page functions, one import line, `TimelineRows`'
  signature, and `render.rs`.
- **Coincidence tests are the real guard.** If `render_timeline_page`'s output
  tests aren't updated to the new structure they'll fail (good — they're doing
  their job); the risk is updating the _component_ without the _projector_ (or
  vice-versa) and shipping a divergence. Both move together in one change.
