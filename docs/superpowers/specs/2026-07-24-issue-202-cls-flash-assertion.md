# Spec — #202: empirical layout-shift (CLS) e2e assertion for the authed-owner flash

- Issue: [#202](https://github.com/jaunder-org/jaunder/issues/202)
- Milestone: Test infrastructure & E2E
- Date: 2026-07-24
- Related: #181 / ADR-0044 (structural flash-free guarantee), #182 (parallel e2e
  `workers>1` — the stability constraint this must not undermine), #357 (sibling
  "can't observe the transient" issue)

## Problem

#181 guarantees the authed-owner flash-free behavior **structurally**: a Rust
coincidence unit test (projector output ≡ the shared render fn, including
reserved decoration slots) plus e2e that assert the pre-paint `authed` class is
set synchronously and owner affordances become present. It deliberately does
**not** do pixel/CLS diffing (ADR-0044 / #181 decision D8), because the
additive-plus- reserved-space design makes reflow structurally impossible and
the coincidence test guards the structure — and because a naive CLS check "races
the wasm load and can flake," which is exactly what #182's parallel-e2e campaign
is sensitive to.

#202 adds the **empirical belt-and-braces** check — but only under the issue's
hard precondition: **add it only if it can be made deterministic** (gated on a
stable readiness signal, not a timer). If it can't, it costs more in flakiness
than it buys.

## Feasibility (why this is buildable deterministically)

The precondition is satisfiable — both sample points gate on stable signals, no
timers:

- **First-paint DOM is present without wasm.** A server-side projector
  (`server::projector`) server-paints the anonymous post/sidebar content into
  the cacheable shell; "the projector's server-painted content and the client's
  first paint coincide" (`web/src/render/mod.rs:3-5`, `189-194`). So the served
  `/` HTML already contains the content to measure, before any wasm.
- **Post-mount signal is stable and synchronous.** `body[data-hydrated]` is set
  at the end of CSR mount (`csr/src/lib.rs:16`) and already gates every existing
  spec via `waitForHydration` (`end2end/tests/hydration.ts:16`). Race-free
  post-mount sample.
- **Pre-mount sample need not race the wasm load.** The harness already
  intercepts requests with `page.route` for fault injection
  (`end2end/tests/helpers.ts:105-112`). The same primitive **holds
  `/pkg/jaunder.wasm`** so `init()` never completes and the projector first
  paint stays frozen while we sample; then we release it and wait on
  `data-hydrated` to sample again. Both ends are gates, not delays.
- **Cross-browser measurement.** Use `getBoundingClientRect` diffing, **not**
  the `layout-shift`/CLS `PerformanceObserver` — the CLS API is Chromium-only
  and the e2e matrix includes Firefox. Bounding-box reads work in every browser.

The residual risk is not sampling determinism (solved) but the **comparison**:
the post-mount sample reads a _fresh_ DOM node (mount drops `#app` and remounts
the byte-coincident shell), so cross-browser sub-pixel rounding could differ.
The byte-coincident-shell design (`render/mod.rs:189-194`) predicts ~zero shift,
so a strict threshold is defensible; the threshold decision below handles the
residual.

## Decision

Add one new Playwright spec, `end2end/tests/authed-cls.spec.ts`, that
empirically asserts content does not shift across the projector-paint →
wasm-mount transition for an authed owner on the cacheable public timeline
(`/`). No product code changes.

**The precise guarantee under test.** Per `render_post_content`
(`web/src/posts/render.rs:172-179`) and the CSS reservation comment
(`server/assets/jaunder.css:1282-1286`): the own-post **action column**
(`.j-post-acts`) is the _one_ affordance deliberately **not** pre-reserved
(ownership is unknown at the anonymous projector paint, and reserving a gutter
on every post would wrongly gutter non-owner posts). It is added client-side at
mount as a flex **sibling to the right** of the post content column (`.j-post`
is `grid: auto 1fr` = avatar | content; the content is a `<div flex:1>` and the
acts column is `flex-shrink:0` appended beside it). The design's claim is that
this addition is "purely additive — never a content change": the content
column's top-left must not move. #202 is the empirical confirmation of exactly
that bounded case.

**Target: the owner's own post content vs. its additive action column** — _not_
the sidebar (the sidebar footer is already structurally reserved via
`html.authed .j-sb-foot { min-height:44px }`, and its anon→authed nav growth is
a different, non-#202 concern that would false-fail this check).

**Mechanism (deterministic by construction):**

1. Register an authed owner via `register(page, firstNav)` (`helpers.ts:163`) —
   **not** the `registeredPage` fixture — because the test needs the
   **username** to scope its measurements to the owner's own post (see step 3a).
   Seed one owner post with a **short, single-line body**
   (`createPostViaApi(page, { body })`, `posts.ts`; `publish` defaults true). A
   short body does not wrap, so the content column narrowing by the acts-column
   width cannot reflow it — the post's content blocks are width-stable.
2. Install a `page.route('**/pkg/jaunder*.wasm', …)` handler that **holds** the
   request (awaits a test-controlled release) before continuing it. Register the
   route **before** `goto` (the intercept must be armed before `init()` fetches
   the wasm). A one-line comment must note this route also _disables
   Playwright's HTTP cache_ for the wasm — which is what forces a fresh,
   holdable request even though `register()`'s earlier navigation warmed the
   wasm — so a maintainer does not "optimise" it away.
3. `goto('/', { waitUntil: 'domcontentloaded' })` — projector first paint is up,
   wasm held.
   - **(3a) Author-scope the measured post.** `/` shows many `.j-post` articles;
     under `workers>1` the first one is not necessarily the owner's, and
     measuring the wrong article would pass _vacuously_. Scope to the owner's
     post by author handle — a Playwright locator, e.g.
     `page.locator('.j-post', { has: page.locator('.j-post-handle', { hasText: `@${username}` }) })`
     — which is stable across **both** phases (the handle is in the anonymous
     projector paint, present before mount). Measure via that locator's
     `.locator('.j-post-head' | '.j-post-body').boundingBox()` — **not**
     `document.querySelector` (which can't express a text scope).
   - **(3b)** Gate the before-sample on
     `await page.evaluate(async () => { await document.fonts.ready; })` (return
     nothing — the `FontFaceSet` is not serializable) so a late font/metrics
     settle cannot masquerade as a shift. Then read the **before** bounding
     boxes of the scoped `.j-post-head` and `.j-post-body`.
4. Release the held wasm request; `await waitForHydration(page)`
   (`body[data-hydrated]`). Assert the scoped owner post's `.j-post-acts`
   `toBeVisible()` (proves the authed path ran and the affordance under test
   actually appeared on the _measured_ post — not an anonymous no-op). Read the
   **after** bounding boxes of the same scoped `.j-post-head` / `.j-post-body`.
5. Assert no positional shift per element (threshold below). `unroute` after.

**Target elements** — the owner's own post content (author-scoped per 3a),
present at _both_ phases, which the additive action column must not push.
Measured by top-left (`boundingBox()` `x`/`y`):

- The post **content column anchor** — `.j-post-head` (the topmost content
  block; its `x` = content-column left, `y` = post top). Core guarantee: the
  action column is added to the _right_, so the content's left/top must not
  move.
- The post **body** — `.j-post-body` (`web/src/posts/render.rs:212`; the
  _rendered_ body div, **not** `SEL.postBody`, which is the composer
  `textarea[name="body"]`) — top-left, the stricter check that no upstream
  reflow shoved it.

The newly-appearing `.j-post-acts` is asserted _present_ (step 4) but is **not**
measured for movement — it is the additive element; the check is that the
_pre-existing_ content stays put. `.j-post`, `.j-post-head`, `.j-post-body`,
`.j-post-acts` are raw CSS locators (as in `authed-flash.spec.ts`).

**Threshold — start exact, loosen only on evidence (per design decision):**

- v1 asserts **exact equality** (`after.x === before.x`, `after.y === before.y`)
  per measured element and axis — matching the byte-coincident guarantee
  literally.
- If the full `{sqlite,postgres}×{chromium,firefox}` matrix shows sub-pixel
  noise on any browser, loosen that axis to `Math.abs(after.n - before.n) < 1`
  with a code comment citing the observed value and browser. A real reflow is
  tens of px, so a `<1px` tolerance still catches every regression this check
  exists to catch.

**Determinism / #182 safety:** the spec waits only on `page.route` release and
`body[data-hydrated]` — never a `setTimeout`/fixed delay — so it is safe to run
under `fullyParallel` `workers>1`. It inherits the suite's existing 10s
assertion timeout and `JAUNDER_E2E_RETRIES` flake containment; it introduces no
new timing dependency.

## Scope

**In scope:** two new e2e files — (1) a **reusable** mount-transition
layout-shift helper, `end2end/tests/layout-shift.ts`, that encapsulates the
invariant machinery (wasm-hold route, `fonts.ready` gate, before/after
`boundingBox` sampling, `waitForHydration`, an optional post-mount assertion
hook, and a per-call `tolerancePx` threshold); and (2)
`end2end/tests/authed-cls.spec.ts`, a thin spec that calls the helper for the
`/` (Local timeline) authed-owner post case. The helper is factored so that
adding the same CLS check to another page is a small spec (its URL + target
locators + auth/seed + a mount assertion), not a re-implemented scaffold — an
explicit design goal, kept minimal (shaped by this use + the page/targets/auth
variation axis, no speculative hooks).

**Out of scope / must not change:**

- Product code (`web`, `csr`, `server` projector) — this is an assertion over
  existing behavior, not a change to it.
- The structural #181 guards (`authed-flash.spec.ts`, the coincidence unit test)
  — unchanged; this is additive.
- The `layout-shift`/CLS PerformanceObserver route (rejected — Chromium-only).
- No `setTimeout`/fixed-delay gating anywhere (the whole point).

## Acceptance criteria

1. **A new deterministic CLS spec exists** — `end2end/tests/authed-cls.spec.ts`
   samples the owner-post content `getBoundingClientRect` with wasm held (first
   paint, after `document.fonts.ready`) and again after `body[data-hydrated]`,
   gating only on the route release and the hydration marker (no timers).
2. **It asserts no shift** of the owner's post content — the author-scoped
   `.j-post-head` and `.j-post-body` top-left — across the projector-paint →
   wasm-mount transition, at exact equality (v1), or `<1px` on a specific
   axis/browser with a documented observed-value comment if the matrix
   empirically requires it.
3. **The affordance under test is exercised** — the spec asserts `.j-post-acts`
   on the **author-scoped owner post** `toBeVisible()` post-mount, so the check
   provably covers the additive own-post action-column case on the _measured_
   post (not an anonymous no-op, and not a different article than the one
   measured).
4. **No product code changes** — the diff is confined to `end2end/` test files.
5. **Green across the full matrix, verified non-flaky** — `cargo xtask validate`
   passes with the new spec on all four `{sqlite,postgres}×{chromium,firefox}`
   combos. Because the gate runs with `JAUNDER_E2E_RETRIES=1` (a fail-then-pass
   is reported `flaky` with exit 0), "green" is confirmed by inspecting the
   Playwright `flaky` count = 0 for this spec, not the exit code alone. Any
   `<1px` loosening is applied and documented before landing.

## Verification

`cargo xtask validate` (all four VM e2e combos) is the empirical determinant: it
both proves the assertion holds and reveals whether exact-equality survives
cross-browser sub-pixel rounding. Per the threshold decision, the first
full-matrix run decides whether v1 exact-equality ships or is loosened to a
documented `<1px`; confirm via the Playwright `flaky` count (§AC5), not just
exit status. Because the check is deterministic by construction, a failure is a
_real_ signal — one of: (a) sub-pixel cross-browser noise → the documented
`<1px` loosening; or (b) a **material** content shift, which would mean the
own-post action column is not in fact purely additive (a genuine finding about
the flash-free guarantee's bounded case) — that outcome is reported back, not
silently tolerance-hidden. It is **not** timer flake. CI's fresh-runner
`e2e-gate` provides the cold-cache confirmation.
