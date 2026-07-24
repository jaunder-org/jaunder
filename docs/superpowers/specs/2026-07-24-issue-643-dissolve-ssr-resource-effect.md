# Spec — Issue #643: dissolve the SSR-era Resource→Effect→signal indirections in posts

**Issue:** jaunder-org/jaunder#643 · **Milestone:** Web: canonical Leptos CSR
convergence · **Branch:** `worktree-issue-643-dissolve-ssr-resource-effect`

## Problem

Four sites in `web/src/posts/component.rs` copy a resolved `Resource` into
`RwSignal`s via an `Effect`, each justified **exclusively** by SSR/hydration
semantics that the CSR migration removed (per-request reactive-owner disposal,
value serialization on hydration). The routed Leptos pages now serve a static
CSR shell and mount fresh on the client (#487; projector seeds adopted per
ADR-0041/0044) — there is no server-render-then-hydrate of these components, so
the disposal race the comments cite cannot occur. The dead rationale, and in one
case an entire ad-hoc state machine shaped by it, should give way to the
CSR-native shape.

The doctrine those comments cite —
`docs/web-style-guide.md §9 "SSR-safe Resource patterns"` — is itself the
vestigial source of the pattern and will regenerate it; it is revised here.

## Scope

In scope: the four sites in `web/src/posts/component.rs`; a small additive
widening of the shared `web/src/timeline` component; the §9 revision; a
stale-doc sweep of `crate::pages::ui` module-doc refs; two new e2e assertions
closing seed-coverage gaps.

Out of scope (split to their own issues, filed by the plan's first task):

- **LogoutPage** (`web/src/auth/component.rs`) "You have been logged out."
  message likely never shown (the #591 redirect hook navigates to `/`). A
  behavior fix in the auth vertical with its own verification.
- No projector-seed (`PageSeed`/`state.adopt`) mechanics change — that is
  current post-SSR design, not a remnant.

## Decisions (resolved in interview)

- **D1 — Site 1 (`AudiencePicker`): full dissolution.** The `named_audiences`
  `RwSignal` exists only to receive the copy (the mutable state, `selection`, is
  a prop). Consume the `named` `Resource` directly in the view closure that
  already renders the checkbox rows; delete the `RwSignal` and its `Effect`.
- **D2 — Sites 2 & 4 (audience seeds): seed-then-edit survives, ceremony dies.**
  The user edits `audience` via the picker after it seeds, so a seed step is
  genuinely needed. Site 4 (`EditPostPage`) already wraps `post` in
  `Suspense`/`Suspend`; **fold the `current_audience` seed into that same
  block**, awaiting it alongside `post`, dissolving the separate `Effect`. Site
  2 (`PostCreateForm`) renders instantly (no `Suspense` — the composer must
  appear without waiting on the async site default), so its `Effect` survives
  with a comment stating the real CSR reason (async default arrives after first
  paint; seed once resolved).
- **D3 — Site 3 (`UserTimelinePage`): full convergence, minus a loading gate.**
  Replace five of the six ad-hoc signals (`timeline`, `next_cursor_created_at`,
  `next_cursor_post_id`, `has_more`, `error`) + `ServerAction` load-more +
  inline rows with the shared `TimelineState` bundle
  - `spawn_load_more` + `TimelineRows`, matching `home.rs`/`cockpit.rs`. The
    `PageSeed::Profile` seed is adopted via `state.adopt(page)`
    (username-guarded as today). The initial fetch keeps a
    `Resource → Effect → state.resolve/fail` shape — canonical CSR, not a
    remnant — with a CSR-correct comment (no SSR rationale). Load-more moves
    from `ServerAction` to `spawn_local` (via `spawn_load_more`). **A single
    boolean loading-gate signal survives** (the former `initial_loaded`,
    restated as a CSR loading gate): the shared `TimelineState`/`LoadStatus` has
    no "never-loaded" state (`home.rs` is always projector-seeded; `cockpit.rs`
    gates on its `username` resolving), but `UserTimelinePage` gets `username`
    from the route param immediately and is not always seeded, so without this
    gate the unseeded client-side-nav first load would flash the shared
    `TimelineRows` empty state ("No posts yet.") before the fetch resolves. The
    gate renders "Loading…" until the first seed/resolve, then hands off to
    `TimelineRows` — matching `cockpit.rs`'s no-flash behavior. It is a CSR
    affordance, not an SSR remnant.
- **D4 — `TimelineRows` widened additively.** Add
  `#[prop(default = TagContext::SiteWide)] tag_context: TagContext`, passed
  through to `PostCard`, so `UserTimelinePage` keeps its `ForUser` tag links.
  `home.rs`/`cockpit.rs` call sites are unchanged (default preserves current
  `SiteWide` behavior).
- **D5 — §9 revised in-scope, narrowly.** §9 has three parts; only the first is
  the vestige. Rewrite is bounded:
  - **Rewrite anti-pattern #1's SSR rationale** (`web-style-guide.md:230-241`) —
    the disposal-race / serialization framing the four dead comments cite — to
    the current CSR reality: routed Leptos components serve a static CSR shell
    and mount fresh via `mount_to_body` (no hydration — verified at
    `csr/src/lib.rs:10-12,44`; the `leptos/ssr` feature serves only server-fns,
    the projector's render fns, and `leptos_axum` routing, not component
    hydration), so a plain client-only `Effect::new` copying a resolved
    `Resource` into signals is the normal idiom, not an SSR-safety workaround.
  - **Preserve the wasm-only placement rule** ("client-only `Effect::new`
    belongs in the vertical's wasm-only `component.rs`") — still CSR-correct and
    cited by live code (`EditPostPage`'s redirect effect, component.rs:1557).
  - **Preserve anti-pattern #2's substantive guidance** (ADR-0016 handle-first,
    graceful `Err` over panic, read-context-before-`await` — good server-fn
    hygiene) while trimming only its now-false SSR claims ("resolved during
    SSR", "serializes... not re-fetched on hydration").
  - **Preserve the sticky-copy subsection verbatim** (`:266-279`,
    `Invalidator::sticky`/`MemberChecklist`) — CSR-current and cross-referenced
    by **ADR-0061** (twice) and `component.rs:334`.
  - **Keep the section number `9`** (retitle away from "SSR-safe", but ADR-0061
    cites "§9" by number, so the anchor must stay). Grounded in
    #487/ADR-0041/0044.
- **D6 — stale-doc sweep.** Correct the pre-#323 `crate::pages::ui` module-doc
  refs at `web/src/posts/render.rs:5` **and** the sibling
  `web/src/render/mod.rs:50,281,315` to the current `crate::posts::component`
  location (docs-track-code).
- **D7 — close the two e2e seed gaps.** The seed behaviors of sites 2 & 4 have
  no e2e assertion today (all audience tests explicitly `selectOption`, never
  asserting a _pre-selected_ state). Add assertions so a silently-broken seed
  fails CI.

## Acceptance criteria

Each is observable so the ship-time conformance review can tell delivered from
not.

1. **No SSR-justified copy remains.** In `web/src/posts/component.rs`, no
   `Resource`→`Effect`→signal copy remains whose only justification was SSR
   serialization/disposal/hydration. Any surviving `Effect` (sites 2, 3) carries
   a comment stating a real CSR-era reason. `grep` for the phrases "disposal",
   "hydration", "SSR", "serialize", "new_isomorphic" in the four former spans
   returns nothing.
2. **Site 1 dissolved.** `AudiencePicker` no longer declares a `named_audiences`
   `RwSignal` or an `Effect` seeding it; the named-checkbox view reads the
   `Resource` directly.
3. **Site 4 dissolved into Suspense.** `EditPostPage` no longer declares a
   standalone `Effect` seeding `audience` from `current_audience`; the seed
   happens inside the existing `Suspense`/`Suspend` block.
4. **Site 3 converged.** `UserTimelinePage` uses `TimelineState`,
   `spawn_load_more`, and `TimelineRows`; the five ad-hoc list/cursor/error
   signals and the `ListUserPosts` `ServerAction` load-more are gone (a single
   boolean loading-gate signal survives per D3). `PageSeed::Profile` adoption
   and the `mutate_version` re-fetch are preserved.
5. **`TimelineRows` additive.** `TimelineRows` accepts an optional `tag_context`
   defaulting to `SiteWide`; `home.rs`/`cockpit.rs` call sites are textually
   unchanged.
6. **§9 revised, narrowly.** `docs/web-style-guide.md §9` no longer presents the
   SSR disposal/serialization race as live guidance for the Effect-copy pattern
   (anti-pattern #1); it documents the client-only `Effect` copy as the CSR
   norm. The section keeps its number `9`, its wasm-only placement rule,
   anti-pattern #2's substantive ADR-0016 guidance, and the sticky-copy
   subsection (`Invalidator::sticky`/`MemberChecklist`) — all preserved. No code
   comment in the repo still cites "§9" for an _SSR-safety_ reason; every
   surviving "§9" citation (`component.rs:334` sticky, `EditPostPage:1557`
   client-only-Effect, `ADR-0061` ×2, `docs/README.md`) still resolves to
   correct, present guidance.
7. **Stale doc refs fixed.** No `pages::ui` module-doc reference remains in
   `web/src/posts/render.rs` or `web/src/render/mod.rs`
   (`rg 'pages::ui' web/src` returns nothing).
8. **Behavior identical, verified in a browser.**
   - `cargo xtask validate --no-e2e` green; `web` wasm clippy clean
     (`cargo clippy -p web --target wasm32 -- -D warnings`).
   - `cargo xtask e2e-local` green on `posts`, `visibility`, and `audiences`
     specs (create-post default seeding, edit-post seeding + publish redirect,
     user-timeline load + pagination, named-audience picker rendering).
   - No new visual flash on the user-timeline projector-seeded first paint;
     **and** the unseeded (client-side-nav) first load shows the "Loading…"
     placeholder, not the "No posts yet." empty state, before the fetch resolves
     (D3 loading gate).
9. **New e2e assertions present and green.**
   - Create: with a non-Public site default configured, a fresh composer shows
     that default selected in `#audience-base`.
   - Edit: opening a post targeted `Subscribers + <named>` pre-selects
     `subscribers` in `#audience-base` and pre-checks the named checkbox.
10. **Coverage policy respected.** No new `cov:ignore` without justification;
    CRAP threshold respected.

## Verification notes

Per the issue, the gate alone is insufficient — browser-level verification of
the touched flows is required, and the two new e2e assertions (AC9) are the
regression guard for the seed behaviors the gate cannot see.
